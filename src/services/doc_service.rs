//! 统一单据服务（doc_service）
//!
//! 单一入口：save / approve / unapprove / void / generate_from_source
//! 在事务内完成：
//!   1. 校验（主表必填、明细非空、库存可用量）
//!   2. 主表 + 明细表持久化
//!   3. 库存三件套（post_ledger）同步
//!   4. QQty 预占/释放（销售订单审核/出库审核/反审）
//!   5. tSys_OperHis / tSys_OperLog 写入
//!
//! 前端调用：`/api/doc/*`

use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tiberius::ToSql;

use crate::metadata::doc_graph::{self, DocMeta};
use crate::services::inventory_ledger::{
    self, apply_qqty_delta, check_period_closed, fill_detail_stock_snapshot,
    fill_io_detail_stock_snapshot, fill_move_detail_stock_snapshot,
    fill_tran_detail_stock_snapshot, post_ledger_with_period, query_doc_state, query_qqty,
    query_stock_qty, record_oper_with_data, reverse_stock_delta_only, update_doc_state_with_cas,
};

pub type Conn = PooledConnection<'static, ConnectionManager>;

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";
const STATE_REVIEWED: &str = "S";
const STATE_NEW: &str = "N";
const STATE_DELETED: &str = "D"; // D=删除/作废，不可审核
const STATE_EDIT: &str = "E";
const STATE_VOID: &str = "C"; // C=已作废，终态
const STATE_CONFIRMED: &str = "Y"; // Y=已确认，下游已使用

/// 根据表名获取默认 doc_type（与前端 docGraph.js 的 DOC_TYPE_MAP 严格一致）
/// 前端 DataPage 通过 getDocTypeByTable(table) 推导 doc_type，
/// 后端此函数确保即使前端不传 doc_type，后端也能正确路由到 post_stock_on_approve 分支。
/// 基于 doc_no_prefix 映射，避免硬编码表名。
fn default_doc_type_for_table(meta: &DocMeta) -> String {
    match meta.doc_no_prefix.as_str() {
        // purchase
        "PO" => "purchase_order",
        "PI" => "purchase_inbound",
        "PR" => "purchase_return",
        "PRQ" => "purchase_quote",
        "PAP" => "purchase_price_adjust",
        // sales
        "SO" => "sales_order",
        "SI" => "sales_outbound",
        "SR" => "sales_return",
        "SRQ" => "sales_quote",
        // stock
        "IO" => "stock_io",
        "MV" => "stock_move",
        "TR" => "stock_check",
        "CYC" => "stock_cycle",
        "RPA" => "replenish_apply",
        "ADJ" => "stock_adjust",
        // finance
        "RCV" => "receipt",
        "PAY" => "payment",
        "CF" => "cash_flow",
        // 兜底：用 biz_type
        _ => &meta.biz_type,
    }
    .to_string()
}

// ============== 请求 / 响应结构 ==============

#[derive(Debug, Deserialize)]
pub struct SaveDocParams {
    /// 业务主表名（如 tPur_Order）
    pub table: String,
    /// 主表 PK 字段（如 POID）
    pub primary_key: String,
    /// 主表数据（包含主键或不含，POST 决定 create/update）
    pub data: serde_json::Value,
    /// 明细行列表
    #[serde(default)]
    pub details: Vec<serde_json::Value>,
    /// 保存并审核（true=保存后自动审核，只写一条 APPROVE 日志）
    #[serde(default)]
    pub auto_approve: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SaveDocResponse {
    pub id: String,
    pub doc_no: String,
    pub operation: String,                             // CREATE / UPDATE
    pub partial_success: Option<bool>,                 // 保存成功但审核失败时为 Some(true)
    pub approve_error: Option<String>,                 // 审核失败原因
    pub shortage_list: Option<Vec<StockShortageItem>>, // 库存不足明细（前端表格展示 + 一键删除）
}

#[derive(Debug, Deserialize)]
pub struct ApproveDocParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub doc_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VoidDocParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateFromSourceParams {
    pub source_table: String,
    pub source_id: String,
    pub target_table: String,
}

#[derive(Debug, Serialize)]
pub struct GenerateFromSourceResponse {
    pub target_table: String,
    pub master: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

/// 库存不足明细行（前端表格展示用 + 后端持久化到 tStk_Shortage）
#[derive(Debug, Clone, Serialize)]
pub struct StockShortageItem {
    /// 明细行号（从 1 开始）
    pub row_no: usize,
    /// 商品 ID（GUID，用于持久化关联查询，前端不展示）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub gds_id: String,
    /// 仓库 ID（GUID，用于持久化关联查询，前端不展示）
    #[serde(skip_serializing_if = "String::is_empty")]
    pub stk_id: String,
    /// 商品编码
    pub gds_no: String,
    /// 商品名称
    pub gds_name: String,
    /// 仓库编码
    pub stk_no: String,
    /// 仓库名称
    pub stk_name: String,
    /// 当前库存
    pub stock: f64,
    /// 预占量
    pub reserved: f64,
    /// 可用量 = 库存 - 预占
    pub available: f64,
    /// 需求量
    pub qty: f64,
    /// 不足数量 = 需求 - 可用
    pub shortage: f64,
}

/// 审核错误类型：支持普通消息和结构化库存不足明细
#[derive(Debug)]
pub enum ApproveError {
    /// 普通错误消息
    Msg(String),
    /// 库存不足明细列表
    Shortage(Vec<StockShortageItem>),
}

impl ApproveError {
    /// 从普通字符串创建
    pub fn msg<S: Into<String>>(s: S) -> Self {
        ApproveError::Msg(s.into())
    }
}

impl From<String> for ApproveError {
    fn from(s: String) -> Self {
        ApproveError::Msg(s)
    }
}

impl From<&str> for ApproveError {
    fn from(s: &str) -> Self {
        ApproveError::Msg(s.to_string())
    }
}

impl std::fmt::Display for ApproveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApproveError::Msg(s) => write!(f, "{}", s),
            ApproveError::Shortage(items) => {
                write!(f, "库存不足，共 {} 条明细短缺", items.len())
            }
        }
    }
}

// ============== 校验辅助 ==============

/// 门店销售单（商场数据录入）BTPID：tSal_Inv 表专用，门店自卖无需客户
/// 与前端 client/src/config/enums.js 的 BTPID.RETAIL_SALE_INV 保持一致
const RETAIL_SALE_INV_BTPID: &str = "A4BA71AE-E908-4C97-9148-E4A26AD66373";

/// 判断是否为门店销售单（从 JSON data 读 BTPID）
/// 门店销售单是门店（商场）自卖，不进库存流水、不校验库存、不验证客户
fn is_retail_sale_inv_by_data(meta: &DocMeta, data: &serde_json::Value) -> bool {
    if meta.table != "tSal_Inv" {
        return false;
    }
    data.get("BTPID").and_then(|v| v.as_str()).unwrap_or("") == RETAIL_SALE_INV_BTPID
}

/// 判断是否为门店销售单（从数据库查 BTPID）
/// 用于审核/反审流程：此时只有 master_id，需查 DB 获取 BTPID
async fn is_retail_sale_inv_by_db(conn: &mut Conn, table: &str, pk: &str, master_id: &str) -> bool {
    if table != "tSal_Inv" {
        return false;
    }
    let sql = format!(
        "SELECT CAST(BTPID AS NVARCHAR(40)) AS B FROM [{}] WHERE [{}] = @p1",
        table, pk
    );
    match conn.query(&sql, &[&master_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .next()
                .and_then(|r| r.get::<&str, _>("B"))
                .map(|b| b == RETAIL_SALE_INV_BTPID)
                .unwrap_or(false),
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// 校验主表必填字段
fn validate_required_fields(meta: &DocMeta, data: &serde_json::Value) -> Result<(), String> {
    let obj = data.as_object().ok_or("data 必须是 JSON 对象")?;
    // 门店销售单（tSal_Inv + RETAIL_SALE_INV_BTPID）是门店自卖，无需客户
    // 同一张 tSal_Inv 表既用于销售出库（需 CustID）也用于门店销售（不需 CustID），按 BTPID 区分
    let is_retail_sale_inv = is_retail_sale_inv_by_data(meta, data);
    for f in &meta.required_fields {
        if meta.primary_key == *f {
            // PK 允许为空（create 时前端尚未生成）
            continue;
        }
        if is_retail_sale_inv && f == "CustID" {
            // 门店销售单是门店（商场）自卖，不验证客户
            continue;
        }
        // 重构：用 if let Some 模式替代 is_none()+unwrap() 链，避免重复 unwrap
        let v = match obj.get(f.as_str()) {
            None => return Err(format!("主表必填字段 {} 缺失", f)),
            Some(val) if val.is_null() => return Err(format!("主表必填字段 {} 缺失", f)),
            Some(val) => val,
        };
        if let Some(s) = v.as_str() {
            if s.is_empty() {
                return Err(format!("主表必填字段 {} 不能为空", f));
            }
        }
    }
    Ok(())
}

/// 校验明细至少 1 行（仅对有明细表且影响库存的单据强制要求）
/// - 无明细表的扁平表（如 tSal_EmpSales, tFin_CashFlow）：跳过
/// - 有明细表但不影响库存（如 tFin_Receipt, tFin_Payment）：明细可选（用于核销，可为空）
/// - 有明细表且影响库存（如 tPur_Order, tStk_IO）：明细必填（库存校验需要）
fn validate_details_nonempty(meta: &DocMeta, details: &[serde_json::Value]) -> Result<(), String> {
    if meta.detail_table.is_empty() {
        return Ok(());
    }
    if !meta.affects_stock {
        return Ok(());
    }
    if details.is_empty() {
        return Err("明细至少需要 1 行".to_string());
    }
    Ok(())
}

/// 校验明细商品不重复（detail_unique_gds=true 时）
fn validate_details_unique_gds(
    meta: &DocMeta,
    details: &[serde_json::Value],
) -> Result<(), String> {
    if !meta.detail_unique_gds {
        return Ok(());
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (idx, d) in details.iter().enumerate() {
        let gdsid = d.get("GDSID").and_then(|v| v.as_str()).unwrap_or("");
        if gdsid.is_empty() {
            return Err(format!("明细第 {} 行 GDSID 必填", idx + 1));
        }
        if let Some(&prev) = seen.get(gdsid) {
            return Err(format!(
                "明细商品 {} 重复（第 {} 行与第 {} 行）",
                gdsid,
                prev + 1,
                idx + 1
            ));
        }
        // 校验数量（仅当明细行含 Qty 字段时；盘点单用 DiffQty，跳过此校验）
        if d.get("Qty").is_some() {
            let qty = json_to_f64(d.get("Qty"));
            if qty <= 0.0 {
                return Err(format!(
                    "明细第 {} 行数量必须 > 0（当前值: {:?}）",
                    idx + 1,
                    d.get("Qty")
                ));
            }
        }
        seen.insert(gdsid.to_string(), idx);
    }
    Ok(())
}

/// 校验收款单/付款单的核销明细行：
/// 1. 源单（tStk_IO）必须存在且 State IN ('S','Y')
/// 2. 源单 Kind 必须与单据类型匹配（receipt 核销 SD/SI/POS/SR，payment 核销 PD/PR）
/// 3. 源单的客户/供应商必须与主表一致，避免跨客户核销污染 AR/AP 报表
/// 4. 已审核+当前单据的核销合计不得超过源单 SumAmt（防止 OpenAmt 变负）
///
/// 设计说明：OpenAmt 是 finance.rs 的派生计算字段（io.SumAmt - SUM(其他已审核单据明细 Amt)），
/// 数据库无物理约束，必须由后端业务层校验。否则绕过前端直接调 /generic/create 即可让 OpenAmt 变负。
async fn validate_writeoff_details(
    conn: &mut Conn,
    meta: &DocMeta,
    data: &serde_json::Value,
    details: &[serde_json::Value],
    is_update: bool,
    current_master_id: &str,
) -> Result<(), ApproveError> {
    // 仅对收款单/付款单生效
    let is_receipt = meta.table == "tFin_Receipt";
    let is_payment = meta.table == "tFin_Payment";
    if !is_receipt && !is_payment {
        return Ok(());
    }
    if details.is_empty() {
        return Ok(());
    }

    // 主表的客户/供应商 ID（用于跨客户校验）
    let party_field = if is_receipt { "CustID" } else { "SuppID" };
    let party_id = data
        .get(party_field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 源单 Kind 白名单
    let allowed_kinds = if is_receipt {
        vec!["SD", "SI", "POS", "SR"]
    } else {
        vec!["PD", "PR"]
    };
    let allowed_kinds_str = allowed_kinds
        .iter()
        .map(|k| format!("'{}'", k))
        .collect::<Vec<_>>()
        .join(", ");

    // 明细表名 + 主表关联字段（用于查"其他已审核单据"的核销合计）
    let (dtl_table, dtl_fk, master_table, master_pk) = if is_receipt {
        ("tFin_ReceiptDtl", "RecID", "tFin_Receipt", "RecID")
    } else {
        ("tFin_PaymentDtl", "PayID", "tFin_Payment", "PayID")
    };

    for (idx, d) in details.iter().enumerate() {
        let source_doc_id = d
            .get("SourceDocID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let amt = json_to_f64(d.get("Amt"));
        if source_doc_id.is_empty() || amt <= 0.0 {
            continue; // 空行跳过
        }

        // 1+2+3) 查源单：必须存在、State IN ('S','Y')、Kind 匹配、CustID/SuppID 匹配
        // 同时取 SumAmt 用于后续金额校验，避免再次查询（也避免借用冲突）
        // ★ SumAmt 是 decimal(18,2)，tiberius 的 row.get::<f64,_> 对 NUMERIC 类型返回 None，
        //   被 unwrap_or(0.0) 吞掉会导致核销金额校验失效（总认为源单金额=0，任何核销都超分配）。
        //   修复：SQL 中显式 CAST AS FLOAT，让 tiberius 按 f64 返回。
        let party_col = if is_receipt { "CustID" } else { "SuppID" };
        let sql_source = format!(
            r#"SELECT CAST(io.SumAmt AS FLOAT) AS SumAmt, io.Kind, io.{} AS PartyID
               FROM tStk_IO io
               WHERE io.IOID = @p1
                 AND io.State IN ('S','Y')
                 AND io.Kind IN ({})"#,
            party_col, allowed_kinds_str
        );
        // 把 stream 的结果立即提取出来，结束 conn 的借用，避免后续 query 时的借用冲突
        let (sum_amt, src_party): (f64, String) = {
            let stream = conn.query(&sql_source, &[&source_doc_id]).await;
            match stream {
                Ok(s) => {
                    match s.into_row().await {
                        Ok(Some(row)) => {
                            let sum_amt: f64 = row.get::<f64, _>("SumAmt").unwrap_or(0.0);
                            let kind: String = row.get::<&str, _>("Kind").unwrap_or("").to_string();
                            let src_party: String =
                                row.get::<&str, _>("PartyID").unwrap_or("").to_string();
                            if kind.is_empty() {
                                return Err(ApproveError::msg(format!(
                                    "明细第 {} 行：源单 {} 不存在或状态不允许核销（必须已审核/已确认）",
                                    idx + 1,
                                    source_doc_id
                                )));
                            }
                            // 跨客户/供应商校验
                            if !party_id.is_empty()
                                && !src_party.is_empty()
                                && src_party != party_id
                            {
                                return Err(ApproveError::msg(format!(
                                    "明细第 {} 行：源单的客户/供应商与单据不匹配，禁止跨客户核销",
                                    idx + 1
                                )));
                            }
                            (sum_amt, src_party)
                        }
                        _ => {
                            return Err(ApproveError::msg(format!(
                                "明细第 {} 行：源单 {} 不存在或 Kind/状态不匹配",
                                idx + 1,
                                source_doc_id
                            )));
                        }
                    }
                }
                Err(e) => {
                    return Err(ApproveError::msg(format!(
                        "校验核销明细时查询源单失败: {}",
                        e
                    )));
                }
            }
        };
        let _ = src_party; // 已在循环内使用

        // 4) 金额超分配校验：
        // 已审核单据（不含当前单据）对该源单的核销合计 + 当前行 Amt 不得超过源单 SumAmt
        // 编辑模式下需排除当前单据自身（否则已审核单据编辑时会被误判超分配）
        let exclude_clause = if is_update && !current_master_id.is_empty() {
            format!("AND m.{} <> @p2", master_pk)
        } else {
            String::new()
        };
        let sql_sum = format!(
            r#"SELECT CAST(ISNULL(SUM(d.Amt), 0) AS FLOAT) AS AlreadyWoff
               FROM {} d
               INNER JOIN {} m ON m.{} = d.{}
               WHERE d.SourceDocID = @p1
                 AND m.State IN ('S','Y')
                 {}"#,
            dtl_table, master_table, master_pk, dtl_fk, exclude_clause
        );
        let already_woff: f64 = if is_update && !current_master_id.is_empty() {
            let s = conn
                .query(&sql_sum, &[&source_doc_id, &current_master_id])
                .await
                .map_err(|e| ApproveError::msg(format!("查询已核销金额失败: {}", e)))?;
            let row = s
                .into_row()
                .await
                .map_err(|e| ApproveError::msg(format!("读取已核销金额失败: {}", e)))?;
            row.map(|r| r.get::<f64, _>("AlreadyWoff").unwrap_or(0.0))
                .unwrap_or(0.0)
        } else {
            let s = conn
                .query(&sql_sum, &[&source_doc_id])
                .await
                .map_err(|e| ApproveError::msg(format!("查询已核销金额失败: {}", e)))?;
            let row = s
                .into_row()
                .await
                .map_err(|e| ApproveError::msg(format!("读取已核销金额失败: {}", e)))?;
            row.map(|r| r.get::<f64, _>("AlreadyWoff").unwrap_or(0.0))
                .unwrap_or(0.0)
        };

        let total_after = already_woff + amt;
        if total_after > sum_amt + 0.01 {
            return Err(ApproveError::msg(format!(
                "明细第 {} 行：核销金额 {} 超过源单剩余可核销金额（源单金额 {}，其他已核销 {}）",
                idx + 1,
                amt,
                sum_amt,
                already_woff
            )));
        }
    }
    Ok(())
}

/// 把 JSON 值统一解析为 f64（兼容 Number / String / null）
fn json_to_f64(v: Option<&serde_json::Value>) -> f64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 查询商品名称（GDSDesc）+ 编码（GDSNO），用于错误提示
async fn query_gds_info(conn: &mut Conn, gdsid: &str) -> (String, String) {
    if gdsid.is_empty() {
        return (String::new(), String::new());
    }
    let sql =
        "SELECT ISNULL(GDSNO,'') AS NO, ISNULL(GDSDesc,'') AS NM FROM tBas_Goods WHERE GDSID = @p1";
    if let Ok(stream) = conn.query(sql, &[&gdsid]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let no = row.get::<&str, _>("NO").unwrap_or("").to_string();
            let nm = row.get::<&str, _>("NM").unwrap_or("").to_string();
            return (no, nm);
        }
    }
    (String::new(), String::new())
}

/// 查询仓库名称（StkName）+ 编码（StkCode），用于错误提示
async fn query_stk_info(conn: &mut Conn, stkid: &str) -> (String, String) {
    if stkid.is_empty() {
        return (String::new(), String::new());
    }
    let sql = "SELECT ISNULL(StkCode,'') AS NO, ISNULL(StkName,'') AS NM FROM tBas_Stock WHERE StkID = @p1";
    if let Ok(stream) = conn.query(sql, &[&stkid]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let no = row.get::<&str, _>("NO").unwrap_or("").to_string();
            let nm = row.get::<&str, _>("NM").unwrap_or("").to_string();
            return (no, nm);
        }
    }
    (String::new(), String::new())
}

/// 格式化商品显示文本：GDSDesc(GDSNO) 或 GDSID
fn fmt_gds(gdsid: &str, gds_no: &str, gds_name: &str) -> String {
    if !gds_name.is_empty() || !gds_no.is_empty() {
        if !gds_name.is_empty() && !gds_no.is_empty() {
            format!("{}({})", gds_name, gds_no)
        } else {
            gds_name.to_string() + gds_no
        }
    } else {
        gdsid.to_string()
    }
}

/// 格式化仓库显示文本：StkName(StkNO) 或 StkID
fn fmt_stk(stkid: &str, stk_no: &str, stk_name: &str) -> String {
    if !stk_name.is_empty() || !stk_no.is_empty() {
        if !stk_name.is_empty() && !stk_no.is_empty() {
            format!("{}({})", stk_name, stk_no)
        } else {
            stk_name.to_string() + stk_no
        }
    } else {
        stkid.to_string()
    }
}

/// 校验出库类库存可用量（保存时）
/// 可用量 = Qty - QQty（当前库存 - 预占量），不允许负库存
/// 返回 ApproveError::Shortage 结构化数据，前端用表格弹窗展示
/// 同时把缺货明细持久化到 tStk_Shortage（缺货记录页面）
async fn validate_outbound_stock(
    conn: &mut Conn,
    meta: &DocMeta,
    data: &serde_json::Value,
    details: &[serde_json::Value],
    user_code: &str,
    emp_id: &str,
    pk_value: &str,
) -> Result<(), ApproveError> {
    // 只有影响库存且 detail 表含 GDSID/StkID/Qty 时才校验
    if !meta.affects_stock {
        return Ok(());
    }
    tracing::info!(
        table = %meta.table,
        kind_field = %meta.kind_field,
        doc_no_prefix = %meta.doc_no_prefix,
        detail_count = details.len(),
        "[validate_outbound_stock] 入口"
    );
    // 判断是否为出库类单据（只有出库类才需要校验库存可用量）
    // - 有 kind_field 的表（tStk_IO/tStk_Move）：按 Kind 值判断方向
    // - 无 kind_field 的表（tSal_Inv/tPur_Return 等）：按 doc_type 判断固定方向
    //   sales_outbound/sales_inv(销售出库) / purchase_return(采购退货) = 出库，需校验
    //   purchase_inbound/sales_return(入库类) = 不校验
    //   stock_move(调拨) = 由 approve 路径的 validate_move_outbound_stock 处理，save 路径跳过
    let need_check = if !meta.kind_field.is_empty() {
        // tStk_IO / tStk_Move：按 Kind 判断方向
        let kind = data
            .get(meta.kind_field.as_str())
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dir = doc_graph::kind_direction(kind);
        tracing::info!(table = %meta.table, kind = %kind, direction = dir, "[validate_outbound_stock] kind_field 分支");
        dir == doc_graph::DIR_OUTBOUND
    } else {
        // 无 Kind 字段的单据表：按 doc_type 判断固定方向
        let doc_type = default_doc_type_for_table(meta);
        let matched = matches!(
            doc_type.as_str(),
            "sales_outbound" | "sales_inv" | "purchase_return"
        );
        tracing::info!(table = %meta.table, doc_type = %doc_type, matched = matched, "[validate_outbound_stock] doc_type 分支");
        matched
    };
    if !need_check {
        tracing::info!(table = %meta.table, "[validate_outbound_stock] 跳过（非出库类）");
        return Ok(());
    }
    let mut shortage: Vec<StockShortageItem> = Vec::new();
    // 销售出库：若有源销售订单（SOID），审核时会先释放预占再出库，净效果 (Qty-QQty) 不变
    //   → 校验 stock >= qty 即可（与 validate_outbound_stock_for_approve 一致）
    // 其他出库类（采购退货等）：校验 available = stock - qqty >= qty
    let doc_type = default_doc_type_for_table(meta);
    let is_sales_outbound = matches!(doc_type.as_str(), "sales_outbound" | "sales_inv");
    let source_soid = if is_sales_outbound {
        data.get("SOID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };
    let is_sales_outbound_with_reserve = is_sales_outbound && !source_soid.is_empty();
    for (idx, d) in details.iter().enumerate() {
        let gdsid = d
            .get("GDSID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let stkid = d
            .get("StkID")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let qty = json_to_f64(d.get("Qty"));
        if gdsid.is_empty() || stkid.is_empty() || qty <= 0.0 {
            continue;
        }
        let stock = query_stock_qty(conn, &gdsid, &stkid).await;
        let qqty = query_qqty(conn, &gdsid, &stkid).await;
        let available = stock - qqty;
        // 有源单的销售出库：校验 stock >= qty（预占会被先释放）
        // 其他情况：校验 available >= qty（数据库 CHECK 约束 Qty >= QQty）
        let check_ok = if is_sales_outbound_with_reserve {
            stock >= qty - 0.0001
        } else {
            available >= qty - 0.0001
        };
        if !check_ok {
            let (gds_no, gds_name) = query_gds_info(conn, &gdsid).await;
            let (stk_no, stk_name) = query_stk_info(conn, &stkid).await;
            let short_qty = if is_sales_outbound_with_reserve {
                (qty - stock).ceil()
            } else {
                (qty - available).ceil()
            };
            shortage.push(StockShortageItem {
                row_no: idx + 1,
                gds_id: gdsid.clone(),
                stk_id: stkid.clone(),
                gds_no,
                gds_name,
                stk_no,
                stk_name,
                stock,
                reserved: qqty,
                available,
                qty,
                shortage: short_qty,
            });
        }
    }
    if !shortage.is_empty() {
        // 持久化缺货记录到 tStk_Shortage（save 场景：单据号从 data 读取，master_id 用 pk_value）
        let doc_no = if !meta.no_field.is_empty() {
            data.get(meta.no_field.as_str())
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        };
        // 读取客户/门店（销售类单据有 CustID；调拨/盘点等无客户则留空）
        let (cust_id, cust_name) = query_cust_info_from_data(conn, meta, data).await;
        // 销售类单据无独立门店字段，门店列为空（门店信息在调拨单 ZP 场景才填充）
        log_shortage_to_db(
            conn,
            &shortage,
            &meta.table,
            &doc_no,
            pk_value,
            user_code,
            emp_id,
            "doc_save",
            &cust_id,
            &cust_name,
            "",
            "",
        )
        .await;
        return Err(ApproveError::Shortage(shortage));
    }
    Ok(())
}

// ============== 主入口: save ==============

pub async fn save_doc(
    conn: &mut Conn,
    user_code: &str,
    user_name: &str,
    mut params: SaveDocParams,
) -> Result<SaveDocResponse, ApproveError> {
    let meta = doc_graph::get_doc_meta(&params.table)
        .ok_or_else(|| format!("未知业务单据表: {}", params.table))?
        .clone();
    let pk_value = params
        .data
        .get(&params.primary_key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let is_update = !pk_value.is_empty();
    let operation = if is_update { "UPDATE" } else { "CREATE" };
    // 操作类型中文动词，用于日志描述
    let oper_cn = if is_update { "修改" } else { "新增" };

    // 自动注入 EUser / EDate（由后端从登录用户填充，前端不必传）
    // EUser 字段是 uniqueidentifier 类型，存的是 EmpID，需要通过 user_code 查 tBas_Emp 获取
    // resolved_emp_id 的所有分支都会在后续使用前被重新赋值，初始空值仅作占位
    #[allow(unused_assignments)]
    let mut resolved_emp_id = String::new();
    {
        let obj = params.data.as_object_mut().ok_or("data 必须是 JSON 对象")?;
        let euser_val = obj
            .get("EUser")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let need_inject_euser = euser_val.is_empty();
        if need_inject_euser {
            // 通过 user_code 查询 EmpID
            let emp_id = query_emp_id_by_code(conn, user_code).await;
            if !emp_id.is_empty() {
                obj.insert(
                    "EUser".to_string(),
                    serde_json::Value::String(emp_id.clone()),
                );
                resolved_emp_id = emp_id;
            } else {
                // P1-6 修复：原逻辑静默写入零 UUID，导致 EUser 字段污染，影响数据完整性
                //   （用户偏好明确要求不显示零 UUID；项目约束要求自动填充当前登录用户 EmpID）
                //   查不到 EmpID 说明登录用户在 tBas_Emp 表中无记录，属于数据完整性问题
                //   改为返回错误阻止保存，提示用户联系管理员修复员工档案
                tracing::error!(
                    user_code = %user_code,
                    "无法解析当前用户 EmpID：登录用户在 tBas_Emp 表中无对应记录，拒绝保存单据"
                );
                return Err(ApproveError::msg(format!(
                    "无法解析当前用户 EmpID（工号 {} 在员工档案中不存在），请联系管理员修复员工档案",
                    user_code
                )));
            }
        } else if !euser_val.contains('-') {
            // EUser 不是 UUID 格式（如 'admin'），替换为对应 EmpID
            let emp_id = query_emp_id_by_code(conn, &euser_val).await;
            if !emp_id.is_empty() {
                obj.insert(
                    "EUser".to_string(),
                    serde_json::Value::String(emp_id.clone()),
                );
                resolved_emp_id = emp_id;
            } else {
                // P1-6 修复：同上，返回错误而非写入零 UUID
                tracing::error!(
                    euser_val = %euser_val,
                    "前端传入的 EUser 不是 UUID 且无法在 tBas_Emp 表中查到对应 EmpID"
                );
                return Err(ApproveError::msg(format!(
                    "无法解析 EUser 字段（值 '{}' 不是 UUID 且在员工档案中不存在）",
                    euser_val
                )));
            }
        } else {
            // 前端已传 UUID 格式的 EUser，直接复用作为缺货记录的 EmpID
            resolved_emp_id = euser_val;
        }
        if !obj.contains_key("EDate")
            || obj
                .get("EDate")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty()
        {
            // ★ 用 Local::now()（本地时区 UTC+8），不能用 Utc::now()（会差 8 小时）。
            //   格式用 "YYYY-MM-DD HH:MM:SS"（与 generic.rs 一致，SQL Server DATETIME 隐式转换无歧义）。
            obj.insert(
                "EDate".to_string(),
                serde_json::Value::String(
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                ),
            );
        }
    }

    // 1) 校验
    validate_required_fields(&meta, &params.data)?;
    validate_details_nonempty(&meta, &params.details)?;
    validate_details_unique_gds(&meta, &params.details)?;
    // 门店销售单是门店自卖，不进库存流水、不校验库存
    if !is_retail_sale_inv_by_data(&meta, &params.data) {
        validate_outbound_stock(
            conn,
            &meta,
            &params.data,
            &params.details,
            user_code,
            &resolved_emp_id,
            &pk_value,
        )
        .await?;
    }

    // 修复 P0-1：更新模式下校验当前 DB 状态，仅 N/E 可修改
    // 否则已审核（S）/已确认（Y）/已作废（C）/已删除（D）单据仍可被覆盖，
    // 特别是 receipt/payment 的明细 Amt 变化会立即影响 finance.rs 的 OpenAmt 派生计算
    if is_update {
        let cur_state =
            inventory_ledger::query_doc_state(conn, &params.table, &params.primary_key, &pk_value)
                .await;
        if !matches!(cur_state.as_str(), "" | "N" | "E") {
            // 先反审或新单据才能修改
            return Err(ApproveError::msg(format!(
                "单据状态为 {}，不允许修改（仅新建/编辑中可修改）",
                cur_state
            )));
        }
    }

    // 修复 P0-3/P0-4：核销明细金额超分配 + 源单存在性/Kind/客户校验
    // 防止绕过前端直接调 /generic/create 让 OpenAmt 变负或污染 AR/AP 报表
    validate_writeoff_details(
        conn,
        &meta,
        &params.data,
        &params.details,
        is_update,
        &pk_value,
    )
    .await?;

    // 基于表结构过滤主表字段并补全 NOT NULL 默认值（统一处理所有单据类型）
    {
        let columns = query_table_columns(conn, &params.table).await;
        if let Some(obj) = params.data.as_object_mut() {
            // 先过滤掉数据库不存在的字段（_isNew/_rowKey/前端展示字段等）
            filter_to_db_columns(obj, &columns, &[&params.primary_key]);
            // 再补全 NOT NULL 字段默认值
            fill_not_null_defaults(obj, &columns, &[&params.primary_key]);
        }
    }

    // 2) 写主表 + 明细（外层事务保护，避免孤儿主表/明细）
    if let Err(e) = inventory_ledger::begin_tran(conn).await {
        return Err(e.into());
    }
    // 修复 M-6：before_snapshot 在事务内查询，避免与其他事务并发修改产生快照漂移
    // （原实现查询在 begin_tran 之前，期间若有其他事务提交更新，before 快照将过时）
    let before_snapshot: Option<serde_json::Value> = if is_update {
        query_doc_snapshot(
            conn,
            &params.table,
            &params.primary_key,
            &pk_value,
            &meta.detail_table,
            &meta.detail_foreign_key,
            &meta.detail_primary_key,
        )
        .await
    } else {
        None
    };
    let mut tx_err: Option<String> = None;
    let id = if is_update {
        match update_master(conn, &params.table, &params.primary_key, &params.data).await {
            Ok(_) => pk_value.clone(),
            Err(e) => {
                tx_err = Some(e);
                String::new()
            }
        }
    } else {
        match insert_master(conn, &params.table, &params.primary_key, &params.data).await {
            Ok(id) => id,
            Err(e) => {
                tx_err = Some(e);
                String::new()
            }
        }
    };

    // 3) 重写明细（先删后插）
    if tx_err.is_none() && !params.details.is_empty() {
        // 物理删除（明细表无 State 字段）
        if let Err(e) = delete_details(conn, &meta, &id).await {
            tx_err = Some(e);
        }
        // 基于表结构过滤明细字段并补全 NOT NULL 默认值
        let det_columns = if !meta.detail_table.is_empty() {
            query_table_columns(conn, &meta.detail_table).await
        } else {
            Vec::new()
        };
        if tx_err.is_none() {
            for (row_idx, d) in params.details.iter_mut().enumerate() {
                if let Some(obj) = d.as_object_mut() {
                    // 先过滤掉数据库不存在的字段（_isNew/_rowKey/AInPrice/前端展示字段等）
                    filter_to_db_columns(
                        obj,
                        &det_columns,
                        &[&meta.detail_primary_key, &meta.detail_foreign_key, "RowNO"],
                    );
                    // 再补全 NOT NULL 字段默认值
                    fill_not_null_defaults(
                        obj,
                        &det_columns,
                        &[&meta.detail_primary_key, &meta.detail_foreign_key, "RowNO"],
                    );
                }
                if let Err(e) = insert_detail(conn, &meta, &id, d, row_idx as i32).await {
                    tx_err = Some(e);
                    break;
                }
            }
        }
        // 回填库存快照
        if tx_err.is_none() {
            post_save_fill_snapshot(conn, &meta, &id).await;
        }
    }

    if let Some(e) = tx_err {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e.into());
    }
    if let Err(e) = inventory_ledger::commit_tran(conn).await {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e.into());
    }

    // 4) 取单据号
    let doc_no = params
        .data
        .get(meta.no_field.as_str())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 5) 保存并审核：auto_approve=true 时调用审核逻辑，只写一条 APPROVE 日志
    if params.auto_approve.unwrap_or(false) {
        let approve_params = ApproveDocParams {
            table: params.table.clone(),
            primary_key: params.primary_key.clone(),
            id: id.clone(),
            doc_type: None,
        };
        // 把保存前的快照传给审核，作为日志的 before（修改前数据）
        // 保存已提交，若审核失败不回滚保存，返回 partial_success 让前端切换为编辑模式
        match approve_doc_internal(
            conn,
            user_code,
            user_name,
            approve_params,
            before_snapshot.clone(),
        )
        .await
        {
            Ok(_) => {
                // ★ 门店销售单审核成功后自动计算提成（对齐 88 项目 storesales.go）
                //   审核路径与保存路径都会触发提成重算，保证数据一致
                if params.table == "tSal_Inv" {
                    if let Err(e) =
                        crate::services::commission_service::recalc_invoice_commission(conn, &id)
                            .await
                    {
                        tracing::warn!(
                            "[save_doc/approve] 门店销售单 {} 提成重算失败（不影响审核）: {}",
                            id,
                            e
                        );
                    }
                }
                return Ok(SaveDocResponse {
                    id,
                    doc_no,
                    operation: "APPROVE".to_string(),
                    partial_success: None,
                    approve_error: None,
                    shortage_list: None,
                });
            }
            Err(approve_err) => {
                let err_msg = match &approve_err {
                    ApproveError::Shortage(items) => format!("库存不足：{} 项", items.len()),
                    ApproveError::Msg(msg) => msg.clone(),
                };
                let shortage_list = match &approve_err {
                    ApproveError::Shortage(items) => Some(items.clone()),
                    _ => None,
                };
                return Ok(SaveDocResponse {
                    id,
                    doc_no,
                    operation: "SAVE_ONLY".to_string(),
                    partial_success: Some(true),
                    approve_error: Some(err_msg),
                    shortage_list,
                });
            }
        }
    }

    // 6) 写操作日志（含数据快照，日志失败不影响单据保存，仅记录）
    // after 快照重新查询 DB：保证 before/after 都是完整行（含 Items），避免请求体只有部分字段导致对比失真
    let after_snapshot: Option<serde_json::Value> = query_doc_snapshot(
        conn,
        &params.table,
        &params.primary_key,
        &id,
        &meta.detail_table,
        &meta.detail_foreign_key,
        &meta.detail_primary_key,
    )
    .await;
    let after_data = after_snapshot.unwrap_or_else(|| params.data.clone());
    let (before_json, after_json): (Option<String>, Option<String>) = if is_update {
        let before_str = before_snapshot
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        let after_str = serde_json::to_string(&after_data).ok();
        (before_str, after_str)
    } else {
        (None, serde_json::to_string(&after_data).ok())
    };
    let _ = record_oper_with_data(
        conn,
        operation,
        &params.table,
        &id,
        user_code,
        Some(&doc_no),
        Some(&format!("{}{}", oper_cn, meta.title)),
        before_json.as_deref(),
        after_json.as_deref(),
    )
    .await;

    // ★ 门店销售单保存成功后自动计算提成（对齐 88 项目 storesales.go）
    //   - 88 项目在 CreateStoreSalesOrder/UpdateStoreSalesOrder 中直接调用 CalculateOrderCommission
    //   - 当前项目统一在 save_doc 末尾调用，覆盖 CREATE + UPDATE 两种场景
    //   - 仅对 tSal_Inv 表生效（其他单据表跳过）
    //   - 提成计算失败不影响单据保存（仅记录日志），保证主流程稳定
    if params.table == "tSal_Inv" {
        if let Err(e) =
            crate::services::commission_service::recalc_invoice_commission(conn, &id).await
        {
            tracing::warn!(
                "[save_doc] 门店销售单 {} 提成重算失败（不影响保存）: {}",
                id,
                e
            );
        }
    }

    Ok(SaveDocResponse {
        id,
        doc_no,
        operation: operation.to_string(),
        partial_success: None,
        approve_error: None,
        shortage_list: None,
    })
}

/// 查询单据快照（主表 + 明细行），用于操作日志变更明细对比
///
/// 关键设计：使用 generic.rs 的 `get_joins_for_table` JOIN 配置，
/// 让快照中除 ID 字段外还包含 SuppName/CustName/DeptName/EmpName/StkName 等名称字段，
/// 前端 useChangeDetail 解析时即可将 UUID 显示为对应名称（而非截断前8位）。
async fn query_doc_snapshot(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    pk_value: &str,
    detail_table: &str,
    detail_foreign_key: &str,
    detail_primary_key: &str,
) -> Option<serde_json::Value> {
    use crate::handlers::base_data::row_to_json;
    use crate::handlers::generic::get_joins_for_table;

    // 查主表（应用 JOIN 配置，让结果含 Name 字段供前端 UUID→名称解析）
    let (master_select, master_join) = get_joins_for_table(table);
    let master_sql = if master_join.is_empty() {
        format!(
            "SELECT {} FROM [{}] t WHERE t.[{}] = @p1",
            master_select, table, primary_key
        )
    } else {
        format!(
            "SELECT {} FROM [{}] t {} WHERE t.[{}] = @p1",
            master_select, table, master_join, primary_key
        )
    };
    let master_val = match conn.query(&master_sql, &[&pk_value]).await {
        Ok(stream) => match stream.into_row().await {
            Ok(Some(row)) => row_to_json(&row),
            _ => return None,
        },
        Err(_) => return None,
    };
    let mut master_obj = master_val.as_object()?.clone();
    // 查明细表（同样应用 JOIN 配置，含 GDSDesc/GoodsGDSNO/UnitName/BrandName 等）
    let mut details: Vec<serde_json::Value> = Vec::new();
    if !detail_table.is_empty() && !detail_foreign_key.is_empty() {
        let order_clause = if detail_primary_key.is_empty() {
            "1".to_string()
        } else {
            detail_primary_key.to_string()
        };
        let (detail_select, detail_join) = get_joins_for_table(detail_table);
        let detail_sql = if detail_join.is_empty() {
            format!(
                "SELECT {} FROM [{}] t WHERE t.[{}] = @p1 ORDER BY t.[{}]",
                detail_select, detail_table, detail_foreign_key, order_clause
            )
        } else {
            format!(
                "SELECT {} FROM [{}] t {} WHERE t.[{}] = @p1 ORDER BY t.[{}]",
                detail_select, detail_table, detail_join, detail_foreign_key, order_clause
            )
        };
        if let Ok(stream) = conn.query(&detail_sql, &[&pk_value]).await {
            if let Ok(rows) = stream.into_first_result().await {
                for row in rows {
                    details.push(row_to_json(&row));
                }
            }
        }
    }
    master_obj.insert("Items".to_string(), serde_json::Value::Array(details));
    Some(serde_json::Value::Object(master_obj))
}

async fn update_master(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    data: &serde_json::Value,
) -> Result<String, String> {
    let obj = data.as_object().ok_or("data 必须是 JSON 对象")?;
    let pk_value = obj
        .get(primary_key)
        .and_then(|v| v.as_str())
        .ok_or("缺少主键")?
        .to_string();
    // 构造 SET 子句（排除主键、Name 结尾展示字段）
    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Send + Sync>> = Vec::new();
    let mut idx = 1;
    for (k, v) in obj {
        if k == primary_key {
            continue;
        }
        // 过滤前端内部字段（以 _ 开头，如 _isNew/_rowKey）
        if k.starts_with('_') {
            continue;
        }
        if k.ends_with("Name") && !k.starts_with("GDS") {
            continue;
        }
        // 业务单号、PK 不修改
        if k == "PoNo"
            || k == "PiNo"
            || k == "PrNo"
            || k == "SoNo"
            || k == "SINo"
            || k == "IONo"
            || k == "MoveNO"
            || k == "TranNo"
            || k == "ReplenishApplyNo"
            || k == "ReceiptNo"
            || k == "PaymentNo"
        {
            continue;
        }
        sets.push(format!("[{}] = @p{}", k, idx));
        params.push(json_to_sql_for_field(k, v));
        idx += 1;
    }
    // 修复 H-1：添加 CAS 条件 State IN ('','N','E')，防止事务内并发请求把已审核单据覆盖
    // UPDATE 会获取 X 锁，并发请求的 UPDATE 会被阻塞到本事务提交；
    // 提交后并发的 CAS 会因 State 已变（如 S）而失败（rows_affected=0）
    let cas_states: [&str; 3] = ["", "N", "E"];
    let cas_placeholders: Vec<String> = (0..cas_states.len())
        .map(|i| format!("@p{}", idx + 1 + i))
        .collect();
    let sql = format!(
        "UPDATE [{}] SET {} WHERE [{}] = @p{} AND (State IS NULL OR State IN ({}))",
        table,
        sets.join(", "),
        primary_key,
        idx,
        cas_placeholders.join(", ")
    );
    params.push(Box::new(pk_value.clone()));
    for s in cas_states.iter() {
        params.push(Box::new(s.to_string()));
    }
    let p_refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref() as &dyn ToSql).collect();
    let result = conn
        .execute(&sql, &p_refs)
        .await
        .map_err(|e| format!("更新主表失败: {}", e))?;
    let rows = result.rows_affected().first().copied().unwrap_or(0);
    if rows == 0 {
        // 状态被并发请求改掉（如已审核 S），返回错误让外层 ROLLBACK
        return Err(format!(
            "单据状态已变更，修改失败（仅新建/编辑中状态可修改）"
        ));
    }
    Ok(pk_value)
}

async fn insert_master(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    data: &serde_json::Value,
) -> Result<String, String> {
    let obj = data.as_object().ok_or("data 必须是 JSON 对象")?;
    let mut cols: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Send + Sync>> = Vec::new();
    let mut placeholders: Vec<String> = Vec::new();
    let mut idx = 1;
    let mut new_pk: Option<String> = None;
    for (k, v) in obj {
        if k == primary_key {
            // create 时若前端传了空值则生成新 UUID
            if v.is_null() || v.as_str().map(|s| s.is_empty()).unwrap_or(true) {
                let uuid = uuid_v4();
                cols.push(format!("[{}]", k));
                placeholders.push(format!("@p{}", idx));
                params.push(Box::new(uuid.clone()));
                new_pk = Some(uuid);
                idx += 1;
                continue;
            }
            new_pk = v.as_str().map(|s| s.to_string());
        }
        // 过滤前端内部字段（以 _ 开头，如 _isNew/_rowKey）
        if k.starts_with('_') {
            continue;
        }
        if k.ends_with("Name") && !k.starts_with("GDS") {
            continue;
        }
        cols.push(format!("[{}]", k));
        placeholders.push(format!("@p{}", idx));
        params.push(json_to_sql_for_field(k, v));
        idx += 1;
    }
    // data 里没有 primary_key 字段时，自动生成新 UUID 并加入 INSERT 列
    let new_pk = if let Some(pk) = new_pk {
        pk
    } else {
        let uuid = uuid_v4();
        cols.push(format!("[{}]", primary_key));
        placeholders.push(format!("@p{}", idx));
        params.push(Box::new(uuid.clone()));
        uuid
    };
    let sql = format!(
        "INSERT INTO [{}] ({}) VALUES ({})",
        table,
        cols.join(", "),
        placeholders.join(", ")
    );
    let p_refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref() as &dyn ToSql).collect();
    conn.execute(&sql, &p_refs)
        .await
        .map_err(|e| format!("插入主表失败: {}", e))?;
    Ok(new_pk)
}

async fn delete_details(conn: &mut Conn, meta: &DocMeta, master_id: &str) -> Result<(), String> {
    if meta.detail_table.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "DELETE FROM [{}] WHERE [{}] = @p1",
        meta.detail_table, meta.detail_foreign_key
    );
    let _ = conn
        .execute(&sql, &[&master_id])
        .await
        .map_err(|e| format!("删除明细失败: {}", e))?;
    Ok(())
}

async fn insert_detail(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
    d: &serde_json::Value,
    row_index: i32,
) -> Result<(), String> {
    let obj = d.as_object().ok_or("明细行必须是 JSON 对象")?;
    let mut cols: Vec<String> = Vec::new();
    let mut params: Vec<Box<dyn ToSql + Send + Sync>> = Vec::new();
    let mut placeholders: Vec<String> = Vec::new();
    let mut idx = 1;

    // 外键
    cols.push(format!("[{}]", meta.detail_foreign_key));
    placeholders.push(format!("@p{}", idx));
    params.push(Box::new(master_id.to_string()));
    idx += 1;

    // 明细主键（NEWID()）
    if !meta.detail_primary_key.is_empty() {
        cols.push(format!("[{}]", meta.detail_primary_key));
        placeholders.push("NEWID()".to_string());
    }

    // 行号（基于循环索引，保证同一次保存内行号有序且不重复）
    // ★ 财务明细表（tFin_ReceiptDtl/tFin_PaymentDtl）的 Rowno 是 IDENTITY(1,1) 自增列，
    //   不能显式插入（否则报 "Cannot insert explicit value for identity column"）。
    //   其他明细表的 RowNO 是普通 int 列，需要前端传入。
    let is_finance_detail = matches!(
        meta.detail_table.as_str(),
        "tFin_ReceiptDtl" | "tFin_PaymentDtl"
    );
    if !is_finance_detail {
        cols.push("[RowNO]".to_string());
        placeholders.push(format!("@p{}", idx));
        params.push(Box::new(row_index + 1));
        idx += 1;
    }

    for (k, v) in obj {
        if k == &meta.detail_primary_key || k == &meta.detail_foreign_key || k == "RowNO" {
            continue;
        }
        // 过滤前端内部字段（以 _ 开头，如 _isNew/_rowKey）
        if k.starts_with('_') {
            continue;
        }
        if k.ends_with("Name") && !k.starts_with("GDS") {
            continue;
        }
        // 过滤前端展示用字段（非明细表列）
        if k == "StkQty" || k == "StockQty" || k == "QQty" {
            continue;
        }
        cols.push(format!("[{}]", k));
        placeholders.push(format!("@p{}", idx));
        params.push(json_to_sql_for_field(k, v));
        idx += 1;
    }
    let sql = format!(
        "INSERT INTO [{}] ({}) VALUES ({})",
        meta.detail_table,
        cols.join(", "),
        placeholders.join(", ")
    );
    let p_refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref() as &dyn ToSql).collect();
    conn.execute(&sql, &p_refs)
        .await
        .map_err(|e| format!("插入明细失败: {}", e))?;
    Ok(())
}

async fn post_save_fill_snapshot(conn: &mut Conn, meta: &DocMeta, master_id: &str) {
    match meta.table.as_str() {
        "tStk_IO" => fill_io_detail_stock_snapshot(conn, master_id).await,
        "tStk_Move" => fill_move_detail_stock_snapshot(conn, master_id).await,
        "tStk_Tran" => fill_tran_detail_stock_snapshot(conn, master_id).await,
        _ => {}
    }
}

// ============== 审核 ==============

pub async fn approve_doc(
    conn: &mut Conn,
    user_code: &str,
    user_name: &str,
    params: ApproveDocParams,
) -> Result<String, ApproveError> {
    approve_doc_internal(conn, user_code, user_name, params, None).await
}

/// 审核核心逻辑（可被 save_doc 的 auto_approve 复用）
/// before_snapshot_override: 外部传入的修改前快照（保存并审核时用保存前的旧数据）
async fn approve_doc_internal(
    conn: &mut Conn,
    user_code: &str,
    _user_name: &str,
    params: ApproveDocParams,
    before_snapshot_override: Option<serde_json::Value>,
) -> Result<String, ApproveError> {
    let meta = doc_graph::get_doc_meta(&params.table)
        .ok_or_else(|| ApproveError::msg(format!("未知业务单据表: {}", params.table)))?
        .clone();
    // doc_type 默认值：基于 doc_no_prefix 映射（与前端 DOC_TYPE_MAP 一致），
    // 确保 post_stock_on_approve/reverse_stock_on_unapprove 分支能正确匹配
    let default_doc_type = default_doc_type_for_table(&meta);
    let doc_type = params
        .doc_type
        .clone()
        .unwrap_or_else(|| default_doc_type.clone());
    tracing::info!(
        "approve_doc start: table={} id={} doc_type={} (default={})",
        params.table,
        params.id,
        doc_type,
        default_doc_type
    );

    // 1) 校验状态
    let cur_state = query_doc_state(conn, &params.table, &params.primary_key, &params.id).await;
    tracing::info!("approve_doc cur_state={}", cur_state);
    // 空状态表示单据不存在/查询失败/State 列为 NULL，不允许审核
    if cur_state.is_empty() {
        return Err(ApproveError::msg("单据不存在或状态字段为空，无法审核"));
    }
    if !matches!(cur_state.as_str(), STATE_NEW | STATE_EDIT) {
        return Err(ApproveError::msg(format!(
            "单据状态 {} 不允许审核（仅 N/E 状态可审核）",
            cur_state
        )));
    }

    // 修改前查旧数据快照（用于操作日志变更明细）
    // 优先使用外部传入的 override（保存并审核场景：用保存前的旧数据作为 before）
    let before_snapshot: Option<serde_json::Value> =
        if let Some(override_val) = before_snapshot_override {
            Some(override_val)
        } else {
            query_doc_snapshot(
                conn,
                &params.table,
                &params.primary_key,
                &params.id,
                &meta.detail_table,
                &meta.detail_foreign_key,
                &meta.detail_primary_key,
            )
            .await
        };

    // 1.5) 会计期间检查（仅影响库存的单据）
    // 门店销售单不进库存流水，跳过期间检查、库存校验和库存过账
    let is_retail_sale =
        is_retail_sale_inv_by_db(conn, &meta.table, &meta.primary_key, &params.id).await;
    if meta.affects_stock && !is_retail_sale {
        if let Some(action_date) = query_doc_date(
            conn,
            &params.table,
            &meta.date_field,
            &params.primary_key,
            &params.id,
        )
        .await
        {
            if let Some(err) = check_period_closed(conn, action_date).await {
                return Err(ApproveError::msg(err.replace("反审核", "审核")));
            }
        }
    }

    // 2) 业务校验（库存不足返回 ApproveError::Shortage 结构化数据）
    if meta.affects_stock && !is_retail_sale {
        validate_outbound_stock_for_approve(conn, &meta, &params.id, &doc_type, user_code).await?;
    }

    // 修复 P1-1：审核时对 receipt/payment 二次校验核销金额
    // 保存与审核之间可能有其他已审核单据核销了同一源单，导致本次审核时 OpenAmt 已不足
    // 保存时的校验是第一次防线，审核时再校验一次才能保证数据一致性
    if matches!(meta.table.as_str(), "tFin_Receipt" | "tFin_Payment") {
        // 复用 query_doc_snapshot 一次取主表 + 明细行，避免重复实现
        let snapshot = query_doc_snapshot(
            conn,
            &meta.table,
            &meta.primary_key,
            &params.id,
            &meta.detail_table,
            &meta.detail_foreign_key,
            &meta.detail_primary_key,
        )
        .await;
        if let Some(snap) = snapshot {
            let master_data = &snap;
            let details: Vec<serde_json::Value> = snap
                .get("Items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            validate_writeoff_details(conn, &meta, master_data, &details, true, &params.id).await?;
        }
    }

    // 外层事务：保证库存过账 + 状态更新 + 操作日志原子化
    // 内部 post_stock_on_approve 的 begin/commit_tran 在嵌套场景下只增减 @@TRANCOUNT，
    // 真正提交由本外层 commit_tran 完成，避免"库存已扣但状态未更新"
    // 门店销售单不进库存流水，无需外层事务保护库存过账
    let need_outer_tran = meta.affects_stock && !is_retail_sale;
    if need_outer_tran {
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(ApproveError::msg(e));
        }
    }
    let mut tx_err: Option<String> = None;

    // 3) 库存过账（门店销售单跳过）
    if !is_retail_sale {
        if let Err(e) = post_stock_on_approve(conn, &meta, &params.id, &doc_type, user_code).await {
            tx_err = Some(e);
        }
    }

    // 4) 更新状态（带 CAS 前置条件：仅 N/E 状态可审核）
    if tx_err.is_none() {
        // 修复 M-4：把工号解析为 EmpID 后再写入 AUser
        let auser = resolve_auser_id(conn, user_code).await;
        let cas_ok = inventory_ledger::update_doc_state_with_cas(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            STATE_REVIEWED,
            &auser,
            Some(&[STATE_NEW, STATE_EDIT]),
        )
        .await;
        if !cas_ok {
            // 状态已被其他请求改掉（非 N/E），回滚事务
            tx_err = Some("单据状态已变更（非 N/E），审核失败".to_string());
        }
    }

    // 5) 写操作日志（含数据快照）
    // after 快照重新查询 DB：保证 before/after 都是完整行（含 Items），便于变更明细展示
    if tx_err.is_none() {
        let after_snapshot: Option<serde_json::Value> = query_doc_snapshot(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            &meta.detail_table,
            &meta.detail_foreign_key,
            &meta.detail_primary_key,
        )
        .await;
        let before_json = before_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let after_json = after_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        // 从 after_snapshot 提取单据号，用于日志 Remark 显示
        let doc_no_str = after_snapshot
            .as_ref()
            .and_then(|v| v.get(meta.no_field.as_str()))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let _ = record_oper_with_data(
            conn,
            "APPROVE",
            &params.table,
            &params.id,
            user_code,
            if doc_no_str.is_empty() {
                None
            } else {
                Some(doc_no_str)
            },
            Some(&format!("审核{}", meta.title)),
            before_json.as_deref(),
            after_json.as_deref(),
        )
        .await;
    }

    if let Some(e) = tx_err {
        if need_outer_tran {
            inventory_ledger::rollback_tran(conn).await;
        }
        return Err(ApproveError::msg(e));
    }
    if need_outer_tran {
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(ApproveError::msg(e));
        }
    }

    Ok("审核成功".to_string())
}

async fn validate_outbound_stock_for_approve(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
    doc_type: &str,
    user_code: &str,
) -> Result<(), ApproveError> {
    // 调拨单：校验调出仓库存（双边过账前预校验，避免中途失败）
    if doc_type == "stock_move" {
        return validate_move_outbound_stock(conn, master_id, user_code).await;
    }

    // 盘点单 / 周期盘点：明细表无 Qty 列（用 DiffQty/RealQty），且 post_stock_tran/post_stock_cycle
    // 已内置库存校验（负差异时检查库存充足），此处跳过通用预校验避免误报"单据无明细行"
    if meta.table == "tStk_Tran" || meta.table == "tStk_StockCycle" {
        return Ok(());
    }

    // 取明细
    let detail_sql = format!(
        "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID, \
                              ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q \
                              FROM [{}] WHERE [{}] = @p1",
        meta.detail_table, meta.detail_foreign_key
    );
    let rows: Vec<(String, String, f64)> = match conn.query(&detail_sql, &[&master_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .map(|r| {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let stkid = r.get::<&str, _>("StkID").unwrap_or("").to_string();
                    let q_str = r.get::<&str, _>("Q").unwrap_or("0");
                    let q: f64 = q_str.parse().unwrap_or(0.0);
                    (gdsid, stkid, q)
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Err(ApproveError::msg("单据无明细行"));
    }

    // 明细行 StkID 为空或全零 UUID 时回退到主表的仓库字段（tStk_IO 的 StkID 在主表上）
    let is_invalid_stkid = |s: &str| s.is_empty() || s == ZERO_UUID;
    let rows: Vec<(String, String, f64)> = if rows.iter().any(|(_, s, _)| is_invalid_stkid(s)) {
        let master_stkid = read_master_stkid(conn, meta, master_id).await;
        if is_invalid_stkid(&master_stkid) {
            return Err(ApproveError::msg(
                "无法确定仓库：明细行和主表的 StkID 均为空",
            ));
        }
        rows.into_iter()
            .map(|(g, s, q)| {
                let final_s = if is_invalid_stkid(&s) {
                    master_stkid.clone()
                } else {
                    s
                };
                (g, final_s, q)
            })
            .collect()
    } else {
        rows
    };

    // 判断是否需要校验出库库存
    // - sales_order: 校验可用量（库存 - 预占），因为审核时会增加预占
    // - sales_outbound / sales_inv: 校验可用量（库存 - 预占），因为出库会消耗库存
    // - purchase_return: 校验库存（采购退货消耗库存，无预占）
    // - stock_io 出库类 Kind（SD/SI/POS/PR/OTO/RI/ADJ）: 校验库存
    // - stock_io 调拨类 Kind（DB/ZP/TH/OT）: 由 stock_move 分支处理或按行符号处理，此处跳过
    // - stock_io 入库类 Kind（PD/SR/OTI/DBI）: 不校验
    let need_check =
        if doc_type == "sales_order" || doc_type == "sales_outbound" || doc_type == "sales_inv" {
            // 销售类：校验可用量
            true
        } else if doc_type == "purchase_return" {
            // 采购退货：校验库存
            true
        } else if doc_type == "stock_io" {
            // 入出库单：按 Kind 判断方向
            let kind = read_kind(conn, meta, master_id).await;
            let dir = doc_graph::kind_direction(&kind);
            // 出库类需要校验；调拨类（DB/ZP/TH/OT）按行 Qty 符号决定，负数行需校验
            if dir == doc_graph::DIR_OUTBOUND {
                true
            } else if dir == doc_graph::DIR_TRANSFER {
                // 调拨类：按行符号判断，有负数行（出库）就需校验
                rows.iter().any(|(_, _, q)| *q < 0.0)
            } else {
                false
            }
        } else {
            false
        };

    if !need_check {
        return Ok(());
    }

    let mut shortage: Vec<StockShortageItem> = Vec::new();
    // 销售出库 / 销售出库单：审核流程是先释放预占（QQty -= qty）再出库（Qty -= qty），
    // 净效果 (Qty - QQty) 不变，所以校验 stock >= qty 即可；
    // 但仅在"有源销售订单"时才会释放预占（无源单的直出库不动 QQty，需校验 stock - qqty >= qty）。
    // 其他出库类（采购退货等）：QQty 不变，需校验 stock - qqty >= qty（数据库 CHECK Qty >= QQty）。
    let source_soid = if doc_type == "sales_outbound" || doc_type == "sales_inv" {
        query_source_soid(conn, &meta.table, &meta.primary_key, master_id).await
    } else {
        String::new()
    };
    let is_sales_outbound_with_reserve =
        (doc_type == "sales_outbound" || doc_type == "sales_inv") && !source_soid.is_empty();
    for (idx, (gdsid, stkid, qty)) in rows.iter().enumerate() {
        if gdsid.is_empty() || stkid.is_empty() {
            continue;
        }
        let abs_qty = qty.abs();
        if abs_qty <= 0.0 {
            continue;
        }

        let stock = query_stock_qty(conn, gdsid, stkid).await;
        let qqty = query_qqty(conn, gdsid, stkid).await;
        let available = stock - qqty;

        // 有源单的销售出库：校验 stock >= qty（预占会被先释放）
        // 其他情况：校验 available >= qty（数据库 CHECK 约束 Qty >= QQty）
        let check_ok = if is_sales_outbound_with_reserve {
            stock >= abs_qty - 0.0001
        } else {
            available >= abs_qty - 0.0001
        };
        if !check_ok {
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
            shortage.push(StockShortageItem {
                row_no: idx + 1,
                gds_id: gdsid.clone(),
                stk_id: stkid.clone(),
                gds_no: gds_no.clone(),
                gds_name: gds_name.clone(),
                stk_no: stk_no.clone(),
                stk_name: stk_name.clone(),
                stock,
                reserved: qqty,
                available,
                qty: abs_qty,
                shortage: (abs_qty
                    - if is_sales_outbound_with_reserve {
                        stock
                    } else {
                        available
                    })
                .ceil(),
            });
        }
    }
    if !shortage.is_empty() {
        // 持久化缺货记录到 tStk_Shortage（approve 场景：单据号从 DB 查询）
        let doc_no = if !meta.no_field.is_empty() {
            let sql = format!(
                "SELECT CAST([{}] AS NVARCHAR(50)) AS NO FROM [{}] WHERE [{}] = @p1",
                meta.no_field, meta.table, meta.primary_key
            );
            match conn.query(&sql, &[&master_id]).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(row)) => row.get::<&str, _>("NO").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            }
        } else {
            String::new()
        };
        let emp_id = query_emp_id_by_code(conn, user_code).await;
        // 读取客户/门店（审核路径：从 DB 查询单据主表的 CustID）
        let (cust_id, cust_name) = query_cust_info_from_db(conn, meta, master_id).await;
        // 销售类单据无独立门店字段，门店列为空（门店信息在调拨单 ZP 场景才填充）
        log_shortage_to_db(
            conn,
            &shortage,
            &meta.table,
            &doc_no,
            master_id,
            user_code,
            &emp_id,
            "doc_approve",
            &cust_id,
            &cust_name,
            "",
            "",
        )
        .await;
        return Err(ApproveError::Shortage(shortage));
    }
    Ok(())
}

/// 调拨单出库预校验：校验调出仓库存是否足够（不允许负库存）
async fn validate_move_outbound_stock(
    conn: &mut Conn,
    move_id: &str,
    user_code: &str,
) -> Result<(), ApproveError> {
    // 读取调拨单 Kind：DB=内部调拨, TH=门店退仓, ZP=门店直配
    // - DB（内部调拨）：仓库间移库，不算缺货
    // - TH（门店退仓）：门店退到总仓，总仓是收货方（入库方向），不算缺货
    // - ZP（门店直配）：总仓发货给门店，总仓是发货方（出库方向），需校验缺货
    let kind_sql = "SELECT ISNULL(Kind, '') AS K FROM tStk_Move WHERE MoveID = @p1";
    let kind: String = match conn.query(kind_sql, &[&move_id]).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(row)) => row.get::<&str, _>("K").unwrap_or("").to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    if kind != "ZP" {
        tracing::info!(
            kind = %kind,
            move_id = %move_id,
            "[validate_move_outbound_stock] 跳过（仅 ZP 门店直配需校验缺货）"
        );
        return Ok(());
    }

    let (from_id, to_id) = query_move_stk(conn, move_id).await;
    if from_id.is_empty() || to_id.is_empty() {
        return Err(ApproveError::msg(
            "调拨单仓库信息不完整（调出仓/调入仓未设置）",
        ));
    }
    let detail_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                      ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q \
                      FROM tStk_MoveDetail WHERE MoveID = @p1";
    let rows: Vec<(String, f64)> = match conn.query(detail_sql, &[&move_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .map(|r| {
                    (
                        r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                        r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Err(ApproveError::msg("调拨单无明细行"));
    }
    let (from_no, from_name) = query_stk_info(conn, &from_id).await;
    let mut shortage: Vec<StockShortageItem> = Vec::new();
    for (idx, (gdsid, qty)) in rows.iter().enumerate() {
        if gdsid.is_empty() || *qty <= 0.0 {
            continue;
        }
        let stock = query_stock_qty(conn, gdsid, &from_id).await;
        let qqty = query_qqty(conn, gdsid, &from_id).await;
        let available = stock - qqty;
        // 调拨校验可用量（库存 - 预占）：CHECK 约束 Qty >= QQty 要求扣减后不能低于 QQty
        if available < *qty - 0.0001 {
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            shortage.push(StockShortageItem {
                row_no: idx + 1,
                gds_id: gdsid.clone(),
                stk_id: from_id.clone(),
                gds_no: gds_no.clone(),
                gds_name: gds_name.clone(),
                stk_no: from_no.clone(),
                stk_name: from_name.clone(),
                stock,
                reserved: qqty,
                available,
                qty: *qty,
                shortage: (*qty - available).ceil(),
            });
        }
    }
    if !shortage.is_empty() {
        // 持久化缺货记录到 tStk_Shortage（调拨场景：单据号从 DB 查询）
        let doc_no_sql =
            "SELECT CAST(MoveNo AS NVARCHAR(50)) AS NO FROM tStk_Move WHERE MoveID = @p1";
        let doc_no = match conn.query(doc_no_sql, &[&move_id]).await {
            Ok(s) => match s.into_row().await {
                Ok(Some(row)) => row.get::<&str, _>("NO").unwrap_or("").to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let emp_id = query_emp_id_by_code(conn, user_code).await;
        // 门店直配（ZP）：总仓发货给门店
        // - 客户列为空（调拨单无客户字段）
        // - 门店列 = ToStkID（调入仓，即门店仓库）
        let (_, shop_name) = query_stk_info(conn, &to_id).await;
        log_shortage_to_db(
            conn,
            &shortage,
            "tStk_Move:ZP",
            &doc_no,
            move_id,
            user_code,
            &emp_id,
            "doc_approve",
            "",
            "",
            &to_id,
            &shop_name,
        )
        .await;
        return Err(ApproveError::Shortage(shortage));
    }
    Ok(())
}

async fn post_stock_on_approve(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
    doc_type: &str,
    user_code: &str,
) -> Result<(), String> {
    // 取主表 Kind
    let kind = read_kind(conn, meta, master_id).await;
    tracing::info!(
        "post_stock_on_approve: table={} doc_type={} kind={} affects_stock={} master_id={}",
        meta.table,
        doc_type,
        kind,
        meta.affects_stock,
        master_id
    );
    tracing::info!(table = %meta.table, doc_type = %doc_type, kind = %kind, affects_stock = meta.affects_stock, master_id = %master_id, "[post_stock_on_approve] 入口");

    // 销售订单：QQty 预占 + 写 tStk_Reserve 预占记录
    if doc_type == "sales_order" {
        let detail_sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                                  CAST([{}] AS NVARCHAR(40)) AS DID \
                                  FROM [{}] WHERE [{}] = @p1",
            meta.detail_primary_key, meta.detail_table, meta.detail_foreign_key
        );
        let rows: Vec<(String, String, f64, String)> =
            match conn.query(&detail_sql, &[&master_id]).await {
                Ok(s) => match s.into_first_result().await {
                    Ok(rs) => rs
                        .iter()
                        .map(|r| {
                            (
                                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                                r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                                r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                                r.get::<&str, _>("DID").unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
        let doc_no = query_doc_no(conn, meta, master_id).await;
        let user_uuid = {
            let emp_id = query_emp_id_by_code(conn, user_code).await;
            if !emp_id.is_empty() {
                emp_id
            } else {
                ZERO_UUID.to_string()
            }
        };
        for (gdsid, stkid, qty, did) in &rows {
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            let stk_txt = fmt_stk(stkid, &stk_no, &stk_name);
            // 销售订单审核：增加预占（QQty += qty）
            if !apply_qqty_delta(conn, gdsid, stkid, *qty).await {
                return Err(format!(
                    "预占失败：商品[{}] 仓库[{}] 预占量不足（不允许负库存）",
                    gds_txt, stk_txt
                ));
            }
            // 写预占记录（DocID=SOID，便于出库时按源单释放）
            // ReserveID 是 uniqueidentifier 类型，必须用标准 UUID 格式（带横线，无前缀）
            let rid = uuid::Uuid::new_v4().to_string();
            if !insert_reserve(
                conn,
                &rid,
                "sales_order",
                master_id,
                &doc_no,
                did,
                gdsid,
                stkid,
                *qty,
                &user_uuid,
            )
            .await
            {
                return Err(format!(
                    "写入预占记录失败：商品[{}] 仓库[{}]",
                    gds_txt, stk_txt
                ));
            }
        }
        return Ok(());
    }

    // 采购入库/退货/销售出库/销售退货/库存入出库：单边过账（带事务保护）
    if matches!(
        doc_type,
        "purchase_inbound"
            | "purchase_receipt"
            | "purchase_return"
            | "sales_outbound"
            | "sales_inv"
            | "sales_return"
            | "stock_io"
    ) {
        // stock_io 的 OT/ZP 类按行 Qty 符号决定方向；其他类型用固定方向
        let use_kind_direction = doc_type == "stock_io";
        let fixed_direction: f64 = match doc_type {
            "purchase_inbound" | "purchase_receipt" | "sales_return" => 1.0,
            "purchase_return" | "sales_outbound" | "sales_inv" => -1.0,
            _ => 0.0, // stock_io 下面单独处理
        };
        // 对 stock_io 但非 OT/ZP 的情况，用 kind_direction
        let kind_dir = if use_kind_direction {
            doc_graph::kind_direction(&kind)
        } else {
            0.0
        };
        // OT/ZP 时 kind_dir=0，需要按行 Qty 符号决定；其他 stock_io 用 kind_dir
        let per_row_sign = use_kind_direction && kind_dir == 0.0;

        // 取单据日期对应的会计月份
        let doc_ym = query_doc_month(conn, meta, master_id).await;

        let detail_sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                                  CAST([{}] AS NVARCHAR(40)) AS DID \
                                  FROM [{}] WHERE [{}] = @p1",
            meta.detail_primary_key, meta.detail_table, meta.detail_foreign_key
        );
        let rows: Vec<(String, String, f64, String)> =
            match conn.query(&detail_sql, &[&master_id]).await {
                Ok(s) => match s.into_first_result().await {
                    Ok(rs) => rs
                        .iter()
                        .map(|r| {
                            (
                                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                                r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                                r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                                r.get::<&str, _>("DID").unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
        if rows.is_empty() {
            return Err("单据无明细行".to_string());
        }
        // 明细行 StkID 为空或全零 UUID 时回退到主表的仓库字段（tStk_IO 的 StkID 在主表上）
        let is_invalid_stkid = |s: &str| s.is_empty() || s == ZERO_UUID;
        let master_stkid = if rows.iter().any(|(_, s, _, _)| is_invalid_stkid(s)) {
            let ms = read_master_stkid(conn, meta, master_id).await;
            if is_invalid_stkid(&ms) {
                return Err("无法确定仓库：明细行和主表的 StkID 均为空或全零".to_string());
            }
            ms
        } else {
            String::new()
        };
        let rows: Vec<(String, String, f64, String)> = if !master_stkid.is_empty() {
            rows.into_iter()
                .map(|(g, s, q, d)| {
                    let final_s = if is_invalid_stkid(&s) {
                        master_stkid.clone()
                    } else {
                        s
                    };
                    (g, final_s, q, d)
                })
                .collect()
        } else {
            rows
        };
        tracing::info!(
            "post_stock_on_approve single-side: master_id={} rows={} fixed_dir={} kind_dir={} per_row_sign={} ym={}",
            master_id,
            rows.len(),
            fixed_direction,
            kind_dir,
            per_row_sign,
            doc_ym
        );

        // 销售出库：查源销售订单 SOID（用于释放 tStk_Reserve 预占）
        let source_soid = if doc_type == "sales_outbound" || doc_type == "sales_inv" {
            query_source_soid(conn, &meta.table, &meta.primary_key, master_id).await
        } else {
            String::new()
        };

        // 开启事务，保证中途失败回滚
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let mut tx_failed: Option<String> = None;
        for (gdsid, stkid, qty, did) in &rows {
            if gdsid.is_empty() || stkid.is_empty() || *qty == 0.0 {
                continue;
            }
            // 决定本行方向
            let row_dir = if per_row_sign {
                if *qty > 0.0 { 1.0 } else { -1.0 }
            } else if use_kind_direction {
                kind_dir
            } else {
                fixed_direction
            };
            if row_dir == 0.0 {
                tx_failed = Some(format!("无法确定库存方向: Kind={}", kind));
                break;
            }
            let abs_qty = qty.abs();
            // 预先查询商品/仓库名称用于错误提示
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            let stk_txt = fmt_stk(stkid, &stk_no, &stk_name);
            // 销售出库：先释放预占（QQty -= qty），再出库（Qty -= qty）
            // 注意：post_ledger 现在只动 Qty 不动 QQty，所以出库不会影响 QQty
            // 销售出库时，订单审核已扣 QQty（预占），出库后预占转为实际出库，需释放预占（QQty 减少）
            // 仅在有源销售订单时才释放预占（无源单的直出库未预占，不应释放 QQty 避免变负）
            if (doc_type == "sales_outbound" || doc_type == "sales_inv")
                && row_dir < 0.0
                && !source_soid.is_empty()
            {
                if !apply_qqty_delta(conn, gdsid, stkid, -abs_qty).await {
                    tx_failed = Some(format!(
                        "释放预占失败：商品[{}] 仓库[{}]（预占量不足）",
                        gds_txt, stk_txt
                    ));
                    break;
                }
                // 同步释放 tStk_Reserve 预占记录（按源单 SOID 匹配）
                if !release_reserve_by_doc(conn, "sales_order", &source_soid, gdsid, stkid, abs_qty)
                    .await
                {
                    tx_failed = Some(format!(
                        "释放预占记录失败：商品[{}] 仓库[{}]",
                        gds_txt, stk_txt
                    ));
                    break;
                }
            }
            let (cur, ok) = post_ledger_with_period(
                conn, gdsid, stkid, abs_qty, row_dir, master_id, did, doc_ym,
            )
            .await;
            tracing::debug!(gdsid = %gdsid, stkid = %stkid, abs_qty, row_dir, cur, ok, "[post_stock single-side] 过账");
            if !ok {
                tx_failed = Some(format!(
                    "库存不足，不允许负库存：商品[{}] 仓库[{}] 现有{} 需求{}（不足 {}）",
                    gds_txt,
                    stk_txt,
                    cur,
                    abs_qty,
                    (abs_qty - cur).ceil()
                ));
                break;
            }
            // 回填快照
            if meta.table.as_str() == "tStk_IO" {
                fill_detail_stock_snapshot(conn, "tStk_IODetail", "IODetailID", did).await;
            }
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 调拨：双边过账
    if doc_type == "stock_move" {
        let doc_ym = query_doc_month(conn, meta, master_id).await;
        return post_stock_move(conn, master_id, doc_ym).await;
    }

    // 盘点：按 DiffQty
    if doc_type == "stock_take" || doc_type == "stock_check" || doc_type == "stocktake" {
        let doc_ym = query_doc_month(conn, meta, master_id).await;
        return post_stock_tran(conn, master_id, doc_ym).await;
    }

    // 周期盘点：按 RealQty - StkQty 的差异过账
    if doc_type == "stock_cycle" {
        let doc_ym = query_doc_month(conn, meta, master_id).await;
        return post_stock_cycle(conn, master_id, doc_ym).await;
    }

    Ok(())
}

async fn read_kind(conn: &mut Conn, meta: &DocMeta, master_id: &str) -> String {
    if meta.kind_field.is_empty() {
        return String::new();
    }
    let sql = format!(
        "SELECT ISNULL(CAST([{}] AS NVARCHAR(40)), '') AS K FROM [{}] WHERE [{}] = @p1",
        meta.kind_field, meta.table, meta.primary_key
    );
    if let Ok(stream) = conn.query(&sql, &[&master_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("K").unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 读取主表的仓库 ID（StkID）。
/// tStk_IO 等单据的仓库字段在主表上，明细表没有 StkID 或为空时需要回退到主表。
async fn read_master_stkid(conn: &mut Conn, meta: &DocMeta, master_id: &str) -> String {
    if meta.warehouse_field.is_empty() {
        return String::new();
    }
    let sql = format!(
        "SELECT ISNULL(CAST([{}] AS NVARCHAR(40)), '') AS W FROM [{}] WHERE [{}] = @p1",
        meta.warehouse_field, meta.table, meta.primary_key
    );
    if let Ok(stream) = conn.query(&sql, &[&master_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("W").unwrap_or("").to_string();
        }
    }
    String::new()
}

async fn post_stock_move(conn: &mut Conn, move_id: &str, ym: i32) -> Result<(), String> {
    // 调出仓、调入仓
    let (from_id, to_id) = query_move_stk(conn, move_id).await;
    if from_id.is_empty() || to_id.is_empty() {
        return Err("调拨单仓库信息不完整".to_string());
    }
    let detail_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                      ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                      CAST(MoveDetailID AS NVARCHAR(40)) AS DID \
                      FROM tStk_MoveDetail WHERE MoveID = @p1";
    let rows: Vec<(String, f64, String)> = match conn.query(detail_sql, &[&move_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .map(|r| {
                    (
                        r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                        r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                        r.get::<&str, _>("DID").unwrap_or("").to_string(),
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Err("调拨单无明细行".to_string());
    }
    // 调拨事务：复用 inventory_ledger 的事务辅助（统一处理 SQL Server 266 错误）
    if let Err(e) = inventory_ledger::begin_tran(conn).await {
        return Err(e);
    }
    let (from_no, from_name) = query_stk_info(conn, &from_id).await;
    let (to_no, to_name) = query_stk_info(conn, &to_id).await;
    let from_txt = fmt_stk(&from_id, &from_no, &from_name);
    let to_txt = fmt_stk(&to_id, &to_no, &to_name);
    let mut tx_failed: Option<String> = None;
    for (gdsid, qty, did) in &rows {
        if gdsid.is_empty() || *qty == 0.0 {
            continue;
        }
        let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
        let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
        // 调出仓 -qty
        let (cur, ok1) =
            post_ledger_with_period(conn, gdsid, &from_id, *qty, -1.0, move_id, did, ym).await;
        if !ok1 {
            tx_failed = Some(format!(
                "调出仓库存不足，不允许负库存：商品[{}] 调出仓[{}] 现有{} 需求{}（不足 {}）",
                gds_txt,
                from_txt,
                cur,
                qty,
                (qty - cur).ceil()
            ));
            break;
        }
        // 调入仓 +qty
        let (_, ok2) =
            post_ledger_with_period(conn, gdsid, &to_id, *qty, 1.0, move_id, did, ym).await;
        if !ok2 {
            tx_failed = Some(format!("调入仓[{}]写入失败：商品[{}]", to_txt, gds_txt));
            break;
        }
        fill_detail_stock_snapshot(conn, "tStk_MoveDetail", "MoveDetailID", did).await;
    }
    if let Some(err) = tx_failed {
        inventory_ledger::rollback_tran(conn).await;
        return Err(err);
    }
    if let Err(e) = inventory_ledger::commit_tran(conn).await {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e);
    }
    Ok(())
}

async fn query_move_stk(conn: &mut Conn, move_id: &str) -> (String, String) {
    let sql = "SELECT ISNULL(CAST(FromStkID AS NVARCHAR(40)),'') AS F, ISNULL(CAST(ToStkID AS NVARCHAR(40)),'') AS T \
               FROM tStk_Move WHERE MoveID = @p1";
    let params: Vec<&dyn ToSql> = vec![&move_id];
    match conn.query(sql, &params).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(r)) => (
                r.get::<&str, _>("F").unwrap_or("").to_string(),
                r.get::<&str, _>("T").unwrap_or("").to_string(),
            ),
            _ => (String::new(), String::new()),
        },
        _ => (String::new(), String::new()),
    }
}

async fn post_stock_tran(conn: &mut Conn, tran_id: &str, ym: i32) -> Result<(), String> {
    let stk_id = query_tran_stk(conn, tran_id).await;
    if stk_id.is_empty() {
        return Err("盘点单仓库信息缺失".to_string());
    }
    let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                   ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                   CAST(TranDetailID AS NVARCHAR(40)) AS DID \
                   FROM tStk_TranDetail WHERE TranID = @p1";
    let rows: Vec<(String, f64, String)> = match conn.query(det_sql, &[&tran_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .map(|r| {
                    (
                        r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                        r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0),
                        r.get::<&str, _>("DID").unwrap_or("").to_string(),
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Err("盘点单无明细行".to_string());
    }
    // 开启事务
    if let Err(e) = inventory_ledger::begin_tran(conn).await {
        return Err(e);
    }
    let (stk_no, stk_name) = query_stk_info(conn, &stk_id).await;
    let stk_txt = fmt_stk(&stk_id, &stk_no, &stk_name);
    let mut tx_failed: Option<String> = None;
    for (gdsid, dq, did) in &rows {
        if gdsid.is_empty() || *dq == 0.0 {
            continue;
        }
        let abs_qty = dq.abs();
        let dir = if *dq > 0.0 { 1.0 } else { -1.0 };
        // 负差异（盘亏）需校验扣减后 Qty >= QQty（数据库 CHECK 约束）
        if dir < 0.0 {
            let stock = query_stock_qty(conn, gdsid, &stk_id).await;
            let qqty = query_qqty(conn, gdsid, &stk_id).await;
            if stock - abs_qty < qqty - 0.0001 {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
                tx_failed = Some(format!(
                    "盘亏后库存将低于预占量：商品[{}] 仓库[{}] 现有{} 预占{} 盘亏{}",
                    gds_txt, stk_txt, stock, qqty, abs_qty
                ));
                break;
            }
        }
        let (cur, ok) =
            post_ledger_with_period(conn, gdsid, &stk_id, abs_qty, dir, tran_id, did, ym).await;
        if !ok {
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            tx_failed = Some(format!(
                "盘点出库库存不足，不允许负库存：商品[{}] 仓库[{}] 现有{} 需求{}（不足 {}）",
                gds_txt,
                stk_txt,
                cur,
                abs_qty,
                (abs_qty - cur).ceil()
            ));
            break;
        }
        fill_detail_stock_snapshot(conn, "tStk_TranDetail", "TranDetailID", did).await;
    }
    if let Some(err) = tx_failed {
        inventory_ledger::rollback_tran(conn).await;
        return Err(err);
    }
    if let Err(e) = inventory_ledger::commit_tran(conn).await {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e);
    }
    Ok(())
}

/// 周期盘点过账：按 RealQty - StkQty 的差异调整库存
async fn post_stock_cycle(conn: &mut Conn, cycle_id: &str, ym: i32) -> Result<(), String> {
    // 周期盘点主表取仓库
    let stk_sql = "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_StockCycle WHERE CycleID = @p1";
    let stk_id = match conn.query(stk_sql, &[&cycle_id]).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(r)) => r.get::<&str, _>("S").unwrap_or("").to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    if stk_id.is_empty() {
        return Err("周期盘点单仓库信息缺失".to_string());
    }
    // 明细：DiffQty 为差异（前端计算 = RealQty - AccQty），与 post_stock_tran 一致
    let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                   ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                   CAST(CycleDetailID AS NVARCHAR(40)) AS DID \
                   FROM tStk_StockCycleDetail WHERE CycleID = @p1";
    let rows: Vec<(String, f64, String)> = match conn.query(det_sql, &[&cycle_id]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rs) => rs
                .iter()
                .map(|r| {
                    (
                        r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                        r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0),
                        r.get::<&str, _>("DID").unwrap_or("").to_string(),
                    )
                })
                .collect(),
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if rows.is_empty() {
        return Err("周期盘点单无明细行".to_string());
    }
    // 开启事务
    if let Err(e) = inventory_ledger::begin_tran(conn).await {
        return Err(e);
    }
    let (stk_no, stk_name) = query_stk_info(conn, &stk_id).await;
    let stk_txt = fmt_stk(&stk_id, &stk_no, &stk_name);
    let mut tx_failed: Option<String> = None;
    for (gdsid, diff, did) in &rows {
        if gdsid.is_empty() {
            continue;
        }
        if diff.abs() < 0.0001 {
            continue;
        }
        let abs_qty = diff.abs();
        let dir = if *diff > 0.0 { 1.0 } else { -1.0 };
        // 负差异（盘亏）需校验扣减后 Qty >= QQty（数据库 CHECK 约束）
        if dir < 0.0 {
            let stock = query_stock_qty(conn, gdsid, &stk_id).await;
            let qqty = query_qqty(conn, gdsid, &stk_id).await;
            if stock - abs_qty < qqty - 0.0001 {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
                tx_failed = Some(format!(
                    "周期盘点盘亏后库存将低于预占量：商品[{}] 仓库[{}] 现有{} 预占{} 盘亏{}",
                    gds_txt, stk_txt, stock, qqty, abs_qty
                ));
                break;
            }
        }
        let (cur, ok) =
            post_ledger_with_period(conn, gdsid, &stk_id, abs_qty, dir, cycle_id, did, ym).await;
        if !ok {
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            tx_failed = Some(format!(
                "周期盘点出库库存不足，不允许负库存：商品[{}] 仓库[{}] 现有{} 需求{}（不足 {}）",
                gds_txt,
                stk_txt,
                cur,
                abs_qty,
                (abs_qty - cur).ceil()
            ));
            break;
        }
        // 回填明细快照（与 post_stock_tran 保持一致）
        fill_detail_stock_snapshot(conn, "tStk_StockCycleDetail", "CycleDetailID", did).await;
    }
    if let Some(err) = tx_failed {
        inventory_ledger::rollback_tran(conn).await;
        return Err(err);
    }
    if let Err(e) = inventory_ledger::commit_tran(conn).await {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e);
    }
    Ok(())
}

async fn query_tran_stk(conn: &mut Conn, tran_id: &str) -> String {
    let sql =
        "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_Tran WHERE TranID = @p1";
    if let Ok(stream) = conn.query(sql, &[&tran_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("S").unwrap_or("").to_string();
        }
    }
    String::new()
}

// ============== 反审 ==============

pub async fn unapprove_doc(
    conn: &mut Conn,
    user_code: &str,
    _user_name: &str,
    params: ApproveDocParams,
) -> Result<String, String> {
    let meta = doc_graph::get_doc_meta(&params.table)
        .ok_or_else(|| format!("未知业务单据表: {}", params.table))?
        .clone();
    // doc_type 默认值：基于 doc_no_prefix 映射（与前端 DOC_TYPE_MAP 一致），
    // 确保 post_stock_on_approve/reverse_stock_on_unapprove 分支能正确匹配
    let default_doc_type = default_doc_type_for_table(&meta);
    let doc_type = params
        .doc_type
        .clone()
        .unwrap_or_else(|| default_doc_type.clone());
    tracing::info!(
        "unapprove_doc start: table={} id={} doc_type={}",
        params.table,
        params.id,
        doc_type
    );

    let cur_state = query_doc_state(conn, &params.table, &params.primary_key, &params.id).await;
    if cur_state != STATE_REVIEWED {
        return Err(format!(
            "单据状态 {} 不允许反审（仅 S 状态可反审）",
            cur_state
        ));
    }

    // 修改前查旧数据快照（用于操作日志变更明细）
    let before_snapshot: Option<serde_json::Value> = query_doc_snapshot(
        conn,
        &params.table,
        &params.primary_key,
        &params.id,
        &meta.detail_table,
        &meta.detail_foreign_key,
        &meta.detail_primary_key,
    )
    .await;

    // 会计期间检查（仅影响库存的单据）
    // 门店销售单不进库存流水，跳过期间检查和反向过账
    let is_retail_sale =
        is_retail_sale_inv_by_db(conn, &meta.table, &meta.primary_key, &params.id).await;
    if meta.affects_stock && !is_retail_sale {
        if let Some(action_date) = query_doc_date(
            conn,
            &params.table,
            &meta.date_field,
            &params.primary_key,
            &params.id,
        )
        .await
        {
            if let Some(err) = check_period_closed(conn, action_date).await {
                return Err(err);
            }
        }
    }

    // 下游引用检查：如果已被下游单据引用，则不允许反审
    if let Err(e) = check_downstream_references(conn, &meta, &params.id).await {
        return Err(e);
    }

    // 外层事务：保证反向过账 + 状态更新 + 操作日志原子化
    // reverse_stock_on_unapprove 内部的事务在嵌套场景下只增减 @@TRANCOUNT，
    // 真正提交由本外层 commit_tran 完成，避免"库存已回滚但状态仍为 S"的不一致
    // 门店销售单不进库存流水，无需外层事务保护反向过账
    let need_outer_tran = meta.affects_stock && !is_retail_sale;
    if need_outer_tran {
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
    }
    let mut tx_err: Option<String> = None;

    // 反向过账（门店销售单跳过）
    if !is_retail_sale {
        if let Err(e) = reverse_stock_on_unapprove(conn, &meta, &params.id, &doc_type).await {
            tx_err = Some(e);
        }
    }

    // 更新状态：反审后回到 N（新建），允许用户编辑后重新审核
    // 带 CAS 前置条件：仅 S 状态可反审
    if tx_err.is_none() {
        // 修复 M-4：把工号解析为 EmpID 后再写入 AUser
        let auser = resolve_auser_id(conn, user_code).await;
        let cas_ok = inventory_ledger::update_doc_state_with_cas(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            STATE_NEW,
            &auser,
            Some(&[STATE_REVIEWED]),
        )
        .await;
        if !cas_ok {
            tx_err = Some("单据状态已变更（非 S），反审失败".to_string());
        }
    }

    // 写操作日志（含数据快照）
    // after 快照重新查询 DB：保证 before/after 都是完整行（含 Items），便于变更明细展示
    if tx_err.is_none() {
        let after_snapshot: Option<serde_json::Value> = query_doc_snapshot(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            &meta.detail_table,
            &meta.detail_foreign_key,
            &meta.detail_primary_key,
        )
        .await;
        let before_json = before_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let after_json = after_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let _ = record_oper_with_data(
            conn,
            "UNAPPROVE",
            &params.table,
            &params.id,
            user_code,
            None,
            Some(&format!("反审{}", meta.title)),
            before_json.as_deref(),
            after_json.as_deref(),
        )
        .await;
    }

    if let Some(e) = tx_err {
        if need_outer_tran {
            inventory_ledger::rollback_tran(conn).await;
        }
        return Err(e);
    }
    if need_outer_tran {
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
    }

    Ok("反审成功".to_string())
}

async fn query_doc_date(
    conn: &mut Conn,
    table: &str,
    date_col: &str,
    primary_key: &str,
    id: &str,
) -> Option<chrono::NaiveDate> {
    if date_col.is_empty() {
        return None;
    }
    let sql = format!(
        "SELECT CAST([{}] AS DATE) AS D FROM [{}] WHERE [{}] = @p1",
        date_col, table, primary_key
    );
    if let Ok(stream) = conn.query(&sql, &[&id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            // 常量日期 unwrap 安全：from_ymd_opt(1970,1,1) 永远返回 Some
            let default_date = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
            let d = row.get::<chrono::NaiveDate, _>("D").unwrap_or(default_date);
            let year_2000 = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
            if d < year_2000 {
                return None;
            }
            return Some(d);
        }
    }
    None
}

/// 查询单据日期对应的会计月份（YYYYMM），失败则回退到当前月
async fn query_doc_month(conn: &mut Conn, meta: &DocMeta, master_id: &str) -> i32 {
    if let Some(d) = query_doc_date(
        conn,
        &meta.table,
        &meta.date_field,
        &meta.primary_key,
        master_id,
    )
    .await
    {
        let ym: i32 = d.format("%Y%m").to_string().parse().unwrap_or(0);
        if ym >= 200001 {
            return ym;
        }
    }
    // 回退到当前月
    chrono::Local::now()
        .format("%Y%m")
        .to_string()
        .parse()
        .unwrap_or(202501)
}

// ============== tStk_Reserve 预占表辅助 ==============

/// 查询单据号
async fn query_doc_no(conn: &mut Conn, meta: &DocMeta, master_id: &str) -> String {
    if meta.no_field.is_empty() {
        return String::new();
    }
    let sql = format!(
        "SELECT ISNULL(CAST([{}] AS NVARCHAR(40)), '') AS N FROM [{}] WHERE [{}] = @p1",
        meta.no_field, meta.table, meta.primary_key
    );
    if let Ok(stream) = conn.query(&sql, &[&master_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("N").unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 写入预占记录（销售订单审核时调用）
async fn insert_reserve(
    conn: &mut Conn,
    reserve_id: &str,
    doc_type: &str,
    doc_id: &str,
    doc_no: &str,
    detail_id: &str,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    user: &str,
) -> bool {
    if qty <= 0.0 || gdsid.is_empty() || stkid.is_empty() {
        return true;
    }
    let sql = "INSERT INTO tStk_Reserve (ReserveID, DocType, DocID, DocNo, DetailID, GDSID, StkID, Qty, ReleasedQty, State, EDate, EUser) \
               VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, 0, 'A', GETDATE(), @p9)";
    let params: Vec<&dyn ToSql> = vec![
        &reserve_id,
        &doc_type,
        &doc_id,
        &doc_no,
        &detail_id,
        &gdsid,
        &stkid,
        &qty,
        &user,
    ];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::error!(
                "[insert_reserve] 写入预占失败: doc_id={} gdsid={} err={}",
                doc_id,
                gdsid,
                e
            );
            false
        }
    }
}

/// 作废预占记录（销售订单反审/作废时调用）
///
/// 修复 M-5：原实现只处理 State='A'，不处理 'X'（已释放但 ReleasedQty 未归零的残留记录），
/// 导致销售订单反审后 tStk_Reserve 中残留 'X' 记录 ReleasedQty > 0，与 tStk_Stock.QQty 不一致。
/// 现统一把 'A'/'X' 都置为 'X' 并把 ReleasedQty 归零，保证预占记录作废后无残留。
async fn void_reserve_by_doc(conn: &mut Conn, doc_type: &str, doc_id: &str) -> bool {
    if doc_id.is_empty() {
        return true;
    }
    let sql = "UPDATE tStk_Reserve SET State = 'X', ReleasedQty = 0 WHERE DocType = @p1 AND DocID = @p2 AND State IN ('A', 'X')";
    let params: Vec<&dyn ToSql> = vec![&doc_type, &doc_id];
    match conn.execute(sql, &params).await {
        Ok(_) => true,
        Err(e) => {
            tracing::error!(
                "[void_reserve_by_doc] 作废预占失败: doc_id={} err={}",
                doc_id,
                e
            );
            false
        }
    }
}

/// 释放预占（销售出库审核时调用）：按源单 SOID 匹配，增加 ReleasedQty
///
/// 修复 H-3：循环处理多条预占记录，直到 ship_qty 耗尽或无可用记录。
/// 原实现只取 TOP 1，当出库量 > 单条预占剩余量时只释放部分，剩余量丢弃，
/// 导致 tStk_Reserve.ReleasedQty 总和与 tStk_Stock.QQty 长期不一致。
async fn release_reserve_by_doc(
    conn: &mut Conn,
    doc_type: &str,
    source_doc_id: &str,
    gdsid: &str,
    stkid: &str,
    ship_qty: f64,
) -> bool {
    if source_doc_id.is_empty() || gdsid.is_empty() || stkid.is_empty() || ship_qty <= 0.0 {
        return true;
    }
    let mut remaining = ship_qty;
    // 循环按 FIFO（EDate ASC）释放，直到 remaining=0 或无可用预占记录
    while remaining > 0.0001 {
        // CAST 为 NVARCHAR 以兼容 tiberius row.get::<&str,_> 读取模式
        let sql = "SELECT TOP 1 CAST(ReserveID AS NVARCHAR(40)) AS RID, \
                   ISNULL(CAST(ISNULL(Qty,0) - ISNULL(ReleasedQty,0) AS NVARCHAR(50)),'0') AS Remain \
                   FROM tStk_Reserve WHERE DocType = @p1 AND DocID = @p2 AND GDSID = @p3 AND StkID = @p4 AND State = 'A' \
                   ORDER BY EDate ASC";
        let params: Vec<&dyn ToSql> = vec![&doc_type, &source_doc_id, &gdsid, &stkid];
        let (reserve_id, remain) = match conn.query(sql, &params).await {
            Ok(stream) => match stream.into_row().await {
                Ok(Some(row)) => {
                    let id = row.get::<&str, _>("RID").unwrap_or("").to_string();
                    let r: f64 = row
                        .get::<&str, _>("Remain")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    (id, r)
                }
                _ => (String::new(), 0.0),
            },
            Err(_) => (String::new(), 0.0),
        };
        if reserve_id.is_empty() || remain <= 0.0001 {
            // 无可用预占记录，剩余的 ship_qty 不再释放（属于无源单出库或源单已耗尽）
            break;
        }
        let to_release = remaining.min(remain).max(0.0);
        if to_release <= 0.0 {
            break;
        }
        let upd = "UPDATE tStk_Reserve SET ReleasedQty = ISNULL(ReleasedQty,0) + @p1, \
                   State = CASE WHEN ISNULL(ReleasedQty,0) + @p1 >= ISNULL(Qty,0) THEN 'X' ELSE 'A' END \
                   WHERE ReserveID = @p2";
        let p2: Vec<&dyn ToSql> = vec![&to_release, &reserve_id];
        if let Err(e) = conn.execute(upd, &p2).await {
            tracing::error!(
                "[release_reserve_by_doc] 释放预占失败: rid={} err={}",
                reserve_id,
                e
            );
            return false;
        }
        remaining -= to_release;
    }
    true
}

/// 反释放预占（销售出库反审时调用）：减少 ReleasedQty，恢复 State='A'
///
/// 修复 H-3：循环处理多条已释放记录（LIFO），直到 ship_qty 耗尽或无已释放记录。
async fn unrelease_reserve_by_doc(
    conn: &mut Conn,
    doc_type: &str,
    source_doc_id: &str,
    gdsid: &str,
    stkid: &str,
    ship_qty: f64,
) -> bool {
    if source_doc_id.is_empty() || gdsid.is_empty() || stkid.is_empty() || ship_qty <= 0.0 {
        return true;
    }
    let mut remaining = ship_qty;
    // 循环按 LIFO（EDate DESC）反释放，直到 remaining=0 或无已释放记录
    while remaining > 0.0001 {
        // CAST 为 NVARCHAR 以兼容 tiberius row.get::<&str,_> 读取模式
        let sql = "SELECT TOP 1 CAST(ReserveID AS NVARCHAR(40)) AS RID, \
                   ISNULL(CAST(ISNULL(ReleasedQty,0) AS NVARCHAR(50)),'0') AS RQ, \
                   ISNULL(CAST(ISNULL(Qty,0) AS NVARCHAR(50)),'0') AS Q \
                   FROM tStk_Reserve WHERE DocType = @p1 AND DocID = @p2 AND GDSID = @p3 AND StkID = @p4 \
                   AND State IN ('A','X') AND ISNULL(ReleasedQty,0) > 0 \
                   ORDER BY EDate DESC";
        let params: Vec<&dyn ToSql> = vec![&doc_type, &source_doc_id, &gdsid, &stkid];
        let (reserve_id, released, total) = match conn.query(sql, &params).await {
            Ok(stream) => match stream.into_row().await {
                Ok(Some(row)) => {
                    let id = row.get::<&str, _>("RID").unwrap_or("").to_string();
                    let rq: f64 = row
                        .get::<&str, _>("RQ")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    let q: f64 = row
                        .get::<&str, _>("Q")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0);
                    (id, rq, q)
                }
                _ => (String::new(), 0.0, 0.0),
            },
            Err(_) => (String::new(), 0.0, 0.0),
        };
        if reserve_id.is_empty() || released <= 0.0001 {
            break;
        }
        let to_unrelease = remaining.min(released).max(0.0);
        if to_unrelease <= 0.0 {
            break;
        }
        let new_released = (released - to_unrelease).max(0.0);
        // 释放量回退后若仍 >= total，State 保持 X；否则恢复 A
        let new_state = if new_released >= total - 0.0001 {
            "X"
        } else {
            "A"
        };
        let upd = "UPDATE tStk_Reserve SET ReleasedQty = @p1, State = @p2 WHERE ReserveID = @p3";
        let p2: Vec<&dyn ToSql> = vec![&new_released, &new_state, &reserve_id];
        if let Err(e) = conn.execute(upd, &p2).await {
            tracing::error!(
                "[unrelease_reserve_by_doc] 反释放预占失败: rid={} err={}",
                reserve_id,
                e
            );
            return false;
        }
        remaining -= to_unrelease;
    }
    true
}

/// 从主表查源 SOID（销售出库释放预占时用）
/// 检查 SOID / FromSOID / SQID 等列
async fn query_source_soid(conn: &mut Conn, table: &str, primary_key: &str, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    for col in &["SOID", "FromSOID", "SQID"] {
        let sql = format!(
            "SELECT ISNULL(CAST([{}] AS NVARCHAR(40)), '') AS S FROM [{}] WHERE [{}] = @p1",
            col, table, primary_key
        );
        if let Ok(stream) = conn.query(&sql, &[&id]).await {
            if let Ok(Some(row)) = stream.into_row().await {
                let v: &str = row.get::<&str, _>("S").unwrap_or("");
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    String::new()
}

/// 检查下游引用：如果当前单据已被下游单据引用（非删除/作废），则不允许反审
/// downstream 格式："tPur_Inv" 或 "tStk_IO:PR"（表名:Kind）
/// 特殊场景：销售退货(SR)/采购退货(PR)通过明细表 tStk_IODetail.SouID 引用源单据主键，
/// 主表 tStk_IO 上没有 SIID/PIID 列，必须 JOIN 明细表才能检查
async fn check_downstream_references(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
) -> Result<(), String> {
    if meta.downstream.is_empty() || master_id.is_empty() {
        return Ok(());
    }
    for ds in &meta.downstream {
        // 解析 "table" 或 "table:kind"
        let (ds_table, ds_kind) = if let Some(idx) = ds.find(':') {
            (&ds[..idx], Some(&ds[idx + 1..]))
        } else {
            (ds.as_str(), None)
        };

        // 获取下游表的 DocMeta（可能不存在）
        let ds_meta = match doc_graph::get_doc_meta(ds_table) {
            Some(m) => m,
            None => continue,
        };

        // 特殊场景：tStk_IO 的 SR/PR 通过明细表 SouID 引用源单据主键
        // 主表上没有 SIID/PIID 列，必须 JOIN tStk_IODetail
        if ds_table == "tStk_IO" && matches!(ds_kind, Some("SR") | Some("PR")) {
            // matches! 已确认 ds_kind = Some("SR") 或 Some("PR")，unwrap 安全
            let kind_str = ds_kind.unwrap_or("SR");
            let sql = "SELECT COUNT(*) AS C FROM tStk_IODetail d \
                       INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                       WHERE io.Kind = @p1 AND d.SouID = @p2 AND ISNULL(io.State,'') NOT IN ('D','C')";
            if let Ok(stream) = conn.query(sql, &[&kind_str, &master_id]).await {
                if let Ok(Some(row)) = stream.into_row().await {
                    let count: i64 = row.get::<i32, _>("C").unwrap_or(0) as i64;
                    if count > 0 {
                        let kind_desc = format!("(Kind={})", kind_str);
                        return Err(format!(
                            "单据已被下游单据 {}{} 引用（{} 条），请先删除/反审下游单据",
                            ds_meta.title, kind_desc, count
                        ));
                    }
                }
            }
            continue; // 已处理，跳过下面的通用逻辑
        }

        // 特殊场景：tStk_ReplenishApply → tStk_IO:PD 通过 Note 字段文本关联（弱关联）
        // tStk_IO 没有 ReplenishApplyID 列，自动生成的 PD 草稿在 Note 中写入 ReplenishApplyID
        // 与 approval.rs unapprove 的 replenish 分支保持一致
        if meta.table == "tStk_ReplenishApply" && ds_table == "tStk_IO" && ds_kind == Some("PD") {
            let kind_str = "PD";
            // 使用 LIKE 匹配 Note 中的 ReplenishApplyID
            let sql = "SELECT COUNT(*) AS C FROM tStk_IO \
                       WHERE Kind = @p1 AND Note LIKE '%' + @p2 + '%' AND ISNULL(State,'') NOT IN ('D','C')";
            if let Ok(stream) = conn.query(sql, &[&kind_str, &master_id]).await {
                if let Ok(Some(row)) = stream.into_row().await {
                    let count: i64 = row.get::<i32, _>("C").unwrap_or(0) as i64;
                    if count > 0 {
                        let kind_desc = format!("(Kind={})", kind_str);
                        return Err(format!(
                            "单据已被下游单据 {}{} 引用（{} 条），请先删除/反审下游单据",
                            ds_meta.title, kind_desc, count
                        ));
                    }
                }
            }
            continue; // 已处理，跳过下面的通用逻辑
        }

        // 候选引用列：当前单据 PK 名 + 下游 source_field
        let mut candidates: Vec<&str> = Vec::new();
        candidates.push(&meta.primary_key);
        if !ds_meta.source_field.is_empty() && ds_meta.source_field != meta.primary_key {
            candidates.push(&ds_meta.source_field);
        }

        for col in &candidates {
            // 构造查询：排除删除(D)和作废(C)状态的下游单据
            let sql = if ds_kind.is_some() {
                if ds_meta.kind_field.is_empty() {
                    continue;
                }
                format!(
                    "SELECT COUNT(*) AS C FROM [{}] WHERE [{}] = @p1 AND [{}] = @p2 AND ISNULL(State,'') NOT IN ('D','C')",
                    ds_table, col, ds_meta.kind_field
                )
            } else {
                format!(
                    "SELECT COUNT(*) AS C FROM [{}] WHERE [{}] = @p1 AND ISNULL(State,'') NOT IN ('D','C')",
                    ds_table, col
                )
            };
            let count: i64 = if let Some(k) = ds_kind {
                let params: Vec<&dyn ToSql> = vec![&master_id, &k];
                match conn.query(&sql, &params).await {
                    Ok(stream) => match stream.into_row().await {
                        Ok(Some(row)) => row.get::<i32, _>("C").unwrap_or(0) as i64,
                        _ => 0,
                    },
                    Err(_) => continue, // 列不存在等错误，跳过尝试下一个候选列
                }
            } else {
                let params: Vec<&dyn ToSql> = vec![&master_id];
                match conn.query(&sql, &params).await {
                    Ok(stream) => match stream.into_row().await {
                        Ok(Some(row)) => row.get::<i32, _>("C").unwrap_or(0) as i64,
                        _ => 0,
                    },
                    Err(_) => continue,
                }
            };
            if count > 0 {
                let kind_desc = ds_kind.map(|k| format!("(Kind={})", k)).unwrap_or_default();
                return Err(format!(
                    "单据已被下游单据 {}{} 引用（{} 条），请先删除/反审下游单据",
                    ds_meta.title, kind_desc, count
                ));
            }
            break; // 该候选列查询成功（无论有无引用），不再尝试其他候选列
        }
    }
    Ok(())
}

async fn reverse_stock_on_unapprove(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
    doc_type: &str,
) -> Result<(), String> {
    if !meta.affects_stock {
        return Ok(());
    }
    // 取单据日期对应的会计月份（反审也按原单据月份回滚 StockYM）
    let doc_ym = query_doc_month(conn, meta, master_id).await;

    // 销售订单反审：释放预占（QQty -= qty）+ 作废 tStk_Reserve 预占记录
    if doc_type == "sales_order" {
        let detail_sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                                  CAST([{}] AS NVARCHAR(40)) AS DID \
                                  FROM [{}] WHERE [{}] = @p1",
            meta.detail_primary_key, meta.detail_table, meta.detail_foreign_key
        );
        let rows: Vec<(String, String, f64, String)> =
            match conn.query(&detail_sql, &[&master_id]).await {
                Ok(s) => match s.into_first_result().await {
                    Ok(rs) => rs
                        .iter()
                        .map(|r| {
                            (
                                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                                r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                                r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                                r.get::<&str, _>("DID").unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
        // 明细行 StkID 为空或全零 UUID 时回退到主表的仓库字段（tStk_IO 的 StkID 在主表上，明细常为 NULL 或全零 UUID）
        // 与 approve_doc 的 post_stock_on_approve 保持一致，否则反审会因 StkID 空/全零而跳过库存回滚
        let is_invalid_stkid = |s: &str| s.is_empty() || s == ZERO_UUID;
        let master_stkid = if rows.iter().any(|(_, s, _, _)| is_invalid_stkid(s)) {
            let ms = read_master_stkid(conn, meta, master_id).await;
            if !is_invalid_stkid(&ms) {
                tracing::warn!(master_stkid = %ms, "[reverse_stock] 检测到明细行 StkID 为空/全零, 回退到主表 StkID");
            }
            ms
        } else {
            String::new()
        };
        let rows: Vec<(String, String, f64, String)> = if !is_invalid_stkid(&master_stkid) {
            rows.into_iter()
                .map(|(g, s, q, d)| {
                    let final_s = if is_invalid_stkid(&s) {
                        master_stkid.clone()
                    } else {
                        s
                    };
                    (g, final_s, q, d)
                })
                .collect()
        } else {
            rows
        };
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let mut tx_failed: Option<String> = None;
        for (gdsid, stkid, qty, _did) in &rows {
            if gdsid.is_empty() || stkid.is_empty() {
                continue;
            }
            // 销售订单反审：释放预占（QQty -= qty）
            if !apply_qqty_delta(conn, gdsid, stkid, -*qty).await {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
                tx_failed = Some(format!(
                    "反审恢复预占失败：商品[{}] 仓库[{}]",
                    fmt_gds(gdsid, &gds_no, &gds_name),
                    fmt_stk(stkid, &stk_no, &stk_name)
                ));
                break;
            }
        }
        // 作废该销售订单的所有预占记录
        if !void_reserve_by_doc(conn, "sales_order", master_id).await {
            tx_failed = Some("作废预占记录失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 销售出库反审：反向过账（Qty += qty）+ 重新预占（QQty -= qty）+ 反释放预占记录 + 删流水
    if doc_type == "sales_outbound" || doc_type == "sales_inv" {
        let detail_sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                                  CAST([{}] AS NVARCHAR(40)) AS DID \
                                  FROM [{}] WHERE [{}] = @p1",
            meta.detail_primary_key, meta.detail_table, meta.detail_foreign_key
        );
        let rows: Vec<(String, String, f64, String)> =
            match conn.query(&detail_sql, &[&master_id]).await {
                Ok(s) => match s.into_first_result().await {
                    Ok(rs) => rs
                        .iter()
                        .map(|r| {
                            (
                                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                                r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                                r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                                r.get::<&str, _>("DID").unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
        // 明细行 StkID 为空或全零 UUID 时回退到主表的仓库字段（tStk_IO 的 StkID 在主表上，明细常为 NULL 或全零 UUID）
        // 与 approve_doc 的 post_stock_on_approve 保持一致，否则反审会因 StkID 空/全零而跳过库存回滚
        let is_invalid_stkid = |s: &str| s.is_empty() || s == ZERO_UUID;
        let master_stkid = if rows.iter().any(|(_, s, _, _)| is_invalid_stkid(s)) {
            let ms = read_master_stkid(conn, meta, master_id).await;
            if !is_invalid_stkid(&ms) {
                tracing::warn!(master_stkid = %ms, "[reverse_stock] 检测到明细行 StkID 为空/全零, 回退到主表 StkID");
            }
            ms
        } else {
            String::new()
        };
        let rows: Vec<(String, String, f64, String)> = if !is_invalid_stkid(&master_stkid) {
            rows.into_iter()
                .map(|(g, s, q, d)| {
                    let final_s = if is_invalid_stkid(&s) {
                        master_stkid.clone()
                    } else {
                        s
                    };
                    (g, final_s, q, d)
                })
                .collect()
        } else {
            rows
        };
        // 查源销售订单 SOID（用于反释放 tStk_Reserve 预占）
        let source_soid = query_source_soid(conn, &meta.table, &meta.primary_key, master_id).await;
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let mut tx_failed: Option<String> = None;
        for (gdsid, stkid, qty, did) in &rows {
            if gdsid.is_empty() || stkid.is_empty() {
                continue;
            }
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            let stk_txt = fmt_stk(stkid, &stk_no, &stk_name);
            // 1) 反向过账：Qty += qty（只动 Qty + StockYM + 快照，不写 TranHis）
            if !reverse_stock_delta_only(conn, gdsid, stkid, *qty, -1.0, doc_ym).await {
                tx_failed = Some(format!(
                    "反审回滚库存失败：商品[{}] 仓库[{}]",
                    gds_txt, stk_txt
                ));
                break;
            }
            // 2) 重新预占：QQty += qty（仅在有源销售订单时才重新预占，与 approve 释放逻辑对称）
            //    无源单的直出库审核时未释放预占，反审时也不应重新预占
            if !source_soid.is_empty() {
                if !apply_qqty_delta(conn, gdsid, stkid, *qty).await {
                    tx_failed = Some(format!(
                        "反审重新预占失败：商品[{}] 仓库[{}]（可用量不足）",
                        gds_txt, stk_txt
                    ));
                    break;
                }
                // 3) 反释放 tStk_Reserve 预占记录（减少 ReleasedQty，恢复 State='A'）
                if !unrelease_reserve_by_doc(conn, "sales_order", &source_soid, gdsid, stkid, *qty)
                    .await
                {
                    tx_failed = Some(format!(
                        "反释放预占记录失败：商品[{}] 仓库[{}]",
                        gds_txt, stk_txt
                    ));
                    break;
                }
            }
            // 4) 回填明细快照
            if meta.table.as_str() == "tStk_IO" {
                fill_detail_stock_snapshot(conn, "tStk_IODetail", "IODetailID", did).await;
            }
        }
        // 删原流水（循环外，一次即可）
        if !inventory_ledger::delete_stock_tran_his(conn, master_id).await {
            tx_failed = Some("删除原流水失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 其他入出库反审：反向过账 + 删流水 + 回填快照（带事务）
    if matches!(
        doc_type,
        "purchase_inbound" | "purchase_receipt" | "purchase_return" | "sales_return" | "stock_io"
    ) {
        let kind = read_kind(conn, meta, master_id).await;
        let use_kind_direction = doc_type == "stock_io";
        let fixed_dir: f64 = match doc_type {
            "purchase_inbound" | "purchase_receipt" | "sales_return" => 1.0,
            "purchase_return" => -1.0,
            _ => 0.0,
        };
        let kind_dir = if use_kind_direction {
            doc_graph::kind_direction(&kind)
        } else {
            0.0
        };
        let per_row_sign = use_kind_direction && kind_dir == 0.0;
        if !use_kind_direction && fixed_dir == 0.0 {
            return Ok(());
        }
        let detail_sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                                  CAST([{}] AS NVARCHAR(40)) AS DID \
                                  FROM [{}] WHERE [{}] = @p1",
            meta.detail_primary_key, meta.detail_table, meta.detail_foreign_key
        );
        let rows: Vec<(String, String, f64, String)> =
            match conn.query(&detail_sql, &[&master_id]).await {
                Ok(s) => match s.into_first_result().await {
                    Ok(rs) => rs
                        .iter()
                        .map(|r| {
                            (
                                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                                r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                                r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                                r.get::<&str, _>("DID").unwrap_or("").to_string(),
                            )
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                },
                Err(_) => Vec::new(),
            };
        // 明细行 StkID 为空或全零 UUID 时回退到主表的仓库字段（tStk_IO 的 StkID 在主表上，明细常为 NULL 或全零 UUID）
        // 与 approve_doc 的 post_stock_on_approve 保持一致，否则反审会因 StkID 空/全零而跳过库存回滚
        let is_invalid_stkid = |s: &str| s.is_empty() || s == ZERO_UUID;
        let master_stkid = if rows.iter().any(|(_, s, _, _)| is_invalid_stkid(s)) {
            let ms = read_master_stkid(conn, meta, master_id).await;
            if !is_invalid_stkid(&ms) {
                tracing::warn!(master_stkid = %ms, "[reverse_stock] 检测到明细行 StkID 为空/全零, 回退到主表 StkID");
            }
            ms
        } else {
            String::new()
        };
        let rows: Vec<(String, String, f64, String)> = if !is_invalid_stkid(&master_stkid) {
            rows.into_iter()
                .map(|(g, s, q, d)| {
                    let final_s = if is_invalid_stkid(&s) {
                        master_stkid.clone()
                    } else {
                        s
                    };
                    (g, final_s, q, d)
                })
                .collect()
        } else {
            rows
        };
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let mut tx_failed: Option<String> = None;
        for (gdsid, stkid, qty, did) in &rows {
            if gdsid.is_empty() || stkid.is_empty() || *qty == 0.0 {
                continue;
            }
            // 决定本行方向：反审时方向取反
            let orig_dir = if per_row_sign {
                if *qty > 0.0 { 1.0 } else { -1.0 }
            } else if use_kind_direction {
                kind_dir
            } else {
                fixed_dir
            };
            if orig_dir == 0.0 {
                continue;
            }
            let abs_qty = qty.abs();
            // 反向过账：只动 Qty + StockYM + 快照，不写 TranHis
            if !reverse_stock_delta_only(conn, gdsid, stkid, abs_qty, orig_dir, doc_ym).await {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                let (stk_no, stk_name) = query_stk_info(conn, stkid).await;
                tx_failed = Some(format!(
                    "反审回滚库存失败：商品[{}] 仓库[{}]",
                    fmt_gds(gdsid, &gds_no, &gds_name),
                    fmt_stk(stkid, &stk_no, &stk_name)
                ));
                break;
            }
            if meta.table.as_str() == "tStk_IO" {
                fill_detail_stock_snapshot(conn, "tStk_IODetail", "IODetailID", did).await;
            }
        }
        // 删原流水（循环外，一次即可）
        if !inventory_ledger::delete_stock_tran_his(conn, master_id).await {
            tx_failed = Some("删除原流水失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 调拨反审：反向双边（带事务）
    if doc_type == "stock_move" {
        let (from_id, to_id) = query_move_stk(conn, master_id).await;
        if from_id.is_empty() || to_id.is_empty() {
            return Ok(());
        }
        let detail_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                          ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, \
                          CAST(MoveDetailID AS NVARCHAR(40)) AS DID \
                          FROM tStk_MoveDetail WHERE MoveID = @p1";
        let rows: Vec<(String, f64, String)> = match conn.query(detail_sql, &[&master_id]).await {
            Ok(s) => match s.into_first_result().await {
                Ok(rs) => rs
                    .iter()
                    .map(|r| {
                        (
                            r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                            r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0),
                            r.get::<&str, _>("DID").unwrap_or("").to_string(),
                        )
                    })
                    .collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let (from_no, from_name) = query_stk_info(conn, &from_id).await;
        let (to_no, to_name) = query_stk_info(conn, &to_id).await;
        let from_txt = fmt_stk(&from_id, &from_no, &from_name);
        let to_txt = fmt_stk(&to_id, &to_no, &to_name);
        let mut tx_failed: Option<String> = None;
        for (gdsid, qty, did) in &rows {
            if gdsid.is_empty() || *qty == 0.0 {
                continue;
            }
            let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
            let gds_txt = fmt_gds(gdsid, &gds_no, &gds_name);
            // 反向：调入仓 -qty（原 +1 → 反向 -1），调出仓 +qty（原 -1 → 反向 +1）
            if !reverse_stock_delta_only(conn, gdsid, &to_id, *qty, 1.0, doc_ym).await {
                tx_failed = Some(format!(
                    "调拨反审回滚失败：商品[{}] 调入仓[{}]",
                    gds_txt, to_txt
                ));
                break;
            }
            if !reverse_stock_delta_only(conn, gdsid, &from_id, *qty, -1.0, doc_ym).await {
                tx_failed = Some(format!(
                    "调拨反审回滚失败：商品[{}] 调出仓[{}]",
                    gds_txt, from_txt
                ));
                break;
            }
            fill_detail_stock_snapshot(conn, "tStk_MoveDetail", "MoveDetailID", did).await;
        }
        if !inventory_ledger::delete_stock_tran_his(conn, master_id).await {
            tx_failed = Some("删除原流水失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 盘点反审：按 DiffQty 反向 + 删流水 + 回填快照（带事务）
    if doc_type == "stock_take" || doc_type == "stock_check" || doc_type == "stocktake" {
        let stk_id = query_tran_stk(conn, master_id).await;
        if stk_id.is_empty() {
            return Ok(());
        }
        let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                       ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                       CAST(TranDetailID AS NVARCHAR(40)) AS DID \
                       FROM tStk_TranDetail WHERE TranID = @p1";
        let rows: Vec<(String, f64, String)> = match conn.query(det_sql, &[&master_id]).await {
            Ok(s) => match s.into_first_result().await {
                Ok(rs) => rs
                    .iter()
                    .map(|r| {
                        (
                            r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                            r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0),
                            r.get::<&str, _>("DID").unwrap_or("").to_string(),
                        )
                    })
                    .collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let (stk_no, stk_name) = query_stk_info(conn, &stk_id).await;
        let stk_txt = fmt_stk(&stk_id, &stk_no, &stk_name);
        let mut tx_failed: Option<String> = None;
        for (gdsid, dq, did) in &rows {
            if gdsid.is_empty() || *dq == 0.0 {
                continue;
            }
            let abs_qty = dq.abs();
            // 原方向：dq>0 → +1（增），dq<0 → -1（减）；反审反向
            let orig_dir = if *dq > 0.0 { 1.0 } else { -1.0 };
            if !reverse_stock_delta_only(conn, gdsid, &stk_id, abs_qty, orig_dir, doc_ym).await {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                tx_failed = Some(format!(
                    "盘点反审回滚失败：商品[{}] 仓库[{}]",
                    fmt_gds(gdsid, &gds_no, &gds_name),
                    stk_txt
                ));
                break;
            }
            fill_detail_stock_snapshot(conn, "tStk_TranDetail", "TranDetailID", did).await;
        }
        if !inventory_ledger::delete_stock_tran_his(conn, master_id).await {
            tx_failed = Some("删除原流水失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    // 周期盘点反审：按 DiffQty 差异反向 + 删流水（带事务）
    if doc_type == "stock_cycle" {
        let stk_sql = "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_StockCycle WHERE CycleID = @p1";
        let stk_id = match conn.query(stk_sql, &[&master_id]).await {
            Ok(s) => match s.into_row().await {
                Ok(Some(r)) => r.get::<&str, _>("S").unwrap_or("").to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        if stk_id.is_empty() {
            return Ok(());
        }
        let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                       ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                       CAST(CycleDetailID AS NVARCHAR(40)) AS DID \
                       FROM tStk_StockCycleDetail WHERE CycleID = @p1";
        let rows: Vec<(String, f64, String)> = match conn.query(det_sql, &[&master_id]).await {
            Ok(s) => match s.into_first_result().await {
                Ok(rs) => rs
                    .iter()
                    .map(|r| {
                        (
                            r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                            r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0),
                            r.get::<&str, _>("DID").unwrap_or("").to_string(),
                        )
                    })
                    .collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        if let Err(e) = inventory_ledger::begin_tran(conn).await {
            return Err(e);
        }
        let (stk_no, stk_name) = query_stk_info(conn, &stk_id).await;
        let stk_txt = fmt_stk(&stk_id, &stk_no, &stk_name);
        let mut tx_failed: Option<String> = None;
        for (gdsid, diff, did) in &rows {
            if gdsid.is_empty() {
                continue;
            }
            if diff.abs() < 0.0001 {
                continue;
            }
            let abs_qty = diff.abs();
            // 原方向：diff>0 → +1（增），diff<0 → -1（减）；反审反向
            let orig_dir = if *diff > 0.0 { 1.0 } else { -1.0 };
            if !reverse_stock_delta_only(conn, gdsid, &stk_id, abs_qty, orig_dir, doc_ym).await {
                let (gds_no, gds_name) = query_gds_info(conn, gdsid).await;
                tx_failed = Some(format!(
                    "周期盘点反审回滚失败：商品[{}] 仓库[{}]",
                    fmt_gds(gdsid, &gds_no, &gds_name),
                    stk_txt
                ));
                break;
            }
            // 回填明细快照（与普通盘点反审保持一致）
            fill_detail_stock_snapshot(conn, "tStk_StockCycleDetail", "CycleDetailID", did).await;
        }
        if !inventory_ledger::delete_stock_tran_his(conn, master_id).await {
            tx_failed = Some("删除原流水失败".to_string());
        }
        if let Some(err) = tx_failed {
            inventory_ledger::rollback_tran(conn).await;
            return Err(err);
        }
        if let Err(e) = inventory_ledger::commit_tran(conn).await {
            inventory_ledger::rollback_tran(conn).await;
            return Err(e);
        }
        return Ok(());
    }

    Ok(())
}

// ============== 硬删除（仅允许软删状态） ==============

/// 硬删除单据（物理删除主表+明细表）
///
/// 仅允许软删除状态（State='D'）的单据被物理删除。
/// 已审核/草稿状态需先经过软删除流程（软删时已回滚库存），保证审计完整性。
///
/// 对于非单据表（doc_graph 中未定义），直接返回 Ok(())，由调用方处理。
pub async fn hard_delete_doc(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    id: &str,
) -> Result<(), String> {
    let meta = match doc_graph::get_doc_meta(table) {
        Some(m) => m.clone(),
        None => return Ok(()), // 非单据表，不处理库存
    };

    // 查询单据状态
    let state = query_doc_state(conn, table, primary_key, id).await;

    // 项目原则：只有软删除状态（State='D'）的单据才能被物理删除。
    // 已审核（'S'）/ 已确认（'Y'）单据必须先反审，再软删，最后才能彻底删除；
    // 草稿（'N'/'E'）也应先软删，避免跳过软删流程直接物理删除导致审计断裂。
    // 反审时已经反向调整过库存，软删时不再回滚库存，因此 D 状态的单据库存是干净的。
    if state != STATE_DELETED {
        return Err(format!(
            "单据状态为 {}，不允许彻底删除（仅软删除状态 'D' 可彻底删除，请先软删除）",
            state
        ));
    }

    // 修复 H-2：用事务包裹"删明细 + 删主表"，避免中途失败导致孤儿主表/明细
    // 同时清理 tStk_Reserve 预占记录（销售订单硬删除后预占记录不应残留）
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(conn).await?;

        // 销售订单：清理 tStk_Reserve 预占记录（DocID 存的是 SOID 主键）
        if table == "tSal_Order" {
            let del_reserve_sql = "DELETE FROM tStk_Reserve WHERE DocID = @p1";
            conn.execute(del_reserve_sql, &[&id])
                .await
                .map_err(|e| format!("清理预占记录失败 [tStk_Reserve]: {}", e))?;
        }

        // 删除明细表（如有）
        if !meta.detail_table.is_empty() && !meta.detail_foreign_key.is_empty() {
            let del_detail_sql = format!(
                "DELETE FROM [{}] WHERE [{}] = @p1",
                meta.detail_table, meta.detail_foreign_key
            );
            conn.execute(&del_detail_sql, &[&id])
                .await
                .map_err(|e| format!("删除明细失败 [{}]: {}", meta.detail_table, e))?;
        }

        // 删除主表
        let del_main_sql = format!("DELETE FROM [{}] WHERE [{}] = @p1", table, primary_key);
        let result = conn
            .execute(&del_main_sql, &[&id])
            .await
            .map_err(|e| format!("删除主表失败 [{}]: {}", table, e))?;
        // 检查 rows_affected，避免单据不存在时仍返回成功
        let rows = result.rows_affected().first().copied().unwrap_or(0);
        if rows == 0 {
            return Err(format!("单据不存在或已被删除: table={} id={}", table, id));
        }

        inventory_ledger::commit_tran(conn).await?;
        Ok(())
    }
    .await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(conn).await;
        return Err(e);
    }

    Ok(())
}

/// 软删除单据前的库存回滚
///
/// 如果单据已审核（State='S'），先回滚库存（reverse_stock_on_unapprove），
/// 然后由调用方继续执行 UPDATE State='D'。
/// 对于非单据表（doc_graph 中未定义）或未审核单据，直接返回 Ok(())。
///
/// 这样设计是为了和 generic_delete 的软删除分支配合：
/// generic_delete 先调用本函数回滚库存，再执行 UPDATE State='D'。
/// 避免出现"单据软删但库存未回滚"的悬空数据。
pub async fn soft_delete_doc(
    conn: &mut Conn,
    table: &str,
    primary_key: &str,
    id: &str,
) -> Result<(), String> {
    let meta = match doc_graph::get_doc_meta(table) {
        Some(m) => m.clone(),
        None => return Ok(()), // 非单据表，不处理库存
    };

    // 查询单据状态
    let state = query_doc_state(conn, table, primary_key, id).await;

    // 已软删状态：幂等返回 Ok，避免阻塞批量删除流程
    // （前端 DataPage 在"显示删除"模式下可能把 D 状态数据传进来再次软删，
    //   generic_delete 的 for 循环遇到 Err 会提前返回，导致后续 id 无法处理）
    if state == STATE_DELETED {
        tracing::debug!(
            "[soft_delete_doc] 单据已软删除，幂等跳过: table={} id={}",
            table,
            id
        );
        return Ok(());
    }

    // 已作废（'C'）/ 已确认（'Y'）状态：不允许直接软删，要求先反审/反确认
    if state == STATE_VOID || state == STATE_CONFIRMED {
        return Err(format!(
            "单据状态为 {}，不允许软删除（请先反审或反确认）",
            state
        ));
    }

    // 如果已审核（'S'），先回滚库存
    // 修复 TOCTOU 竞态（C-1）：用 CAS S→D 抢占状态锁，保证并发场景下只有一个请求
    // 能进入反向过账流程，避免"反审 + 软删并发"导致库存被重复反向调整。
    // 注：原设计依赖 SQL Server X 锁避免竞态是错误的——reverse_stock 操作的是
    // tStk_Stock，与单据表的 X 锁不互斥，无法阻止并发的 unapprove_doc CAS S→N。
    // 门店销售单不进库存流水，已审核门店销售单软删时无需反向过账，直接 CAS S→D 即可
    if state == STATE_REVIEWED {
        // 确定 doc_type（tStk_IO 统一用 "stock_io"，内部按 Kind 路由）
        let doc_type = if table == "tStk_IO" {
            "stock_io".to_string()
        } else {
            meta.biz_type.to_string()
        };
        // CAS S→D：抢占状态锁，确保只有一个请求能进入反向过账
        let cas_ok = update_doc_state_with_cas(
            conn,
            table,
            primary_key,
            id,
            STATE_DELETED,
            ZERO_UUID,
            Some(&[STATE_REVIEWED]),
        )
        .await;
        if !cas_ok {
            return Err(format!("单据状态已变更，软删失败（仅 S 状态可回滚库存）"));
        }
        // 门店销售单跳过反向过账（不进库存流水）
        let is_retail_sale = is_retail_sale_inv_by_db(conn, table, primary_key, id).await;
        if is_retail_sale {
            tracing::info!("[soft_delete_doc] 门店销售单软删，跳过反向过账: id={}", id);
            return Ok(());
        }
        tracing::info!(
            "[soft_delete_doc] 单据已审核，软删前先回滚库存: table={} id={} doc_type={}",
            table,
            id,
            doc_type
        );
        // 反向过账（内部已有 begin_tran/commit_tran/rollback_tran 事务包裹）
        if let Err(e) = reverse_stock_on_unapprove(conn, &meta, id, &doc_type).await {
            // 反向过账失败：回滚状态 D→S，让用户可以重试
            let _ = update_doc_state_with_cas(
                conn,
                table,
                primary_key,
                id,
                STATE_REVIEWED,
                ZERO_UUID,
                Some(&[STATE_DELETED]),
            )
            .await;
            return Err(e);
        }
        // 软删完成：状态已是 D，库存已回滚。generic_delete 后续的 UPDATE State='D' 是幂等无害。
        return Ok(());
    }

    Ok(())
}

// ============== 作废 ==============

pub async fn void_doc(
    conn: &mut Conn,
    user_code: &str,
    _user_name: &str,
    params: VoidDocParams,
) -> Result<String, String> {
    let meta = doc_graph::get_doc_meta(&params.table)
        .ok_or_else(|| format!("未知业务单据表: {}", params.table))?
        .clone();
    let cur_state = query_doc_state(conn, &params.table, &params.primary_key, &params.id).await;
    if cur_state == STATE_VOID {
        return Err("单据已作废".to_string());
    }
    // 已审核的单据需先反审才能作废
    if cur_state == STATE_REVIEWED {
        return Err("已审核单据请先反审再作废".to_string());
    }
    // 修复 M-2：检查下游引用，避免作废已被下游单据引用的源单
    // （N/E 状态单据虽然尚未审核，但下游草稿可能已引用；作废源单将使下游挂空引用）
    if let Err(e) = check_downstream_references(conn, &meta, &params.id).await {
        return Err(e);
    }
    // 修改前查旧数据快照（用于操作日志变更明细）
    let before_snapshot: Option<serde_json::Value> = query_doc_snapshot(
        conn,
        &params.table,
        &params.primary_key,
        &params.id,
        &meta.detail_table,
        &meta.detail_foreign_key,
        &meta.detail_primary_key,
    )
    .await;
    // 修复 M-4：把工号解析为 EmpID 后再写入 AUser
    let auser = resolve_auser_id(conn, user_code).await;
    // 修复 M-1：CAS 状态变更 + after_snapshot + 操作日志 整体放入事务，
    // 避免状态已置为 C 但日志写入失败时审计字段缺失
    if let Err(e) = inventory_ledger::begin_tran(conn).await {
        return Err(e);
    }
    let tx_result: std::result::Result<(), String> = async {
        // 修复 P0-2：检查 CAS 返回值，避免单据状态被并发请求变更时仍返回"作废成功"
        let cas_ok = update_doc_state_with_cas(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            STATE_VOID,
            &auser,
            Some(&[STATE_NEW, STATE_EDIT]),
        )
        .await;
        if !cas_ok {
            return Err(format!(
                "单据 {} 状态已变更，作废失败（仅新建/编辑中可作废）",
                params.id
            ));
        }
        let reason = params.reason.clone().unwrap_or_default();
        // 写操作日志（含数据快照）
        // after 快照重新查询 DB：保证 before/after 都是完整行（含 Items），便于变更明细展示
        let after_snapshot: Option<serde_json::Value> = query_doc_snapshot(
            conn,
            &params.table,
            &params.primary_key,
            &params.id,
            &meta.detail_table,
            &meta.detail_foreign_key,
            &meta.detail_primary_key,
        )
        .await;
        let before_json = before_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let after_json = after_snapshot
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());
        let _ = record_oper_with_data(
            conn,
            "VOID",
            &params.table,
            &params.id,
            user_code,
            None,
            Some(&format!("作废{} ({})", meta.title, reason)),
            before_json.as_deref(),
            after_json.as_deref(),
        )
        .await;
        Ok(())
    }
    .await;
    match tx_result {
        Ok(()) => {
            if let Err(e) = inventory_ledger::commit_tran(conn).await {
                inventory_ledger::rollback_tran(conn).await;
                return Err(e);
            }
            Ok("作废成功".to_string())
        }
        Err(e) => {
            inventory_ledger::rollback_tran(conn).await;
            Err(e)
        }
    }
}

// ============== 参照生单 ==============

pub async fn generate_from_source(
    conn: &mut Conn,
    _user_code: &str,
    params: GenerateFromSourceParams,
) -> Result<GenerateFromSourceResponse, String> {
    let source_meta = doc_graph::get_doc_meta(&params.source_table)
        .ok_or_else(|| format!("未知源单表: {}", params.source_table))?
        .clone();
    let target_meta = doc_graph::get_doc_meta(&params.target_table)
        .ok_or_else(|| format!("未知目标表: {}", params.target_table))?
        .clone();

    // 校验上下游关系
    if !target_meta
        .upstream
        .iter()
        .any(|s| s == &params.source_table)
    {
        return Err(format!(
            "{} 不是 {} 的合法上游",
            params.source_table, params.target_table
        ));
    }

    // 取主表
    let master_sql = format!(
        "SELECT * FROM [{}] WHERE [{}] = @p1",
        source_meta.table, source_meta.primary_key
    );
    let master_row = match conn.query(&master_sql, &[&params.source_id]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(_) => None,
    };
    let mut master_json = if let Some(row) = master_row {
        row_to_json(row, &source_meta.table)
    } else {
        return Err("源单主表未找到".to_string());
    };

    // 修复 M-3：校验源单状态，仅 S（已审核）/Y（已确认）状态可被参照生单
    // N/E 状态源单尚未审核，不应被下游引用；D/C 状态源单已失效
    let src_state = master_json
        .get("State")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if !matches!(src_state.as_str(), STATE_REVIEWED | STATE_CONFIRMED) {
        let state_desc = match src_state.as_str() {
            "" => "空",
            STATE_NEW => "新建",
            STATE_EDIT => "编辑中",
            STATE_DELETED => "删除",
            STATE_VOID => "已作废",
            _ => src_state.as_str(),
        };
        return Err(format!(
            "源单状态为 {}，仅已审核/已确认状态可参照生单",
            state_desc
        ));
    }

    // 清空主键、单据号、源单据关联
    if let Some(obj) = master_json.as_object_mut() {
        obj.remove(&target_meta.primary_key);
        obj.remove(&target_meta.no_field);
        obj.remove("State");
        obj.remove("AUser");
        obj.remove("ADate");
        obj.remove("SUser");
        obj.remove("SDate");
        obj.remove("LUTime");
        obj.remove("EDate");
        obj.remove("EUser");
        if !target_meta.source_field.is_empty() {
            // 保留 source_field 指向源单主键
            obj.insert(
                target_meta.source_field.clone(),
                serde_json::Value::String(params.source_id.clone()),
            );
        }
    }

    // 取明细
    let detail_sql = format!(
        "SELECT * FROM [{}] WHERE [{}] = @p1",
        source_meta.detail_table, source_meta.detail_foreign_key
    );
    let detail_rows = match conn.query(&detail_sql, &[&params.source_id]).await {
        Ok(s) => s.into_first_result().await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let mut details: Vec<serde_json::Value> = Vec::new();
    for r in detail_rows {
        let mut d = row_to_json(r, &source_meta.detail_table);
        if let Some(obj) = d.as_object_mut() {
            obj.remove(&source_meta.detail_primary_key);
            obj.remove(&source_meta.detail_foreign_key);
            obj.remove("StkQty");
            obj.remove("AQty");
            obj.remove("RowNO");
        }
        details.push(d);
    }

    Ok(GenerateFromSourceResponse {
        target_table: target_meta.table.to_string(),
        master: master_json,
        details,
    })
}

// ============== 工具函数 ==============

fn row_to_json(row: tiberius::Row, _table: &str) -> serde_json::Value {
    use tiberius::ColumnType;
    let mut obj = serde_json::Map::new();
    for col in row.columns() {
        let name = col.name();
        let v: serde_json::Value = match col.column_type() {
            ColumnType::Int1 | ColumnType::Int2 | ColumnType::Int4 => row
                .get::<i32, _>(name)
                .map(|x| serde_json::Value::from(x))
                .unwrap_or(serde_json::Value::Null),
            ColumnType::Int8 | ColumnType::Intn => row
                .get::<i64, _>(name)
                .map(|x| serde_json::Value::from(x))
                .unwrap_or(serde_json::Value::Null),
            ColumnType::Float4 | ColumnType::Float8 | ColumnType::Floatn => row
                .get::<f64, _>(name)
                .map(|x| serde_json::json!(x))
                .unwrap_or(serde_json::Value::Null),
            ColumnType::Bit => row
                .get::<bool, _>(name)
                .map(|x| serde_json::Value::from(x))
                .unwrap_or(serde_json::Value::Null),
            ColumnType::Guid => row
                .get::<uuid::Uuid, _>(name)
                .map(|x| serde_json::Value::from(x.to_string()))
                .unwrap_or(serde_json::Value::Null),
            _ => row
                .get::<&str, _>(name)
                .map(|x| serde_json::Value::from(x))
                .unwrap_or(serde_json::Value::Null),
        };
        obj.insert(name.to_string(), v);
    }
    serde_json::Value::Object(obj)
}

/// 把 JSON Value 转成 Box<dyn ToSql>
fn json_to_sql(v: &serde_json::Value) -> Box<dyn ToSql + Send + Sync> {
    match v {
        serde_json::Value::Null => Box::new(Option::<String>::None),
        serde_json::Value::Bool(b) => Box::new(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        _ => Box::new(v.to_string()),
    }
}

/// 根据字段名判断是否为 UUID 字段，并对空字符串做特殊处理
/// - UUID 字段（以 ID 结尾）：空字符串 → 全零 UUID（NOT NULL 字段也能接受）
/// - 非 UUID 字段：空字符串 → NULL
fn json_to_sql_for_field(field: &str, v: &serde_json::Value) -> Box<dyn ToSql + Send + Sync> {
    match v {
        serde_json::Value::String(s) => {
            if s.is_empty() {
                // UUID 字段（以 ID 结尾，如 DeptID/EmpID/StkID/SuppID 等）用全零 UUID
                if field.ends_with("ID") {
                    Box::new("00000000-0000-0000-0000-000000000000".to_string())
                } else {
                    // 非 UUID 字段空字符串转为 NULL
                    Box::new(Option::<String>::None)
                }
            } else {
                Box::new(s.clone())
            }
        }
        _ => json_to_sql(v),
    }
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// 表列信息（用于补全 NOT NULL 字段默认值）
struct ColumnInfo {
    name: String,
    data_type: String,
    is_nullable: bool,
    has_default: bool,
}

/// 查询表的列信息
async fn query_table_columns(conn: &mut Conn, table: &str) -> Vec<ColumnInfo> {
    let sql = "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT \
               FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @p1";
    let params: Vec<&dyn ToSql> = vec![&table];
    let mut result: Vec<ColumnInfo> = Vec::new();
    if let Ok(stream) = conn.query(sql, &params).await {
        if let Ok(rows) = stream.into_first_result().await {
            for r in rows {
                let name = r.get::<&str, _>("COLUMN_NAME").unwrap_or("").to_string();
                let data_type = r.get::<&str, _>("DATA_TYPE").unwrap_or("").to_string();
                let is_nullable = r.get::<&str, _>("IS_NULLABLE").unwrap_or("YES") == "YES";
                let has_default = r.get::<&str, _>("COLUMN_DEFAULT").is_some();
                if !name.is_empty() {
                    result.push(ColumnInfo {
                        name,
                        data_type,
                        is_nullable,
                        has_default,
                    });
                }
            }
        }
    }
    result
}

/// 根据数据类型生成默认值
fn default_value_for_type(data_type: &str) -> serde_json::Value {
    match data_type {
        "uniqueidentifier" => {
            serde_json::Value::String("00000000-0000-0000-0000-000000000000".to_string())
        }
        "int" | "tinyint" | "smallint" | "bigint" => {
            serde_json::Value::Number(serde_json::Number::from(0))
        }
        "decimal" | "numeric" | "float" | "real" | "money" | "smallmoney" => {
            serde_json::Number::from_f64(0.0)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        "bit" => serde_json::Value::Bool(false),
        "nvarchar" | "varchar" | "nchar" | "char" | "text" | "ntext" => {
            serde_json::Value::String(String::new())
        }
        "datetime" | "datetime2" | "smalldatetime" | "date" => {
            serde_json::Value::String(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        }
        _ => serde_json::Value::Null,
    }
}

/// 补全 JSON 对象中缺失的 NOT NULL 字段（无默认值的）
fn fill_not_null_defaults(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    columns: &[ColumnInfo],
    skip_keys: &[&str],
) {
    for col in columns {
        // 跳过已处理的主键、外键、RowNO
        if skip_keys.contains(&col.name.as_str()) {
            continue;
        }
        // 只处理 NOT NULL 且无默认值的字段
        if col.is_nullable || col.has_default {
            continue;
        }
        // 如果前端没传这个字段，补上默认值
        if !obj.contains_key(&col.name) {
            let dv = default_value_for_type(&col.data_type);
            if !dv.is_null() {
                obj.insert(col.name.clone(), dv);
            }
        } else if obj.get(&col.name).map(|v| v.is_null()).unwrap_or(false) {
            // 前端传了 null，也补上默认值
            let dv = default_value_for_type(&col.data_type);
            if !dv.is_null() {
                obj.insert(col.name.clone(), dv);
            }
        }
    }
}

/// 过滤 JSON 对象：只保留数据库表中存在的字段，丢弃 _isNew/_rowKey/AInPrice 等无关字段
/// 同时对 UUID 类型的空字符串/无效值补全为全零 UUID
fn filter_to_db_columns(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    columns: &[ColumnInfo],
    skip_keys: &[&str],
) {
    // 收集数据库列名集合
    let db_names: std::collections::HashSet<String> =
        columns.iter().map(|c| c.name.clone()).collect();
    // 删除数据库不存在的字段
    let keys_to_remove: Vec<String> = obj
        .keys()
        .filter(|k| !db_names.contains(*k))
        .cloned()
        .collect();
    for k in keys_to_remove {
        obj.remove(&k);
    }
    // 对 UUID 类型字段，空值补全为全零 UUID（避免 NOT NULL 的 uniqueidentifier 转换失败）
    for col in columns {
        if skip_keys.contains(&col.name.as_str()) {
            continue;
        }
        if col.data_type != "uniqueidentifier" {
            continue;
        }
        if let Some(v) = obj.get(&col.name) {
            let need_fix = match v {
                serde_json::Value::Null => true,
                serde_json::Value::String(s) => s.is_empty() || (!s.contains('-') && s.len() != 36),
                _ => false,
            };
            if need_fix {
                obj.insert(
                    col.name.clone(),
                    serde_json::Value::String("00000000-0000-0000-0000-000000000000".to_string()),
                );
            }
        }
    }
}

/// 通过工号查询 EmpID（EUser 字段存的是 EmpID 的 UUID）
async fn query_emp_id_by_code(conn: &mut Conn, user_code: &str) -> String {
    if user_code.is_empty() {
        return String::new();
    }
    let sql = "SELECT CAST(EmpID AS NVARCHAR(40)) AS EID FROM tBas_Emp WHERE EmpNO = @p1";
    let params: Vec<&dyn ToSql> = vec![&user_code];
    if let Ok(stream) = conn.query(sql, &params).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("EID").unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 从单据主表 JSON 中读取客户 ID 和名称（save 路径）
/// - 销售类单据（tSal_Inv / tSal_Order / tSal_Return）字段名是 CustID
/// - 采购类单据不缺货（方向反了），不会走到这里
/// - 调拨/盘点等无客户字段，返回空
async fn query_cust_info_from_data(
    _conn: &mut Conn,
    meta: &DocMeta,
    data: &serde_json::Value,
) -> (String, String) {
    // 销售类单据 + 入出库单（tStk_IO 也有 CustID 字段，如门店销售 SD/SI/POS）
    let doc_type = default_doc_type_for_table(meta);
    if !matches!(
        doc_type.as_str(),
        "sales_outbound" | "sales_inv" | "sales_return" | "sales_order" | "stock_io"
    ) {
        return (String::new(), String::new());
    }
    let cust_id = data
        .get("CustID")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if cust_id.is_empty() {
        return (String::new(), String::new());
    }
    // CustName 可能在 data 中（前端传入），也可能不在（需查 DB）
    let cust_name = data
        .get("CustName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !cust_name.is_empty() {
        return (cust_id, cust_name);
    }
    // data 中没有 CustName，查询 tBas_Cust
    let sql = "SELECT TOP 1 CustName FROM tBas_Cust WHERE CustID = @p1";
    let params: Vec<&dyn ToSql> = vec![&cust_id];
    if let Ok(stream) = _conn.query(sql, &params).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let name = row.get::<&str, _>("CustName").unwrap_or("").to_string();
            return (cust_id, name);
        }
    }
    (cust_id, String::new())
}

/// 从单据主表 DB 中读取客户 ID 和名称（approve 路径）
/// - master_id 是单据主键值
/// - 通过 meta.table + meta.primary_key 查询 CustID，再 JOIN tBas_Cust 取 CustName
async fn query_cust_info_from_db(
    conn: &mut Conn,
    meta: &DocMeta,
    master_id: &str,
) -> (String, String) {
    let doc_type = default_doc_type_for_table(meta);
    if !matches!(
        doc_type.as_str(),
        "sales_outbound" | "sales_inv" | "sales_return" | "sales_order" | "stock_io"
    ) {
        return (String::new(), String::new());
    }
    if master_id.is_empty() {
        return (String::new(), String::new());
    }
    // 单据表 LEFT JOIN tBas_Cust 一次取回 CustID + CustName
    let sql = format!(
        "SELECT TOP 1 CAST(s.CustID AS NVARCHAR(40)) AS CustID, c.CustName AS CustName \
         FROM [{}] s LEFT JOIN tBas_Cust c ON s.CustID = c.CustID \
         WHERE s.[{}] = @p1",
        meta.table, meta.primary_key
    );
    let params: Vec<&dyn ToSql> = vec![&master_id];
    if let Ok(stream) = conn.query(&sql, &params).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let cid = row.get::<&str, _>("CustID").unwrap_or("").to_string();
            let cname = row.get::<&str, _>("CustName").unwrap_or("").to_string();
            return (cid, cname);
        }
    }
    (String::new(), String::new())
}

/// 把缺货明细持久化到 tStk_Shortage 表（缺货记录页面数据源）
///
/// 在 validate_outbound_stock / validate_outbound_stock_for_approve /
/// validate_move_outbound_stock 内部调用：
/// - 调用时机：在返回 ApproveError::Shortage 之前
/// - 事务边界：在 begin_tran 之前调用，缺货记录独立于单据事务，单据回滚不影响缺货记录
/// - 失败处理：写入失败仅记录日志，不影响主流程（已通过 Err 返回缺货错误给前端）
///
/// source_doc_table / source_doc_no / source_doc_id：
///   - save 场景：doc_no 从 data 读取（可能是占位符），source_doc_id = pk_value（新增时为空）
///   - approve 场景：doc_no 从 DB 查询，source_doc_id = master_id（已存在的单据 ID）
async fn log_shortage_to_db(
    conn: &mut Conn,
    items: &[StockShortageItem],
    source_doc_table: &str,
    source_doc_no: &str,
    source_doc_id: &str,
    user_code: &str,
    emp_id: &str,
    source_kind: &str,
    cust_id: &str,
    cust_name: &str,
    shop_id: &str,
    shop_name: &str,
) {
    if items.is_empty() {
        return;
    }
    tracing::info!(
        table = %source_doc_table,
        doc_no = %source_doc_no,
        doc_id = %source_doc_id,
        source_kind = %source_kind,
        user_code = %user_code,
        emp_id = %emp_id,
        item_count = items.len(),
        "[log_shortage_to_db] 开始写入缺货记录"
    );
    if emp_id.is_empty() {
        tracing::warn!(
            table = %source_doc_table,
            doc_no = %source_doc_no,
            "缺货记录写入跳过：无法解析当前用户 EmpID（user_code={}）",
            user_code
        );
        return;
    }
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // SourceDocID 是 UNIQUEIDENTIFIER 类型，空字符串无法转换，需转为 NULL
    // save 场景新增单据时 pk_value 为空，approve 场景 master_id 已存在
    let source_doc_id_param: Option<&str> = if source_doc_id.is_empty() {
        None
    } else {
        Some(source_doc_id)
    };
    // CustID 同为 UNIQUEIDENTIFIER，空字符串需转 NULL
    let cust_id_param: Option<&str> = if cust_id.is_empty() {
        None
    } else {
        Some(cust_id)
    };
    // CustName 空字符串转 NULL（避免显示空字符串）
    let cust_name_param: Option<&str> = if cust_name.is_empty() {
        None
    } else {
        Some(cust_name)
    };
    // ShopID 同为 UNIQUEIDENTIFIER，空字符串需转 NULL
    let shop_id_param: Option<&str> = if shop_id.is_empty() {
        None
    } else {
        Some(shop_id)
    };
    let shop_name_param: Option<&str> = if shop_name.is_empty() {
        None
    } else {
        Some(shop_name)
    };

    let sql = "INSERT INTO [tStk_Shortage] \
        ([ShortageID], [GDSID], [StkID], [Qty], [ShortQty], [StockQty], [ReservedQty], \
         [SourceDocTable], [SourceDocNo], [SourceDocID], [SourceKind], [Remark], \
         [EUser], [EmpID], [EDate], [State], [CustID], [CustName], [ShopID], [ShopName]) \
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, 'N', @p15, @p16, @p17, @p18)";

    for item in items {
        if item.gds_id.is_empty() || item.stk_id.is_empty() {
            continue;
        }
        // Remark 简短记录来源单据 + 商品名（便于人工排查；详细来源通过 SourceDocID 关联查询）
        let remark = format!("{}: {}", source_doc_no, item.gds_name);
        let params: Vec<&dyn ToSql> = vec![
            &item.gds_id,
            &item.stk_id,
            &item.qty,
            &item.shortage,
            &item.stock,
            &item.reserved,
            &source_doc_table,
            &source_doc_no,
            &source_doc_id_param,
            &source_kind,
            &remark,
            &user_code,
            &emp_id,
            &now,
            &cust_id_param,
            &cust_name_param,
            &shop_id_param,
            &shop_name_param,
        ];
        if let Err(e) = conn.execute(sql, &params).await {
            tracing::error!(
                table = %source_doc_table,
                doc_no = %source_doc_no,
                gds_id = %item.gds_id,
                stk_id = %item.stk_id,
                qty = item.qty,
                short_qty = item.shortage,
                stock_qty = item.stock,
                reserved = item.reserved,
                source_kind = %source_kind,
                user_code = %user_code,
                emp_id = %emp_id,
                "缺货记录写入失败: {}",
                e
            );
        } else {
            tracing::info!(
                gds_id = %item.gds_id,
                stk_id = %item.stk_id,
                qty = item.qty,
                short_qty = item.shortage,
                "[log_shortage_to_db] 写入成功"
            );
        }
    }
}

/// 解析 user_code（可能是工号或 UUID）为可写入 AUser 字段的 EmpID
/// 修复 M-4：原 format_uuid_or_zero(user_code) 把工号当 UUID，导致 AUser 恒为零 UUID，审计字段失效
/// 优先按工号查 tBas_Emp.EmpID；查不到则返回零 UUID
async fn resolve_auser_id(conn: &mut Conn, user_code: &str) -> String {
    if user_code.is_empty() {
        return ZERO_UUID.to_string();
    }
    // 若 user_code 本身就是 UUID 格式，直接使用
    if user_code.len() == 36 && user_code.chars().filter(|c| *c == '-').count() == 4 {
        return user_code.to_string();
    }
    // 按工号查 EmpID
    let emp_id = query_emp_id_by_code(conn, user_code).await;
    if emp_id.is_empty() {
        tracing::warn!(
            "[resolve_auser_id] 未找到员工记录，AUser 将置为零 UUID: user_code={}",
            user_code
        );
        return ZERO_UUID.to_string();
    }
    emp_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_save_params_deserialize() {
        let json = r#"{"table":"tPur_Order","primary_key":"POID","data":{"POID":"","PoNo":"PO-001","PODate":"2026-01-01","StkID":"S1","SuppID":"SU1","EUser":"admin"},"details":[{"GDSID":"G1","Qty":10,"Price":5.0}]}"#;
        let params: SaveDocParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.table, "tPur_Order");
        assert_eq!(params.details.len(), 1);
    }

    #[test]
    fn test_approve_params_deserialize() {
        let json = r#"{"table":"tPur_Order","primary_key":"POID","id":"xxx-uuid","doc_type":"purchase_order"}"#;
        let params: ApproveDocParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.table, "tPur_Order");
        assert_eq!(params.doc_type, Some("purchase_order".to_string()));
    }

    #[test]
    fn test_json_to_sql() {
        let v = serde_json::json!(123);
        let _b = json_to_sql(&v);
        let v2 = serde_json::json!("abc");
        let _b2 = json_to_sql(&v2);
        let v3 = serde_json::json!(true);
        let _b3 = json_to_sql(&v3);
    }

    #[test]
    fn test_uuid_v4() {
        let u = uuid_v4();
        assert_eq!(u.len(), 36);
    }
}
