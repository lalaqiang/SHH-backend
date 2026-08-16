use axum::extract::{State, Json, Multipart, Extension};
use axum::response::Response;
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::utils::error_codes::*;
use crate::handlers::base_data::{try_get_value, row_to_json};
use crate::middleware::auth::Claims;
use crate::services::inventory_ledger;
use crate::utils::password::hash_password;

/// 系统敏感表黑名单：这些表禁止通过 /api/generic/* 接口直接 CRUD，
/// 必须走专用接口（专用接口已配置权限码校验）。
///
/// 原因：通用接口无法从表名自动推断权限码，且系统表操作影响全局安全。
/// 专用接口路径明确，可在 `middleware/permission.rs` 中精确映射权限码。
///
/// P0-S3 修复：原黑名单仅 9 张表，遗漏大量敏感系统表（tBas_Emp 含密码哈希、
///   tSys_Parameters/tSys_Config 含业务规则、tSys_OperHis 可被篡改、tSys_Backup 含备份记录等）
///   补全为前缀匹配 + 显式列表双重防护
const SYSTEM_TABLE_BLACKLIST: &[&str] = &[
    // ===== 原有 9 张 =====
    "tSys_Rule",          // 角色：走 /api/permission/role/*
    "tSys_RuleMenu",      // 角色权限：走 /api/permission/assign
    "tSys_UserRule",      // 用户角色：走 /api/permission/assign-user-roles
    "tSys_Menus",         // 菜单：通过菜单管理专用接口
    "tSys_DocNoSeq",      // 单据号序列：高危
    "tSys_PrintTemplate", // 打印模板：走专用接口
    "tSys_TableColumnConfig", // 列配置：走 /api/permission/table-column-config/*
    "tSys_OperLog",       // 操作日志：只读，禁止改删
    "tSys_Rpt",           // 报表模板：高危
    // ===== P0-S3 补全：敏感系统表 =====
    "tBas_Emp",           // P0-S1：员工表含 PassWordStr 密码哈希，禁止通过 generic 读写
    "tSys_Parameters",    // 系统参数（业务规则）
    "tSys_Params",        // 系统参数别名
    "tSys_Config",        // 系统配置
    "tSys_Backup",        // 备份记录
    "tSys_Permission",   // 权限定义
    "tSys_OperHis",       // 操作历史（可被篡改/删除）
    "tSys_Company",      // 公司信息
    "tSys_UploadFile",    // 上传文件元数据
    "tSys_Notification",  // 通知（可被任意读取）
    "tSys_DataPack",      // 数据包
    "tSys_AutoMsg",       // 自动消息
    "tSys_AutoMsgRule",  // 自动消息规则
    "tSys_RulePermission", // 角色权限映射
    "tSys_RuleStock",    // 角色仓库权限
    "tSys_ITReport",     // IT 报表
    "tSys_RptPrintHis",  // 打印历史
    "tSys_RptPrintNum",  // 打印计数
    "tSys_Migration",    // 迁移记录（禁止用户层操作）
];

/// P0-S3 辅助：tSys_* 前缀的所有表默认视为敏感（除白名单显式放行的业务表外）
/// 这样后续新增的 tSys_* 表自动受保护，无需手动加入黑名单
fn is_system_prefix_table(table: &str) -> bool {
    let t = table.trim().to_lowercase();
    t.starts_with("tsys_")
}

/// P0-S3 辅助：tBas_Emp 也单独标记（不走 tSys_ 前缀但含密码哈希）
fn is_password_table(table: &str) -> bool {
    let t = table.trim().to_lowercase();
    t == "tbas_emp"
}

/// 校验表名是否在系统黑名单中（admin 用户除外）
///
/// 返回 true 表示拒绝，false 表示放行。
///
/// P0-S3 修复：除显式黑名单外，所有 tSys_* 前缀表和 tBas_Emp 也默认拒绝
///   双重防护：显式列表 + 前缀匹配，避免新增 tSys_* 表遗漏
fn is_table_blacklisted(table: &str, claims: &Claims) -> bool {
    // admin 超级权限放行
    if claims.user_code.eq_ignore_ascii_case("admin") {
        return false;
    }
    let table_trim = table.trim();
    // 1. 显式黑名单匹配
    if SYSTEM_TABLE_BLACKLIST.iter().any(|t| table_trim.eq_ignore_ascii_case(t)) {
        return true;
    }
    // 2. P0-S3：tSys_* 前缀的所有表默认拒绝（防止新增系统表遗漏）
    if is_system_prefix_table(table_trim) {
        return true;
    }
    // 3. P0-S1：tBas_Emp 单独拦截（含密码哈希）
    if is_password_table(table_trim) {
        return true;
    }
    false
}

/// 检查指定 EmpID 对应的员工是否是 admin（工号为 admin）
/// 用于保护 admin 账号不被删除/停用，避免系统锁死
async fn is_admin_employee(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    emp_id: &str,
) -> bool {
    if emp_id.is_empty() {
        return false;
    }
    let sql = "SELECT TOP 1 EmpNo FROM tBas_Emp WHERE EmpID = @p1";
    let v: &dyn tiberius::ToSql = &emp_id;
    if let Ok(stream) = conn.query(sql, &[v]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            if let Some(emp_no) = row.get::<&str, _>("EmpNo") {
                return emp_no.eq_ignore_ascii_case("admin");
            }
        }
    }
    false
}

/// P0-S4: 记录级越权防护
/// 对于业务单据表（含 EUser 字段），非 admin 用户只能更新/删除自己创建的记录。
/// 基础资料表（无 EUser 字段）不做此限制（由系统统一管控，所有用户可见可改）。
///
/// 返回 `Ok(())` 表示放行，`Err(msg)` 表示拒绝（msg 用于错误提示）。
///
/// 安全设计：
///   - admin 全放行（系统管理员）
///   - 旧 token 无 emp_id：放行（避免误伤历史 token；新登录会带 emp_id）
///   - 无 EUser 列的表：放行（基础资料表，非业务单据）
///   - EUser 为 NULL 的历史记录：放行（兼容迁移期数据）
///   - EUser 与当前用户 emp_id 不一致：拒绝
async fn check_record_ownership(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
    primary_key: &str,
    ids: &[String],
    claims: &Claims,
) -> std::result::Result<(), String> {
    // admin 放行
    if claims.user_code.eq_ignore_ascii_case("admin") {
        return Ok(());
    }
    if claims.emp_id.is_empty() || ids.is_empty() {
        return Ok(());
    }
    // 检查表是否有 EUser 列（INFORMATION_SCHEMA 查询，结果会被 SQL Server 缓存）
    let has_euser: bool = {
        let sql = "SELECT 1 FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @p1 AND COLUMN_NAME = 'EUser'";
        let v: &dyn tiberius::ToSql = &table;
        match conn.query(sql, &[v]).await {
            Ok(stream) => match stream.into_first_result().await {
                Ok(rows) => !rows.is_empty(),
                Err(_) => false,
            },
            Err(_) => false,
        }
    };
    if !has_euser {
        return Ok(()); // 基础资料表无 EUser，不做所有权校验
    }
    // ★ tStk_Shortage 是系统自动写入的缺货记录表，任何登录用户（采购员）都应能标记处理状态
    //   避免出现"只有上报人才能标记已处理"的权限死锁
    if table.eq_ignore_ascii_case("tStk_Shortage") {
        return Ok(());
    }
    // 批量校验：统计不属于当前用户的记录数
    // EUser IS NULL 视为历史数据放行（兼容迁移期），仅 EUser <> 当前用户 才拒绝
    let placeholders: Vec<String> = (0..ids.len())
        .map(|i| format!("@p{}", i + 1))
        .collect();
    let emp_param_idx = ids.len() + 1;
    let sql = format!(
        "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({}) AND ([EUser] IS NOT NULL AND [EUser] <> @p{})",
        table, primary_key,
        placeholders.join(", "),
        emp_param_idx
    );
    let mut params: Vec<&dyn tiberius::ToSql> = Vec::with_capacity(ids.len() + 1);
    for id in ids {
        params.push(id);
    }
    let emp_id = claims.emp_id.as_str();
    params.push(&emp_id);
    match conn.query(&sql, &params).await {
        Ok(stream) => match stream.into_first_result().await {
            Ok(rows) => {
                if let Some(row) = rows.first() {
                    let cnt: i64 = row.get::<i64, _>("cnt").unwrap_or(0);
                    if cnt > 0 {
                        return Err(format!(
                            "无权操作：{} 条记录不属于当前用户（创建人非本人），如需操作请联系创建人或管理员",
                            cnt
                        ));
                    }
                }
                Ok(())
            }
            Err(e) => Err(format!("记录权限校验失败: {}", e)),
        },
        Err(e) => Err(format!("记录权限校验失败: {}", e)),
    }
}

#[derive(Deserialize)]
pub struct WhereCondition {
    pub field: String,
    pub op: String,
    pub value: serde_json::Value,
}

#[derive(Deserialize)]
pub struct GenericQueryParams {
    pub table: String,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    // 兼容前端 camelCase（keywordFields）和 snake_case（keyword_fields）两种写法
    #[serde(alias = "keywordFields")]
    pub keyword_fields: Option<Vec<String>>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub wheres: Option<Vec<WhereCondition>>,
    pub include_deleted: Option<bool>,
    /// 仅显示已删除/已停用行（与 include_deleted 互斥）
    pub only_deleted: Option<bool>,
    /// 仓库 ID（查询 tBas_Goods 时 LEFT JOIN tStk_Stock 返回 StockQty/QQty）
    pub warehouse_id: Option<String>,
}

#[derive(Deserialize)]
pub struct GenericDeleteParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub state_field: Option<String>,
    /// true = 物理删除（DELETE FROM），false = 软删除（更新状态字段）
    pub permanent: Option<bool>,
    /// true = 作废（State='C'，业务作废保留可查），false/unset = 删除（State='D'，软删）
    pub void: Option<bool>,
}

#[derive(Deserialize)]
pub struct GenericCreateParams {
    pub table: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
pub struct GenericUpdateParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// 哪些列即使值为空也要以 '' 写入（NOT NULL 文本列，避免 '' 被误转 NULL）
fn default_empty_string_cols() -> std::collections::HashSet<String> {
    [
        "PHelp".to_string(),
        "PValue".to_string(),
        "CheckSQL".to_string(),
        "PTerm".to_string(),
    ].into_iter().collect()
}

/// 可清空的关联字段白名单（表名.列名，全部小写存储）。
/// 这些列允许用户通过通用 update 接口显式置为 NULL（如客户解绑定价模板），
/// 否则会被「防御性跳过 null/空值」逻辑忽略，导致无法解绑。
/// 仅 nullable 的 uniqueidentifier / 外键关联列才应加入此名单。
fn clearable_nullable_cols() -> std::collections::HashSet<String> {
    [
        "tbas_cust.pricingtemplateid".to_string(),
    ].into_iter().collect()
}

/// 判断 (table, column) 是否在可清空白名单中（大小写不敏感）
fn is_clearable_nullable(table: &str, col: &str) -> bool {
    let key = format!("{}.{}", table.to_lowercase(), col.to_lowercase());
    clearable_nullable_cols().contains(&key)
}

fn json_to_sql_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            // Treat empty/whitespace-only strings as NULL — SQL Server cannot
            // convert '' to uniqueidentifier / datetime / numeric, and any
            // nullable column should accept NULL gracefully.
            if s.trim().is_empty() { None } else { Some(s.clone()) }
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(if *b { "1".to_string() } else { "0".to_string() }),
        _ => Some(v.to_string()),
    }
}

/// 把 BIT 字段的值规范化为 SQL Server 可接受的 '1'/'0' 字符串。
/// SQL Server 的 bit 类型只接受 0/1/true/false，不接受 'Y'/'N' 字符串。
/// 前端 switch 控件可能传 boolean true/false、字符串 'Y'/'N'/'true'/'false'、数字 1/0，
/// 统一转成 '1'/'0' 字符串，由 json_to_sql_value 进一步处理。
fn normalize_bit_value(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_json::Value::Number(n) => {
            // 数字 0 → false，非 0 → true
            serde_json::Value::Bool(n.as_i64().map_or(false, |i| i != 0))
        }
        serde_json::Value::String(s) => {
            let lower = s.trim().to_lowercase();
            match lower.as_str() {
                "1" | "true" | "y" | "yes" | "on" => serde_json::Value::Bool(true),
                "0" | "false" | "n" | "no" | "off" | "" => serde_json::Value::Bool(false),
                _ => serde_json::Value::Bool(false), // 未知值默认 false
            }
        }
        serde_json::Value::Null => serde_json::Value::Null,
        _ => serde_json::Value::Bool(false),
    }
}

/// Returns the primary-key column name for a given table. The PK column
/// is treated specially by the generic insert/update logic (must be a
/// non-empty uniqueidentifier when present, and is auto-generated as a
/// fresh UUID when the client omits it or sends an empty string).
fn get_primary_key_for_table(table: &str) -> Option<&'static str> {
    match table {
        "tPur_Order" => Some("POID"),
        "tPur_OrderDetail" => Some("PODetailID"),
        "tPur_Inv" => Some("PIID"),
        "tPur_InvDetail" => Some("PIDetailID"),
        "tPur_Return" => Some("PRID"),
        "tPur_ReturnDetail" => Some("PRDetailID"),
        "tPur_Quote" => Some("PQID"),
        "tPur_AdjPrice" => Some("PAPID"),
        "tPur_AdjPriceDetail" => Some("PAPDetailID"),
        "tSal_Order" => Some("SOID"),
        "tSal_OrderDetail" => Some("SODetailID"),
        "tSal_Inv" => Some("SIID"),
        "tSal_InvDetail" => Some("SIDetailID"),
        "tSal_Quote" => Some("SQID"),
        "tSal_QuoteDetail" => Some("SQDetailID"),
        "tSal_AdjPrice" => Some("SAPID"),
        "tSal_AdjPriceDetail" => Some("SAPDetailID"),
        "tStk_IO" => Some("IOID"),
        "tStk_IODetail" => Some("IODetailID"),
        "tStk_Move" => Some("MoveID"),
        "tStk_MoveDetail" => Some("MoveDetailID"),
        "tStk_ReplenishApply" => Some("ReplenishApplyID"),
        "tStk_ReplenishApplyDetail" | "tStk_ReplenishApplyDtl" => Some("ReplenishApplyDtlID"),
        "tStk_StockCycle" => Some("CycleID"),
        "tStk_StockCycleDetail" => Some("CycleDetailID"),
        "tStk_Qty" => Some("QtyID"),
        "tBas_Goods" => Some("GDSID"),
        "tBas_Supp" => Some("SuppID"),
        "tBas_Cust" => Some("CustID"),
        "tBas_Emp" => Some("EmpID"),
        "tBas_Stock" => Some("StkID"),
        "tStk_Stock" => Some("GDSStockID"),
        "tBas_Dept" => Some("DeptID"),
        "tBas_Duty" => Some("DutyID"),
        "tBas_Brand" => Some("BrandID"),
        "tBas_GDSType" => Some("GDSTypeID"),
        "tBas_GDSProperty" => Some("GDSPropertyID"),
        "tBas_GDSKind" => Some("GDSKindID"),
        "tBas_DeaType" => Some("DeaTypeID"),
        "tBas_Unit" => Some("UnitID"),
        "tBas_SuppType" => Some("SuppTypeID"),
        "tBas_CustType" => Some("CustTypeID"),
        "tBas_Area" => Some("AreaID"),
        "tBas_Payment" => Some("PaymentID"),
        "tBas_CommTemplate" => Some("CommTplID"),
        "tBas_PriceTemplate" => Some("PriceTplID"),
        "tBas_EmpCommission" => Some("CommID"),
        "tBas_CustPrice" => Some("CustPriceID"),
        // tBas_CustPriceTac 是复合主键 (CustID, BrandID)，无单列 PK，走 commission_pricing 专用 API
        "tBas_CustPriceTac" => None,
        "tBas_Dictionary" | "tBas_Dict" => Some("DictID"),
        "tFin_Payment_TEST" => Some("PaymentID"),
        "tFin_Receipt_TEST" => Some("ReceiptID"),
        "tFin_Payment" => Some("PayID"),
        "tFin_PaymentDtl" => Some("PaymentDtlID"),
        "tFin_Receipt" => Some("RecID"),
        "tFin_ReceiptDtl" => Some("ReceiptDtlID"),
        "tFin_Payable" => Some("PayableID"),
        "tFin_Receivable" => Some("ReceivableID"),
        "tFin_CashFlow" => Some("CFID"),
        "tArd_AR" => Some("RowID"),
        "tSys_Rule" => Some("RuleID"),
        "tSys_Msg" => Some("MsgID"),
        "tSys_Parameters" | "tSys_Params" => Some("ParametersID"),
        "tSys_Rpt" => Some("RptID"),
        // tSys_RptPrintHis 复合主键 (DocID, PrintDate)，tSys_RptPrintNum 主键 DocID
        // 二者均为日志/计数表，走 print.rs 专用 API，不通过 generic CRUD 操作
        "tSys_RptPrintHis" => None,
        "tSys_RptPrintNum" => None,
        "tSys_Menus" => Some("MenuID"),
        "tSys_OperHis" => Some("OperHisID"),
        "tSys_DataPack" => Some("DataPackID"),
        "tSys_Notification" => Some("NotifyID"),
        "tSys_TableColumnConfig" => Some("ColumnConfigID"),
        "tSys_Config" => Some("ConfigID"),
        "tSys_UploadFile" => Some("FileID"),
        "tSys_Permission" => Some("PermissionID"),
        "tSys_PrintTemplate" => Some("TemplateID"),
        "tSys_Backup" => Some("BackupID"),
        "tSys_AutoMsg" => Some("AutoMsgID"),
        "tSys_AutoMsgRule" => Some("RuleID"),
        "tSys_Company" => Some("CompanyID"),
        "tSys_RuleMenu" => Some("RuleMenuID"),
        "tSys_UserRule" => Some("UserRuleID"),
        "tSys_RulePermission" => Some("RulePermID"),
        "tSys_RuleStock" => Some("RuleStockID"),
        "tOA_Notice" => Some("NoticeID"),
        "tOA_Workflow" => Some("WFID"),
        "tOA_Email" => Some("EmailID"),
        "tOnline_Goods" => Some("OnlineGDSID"),
        "tOnline_Order" => Some("OnlineOrderID"),
        "tOnline_OrderDetail" => Some("OnlineOrderDtlID"),
        "tOnline_Address" => Some("AddressID"),
        "tOnline_PaymentConfig" => Some("PaymentCfgID"),
        "tSal_VIP" => Some("VIPID"),
        "tSal_SaleTask" => Some("TaskID"),
        "tSal_EmpSales" => Some("ID"),
        "tStk_Tran" => Some("TranID"),
        "tStk_TranDetail" => Some("TranDetailID"),
        "tRpt_Custom" => Some("CustomRptID"),
        "tSys_ITReport" => Some("ITRptID"),
        "vStk_IOFlow" => Some("IODetailID"),
        _ => None,
    }
}

fn get_state_field_for_table(table: &str) -> Option<&'static str> {
    match table {
        "tBas_Brand" | "tBas_Stock" | "tBas_GDSType" | "tBas_GDSProperty" | "tBas_GDSKind"
        | "tBas_DeaType" | "tBas_Unit" | "tBas_SuppType" | "tBas_CustType" | "tBas_Area"
        | "tBas_Dept" | "tBas_Duty" | "tBas_Payment"
        | "tBas_CommTemplate" | "tBas_PriceTemplate"
        | "tSys_Menus" | "tSys_Config" => Some("Used"),
        "tBas_Goods" | "tBas_Supp" | "tBas_Cust" | "tBas_Emp"
        | "tPur_Order" | "tPur_Inv" | "tPur_Return" | "tPur_Quote" | "tPur_AdjPrice"
        | "tSal_Order" | "tSal_Inv" | "tSal_Return" | "tSal_Quote"
        | "tStk_IO" | "tStk_Move" | "tStk_Tran"
        | "tStk_ReplenishApply" | "tStk_StockCycle"
        | "tStk_Shortage"
        | "tFin_Receipt" | "tFin_Payment" | "tFin_CashFlow"
        | "tArd_PD" | "tAcc_PayOut" | "tAcc_PayIn"
        | "tSys_Rpt" | "tSys_Msg" | "tSys_DataPack"
        | "tSys_Rule" | "tSal_VIP" => Some("State"),
        "tSys_OperLog" | "tSys_OperHis" | "tSys_Dictionary" => None,
        // tStk_Stock / tStk_Qty 的 State 来自 tBas_Goods (别名 g)，由 build_conditions 单独处理
        "tStk_Qty" | "tStk_Stock" => Some("State"),
        _ => None,
    }
}

pub fn get_joins_for_table(table: &str) -> (String, String) {
    match table {
        // P0-S1 修复：tBas_Emp 不再用 t.*（会泄露 PassWordStr 密码哈希）
        //   改为显式列名，PassWordStr 不在列表中。即使 admin 走 generic_query 也不返回密码哈希
        // P0 修复：使用实际表结构中的列名（旧代码引用了 CardID/HireDate/LeaveDate/EmpState/
        //   Remark/LUser/LDate/SalEmpID/IDCard/HomeAddr 等不存在的列，导致 genericQuery 失败，
        //   进而使仓库页"业务员"下拉等所有 employee 选择器无法加载）
        "tBas_Emp" => (
            "t.[EmpID], t.[EmpNo], t.[EmpName], t.[Sex], t.[DeptID], t.[DutyID], t.[StkID], \
             t.[Tel], t.[IDCode], t.[LinkMan], t.[Birthday], t.[InDate], t.[OutDate], t.[WorkState], \
             t.[AllowLogin], t.[State], t.[Note], t.[EUser], t.[EDate], t.[AUser], t.[ADate], \
             t.[SUser], t.[SDate], t.[PYCode], t.[HomeTel], t.[Email], t.[IDAddr], \
             t.[ExLinkMan], t.[ExLinkTel], t.[ExLinkNex], t.[BilTel], t.[QQ], t.[OnlyLogin], \
             t.[BaseWagePrice], t.[BaseWageKind], t.[empSD], t.[LUTime], \
             d.[DeptName], du.[DutyName], s.[StkName], \
             eu.[EmpName] AS [EUserName]".to_string(),
            "LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Duty] du ON t.[DutyID] = du.[DutyID] \
             LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID]".to_string()
        ),
        "tBas_Supp" => (
            "t.*, st.[SuppTypeName], dt.[DeaTypeName], e.[EmpName], \
             eu.[EmpName] AS [EUserName]".to_string(),
            "LEFT JOIN [tBas_SuppType] st ON t.[SuppTypeID] = st.[SuppTypeID] \
             LEFT JOIN [tBas_DeaType] dt ON t.[DeaTypeID] = dt.[DeaTypeID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID]".to_string()
        ),
        "tBas_Cust" => (
            // pt.PName AS PricingTemplateName：客户绑定的定价模板名称（tSys_Parameters PKind='pricing'）
            "t.*, ct.[CustTypeName], a.[AreaName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], pt.[PName] AS [PricingTemplateName]".to_string(),
            "LEFT JOIN [tBas_CustType] ct ON t.[CustTypeID] = ct.[CustTypeID] \
             LEFT JOIN [tBas_Area] a ON t.[AreaID] = a.[AreaID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tSys_Parameters] pt ON t.[PricingTemplateID] = pt.[ParametersID] AND pt.[PKind] = 'pricing'".to_string()
        ),
        "tBas_Goods" => (
            // 性能优化：剥离前端未使用的 BrandABC/BrandNote 字段，减少传输和序列化
            // ★ tBas_Goods 表本身已有 GDSPropertyName 字段（冗余存储），不能再 SELECT gp.[GDSPropertyName]，
            //   否则外层 COUNT 派生表会因列名重复报错（code=8156）
            "t.*, gt.[GDSTypeName], b.[BrandName], gk.[GDSKindName], \
             dt.[DeaTypeName], s.[SuppName], u.[UnitName], sk.[StkName], \
             eu.[EmpName] AS [EUserName]".to_string(),
            "LEFT JOIN [tBas_GDSType] gt ON t.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_GDSProperty] gp ON t.[GDSPropertyID] = gp.[GDSPropertyID] \
             LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID] \
             LEFT JOIN [tBas_GDSKind] gk ON t.[GDSKindID] = gk.[GDSKindID] \
             LEFT JOIN [tBas_DeaType] dt ON t.[DeaTypeID] = dt.[DeaTypeID] \
             LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Unit] u ON t.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID]".to_string()
        ),
        "tBas_Stock" => (
            "t.*, e.[EmpName] AS [SalEmpName], \
             eu.[EmpName] AS [EUserName], \
             p.[StkName] AS [ParentStkName], \
             ct.[PName] AS [CommissionTemplateName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[SalEmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Stock] p ON t.[StkPID] = p.[StkID] \
             LEFT JOIN [tSys_Parameters] ct ON t.[CommissionTemplateID] = ct.[ParametersID]".to_string()
        ),
        "tPur_Order" => (
            "t.*, s.[SuppName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_IO" => (
            "t.*, s.[SuppName], c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tSal_Inv" => (
            "t.*, c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_Move" => (
            // 用 t.* 避免硬编码字段名错误（曾误把 tStk_IO 的 USID/DeptID/BTPID/SUser/SDate 写进 tStk_Move）
            "t.*, fs.[StkName] AS [FromStkName], ts.[StkName] AS [ToStkName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Stock] fs ON t.[FromStkID] = fs.[StkID] \
             LEFT JOIN [tBas_Stock] ts ON t.[ToStkID] = ts.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_ReplenishApply" => (
            "t.*, sk.[StkName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_Qty" => (
            "t.*, sk.[StkName], g.[GDSDesc], g.[GDSSpec], g.[GDSNO], g.[BarCode], \
             g.[AInPrice], g.[BPrice], g.[SPrice], g.[UnitNO], g.[WarnQty], \
             gt.[GDSTypeName], b.[BrandName], u.[UnitName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_EmpSales" => (
            "t.*, e.[EmpName], d.[DeptName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON e.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSys_Rpt" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RptPrintHis" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RptPrintNum" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Msg" => (
            "t.*, e.[EmpName] AS [ToUserName], fe.[EmpName] AS [FromUserName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[TEmpID] = e.[EmpID] LEFT JOIN [tBas_Emp] fe ON t.[FEmpID] = fe.[EmpID]".to_string()
        ),
        "tSys_Parameters" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_CustPriceTac" => (
            "t.*, c.[CustName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSys_DataPack" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tOnline_Goods" => (
            // tBas_Goods 实际字段名是 BarCode（不是 GDSBarCode）
            "t.*, g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO], g.[GDSSpec] AS [GoodsGDSSpec], g.[BarCode] AS [GoodsBarCode], s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tOnline_Order" => (
            "t.*, e.[EmpName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tOnline_OrderDetail" => (
            "t.*, g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID]".to_string()
        ),
        "tOnline_Address" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tOnline_PaymentConfig" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tAcc_PayOut" => (
            "t.*, s.[SuppName], e.[EmpName], d.[DeptName], k.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tAcc_PayIn" => (
            "t.*, c.[CustName], e.[EmpName], d.[DeptName], k.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tArd_PD" => (
            "t.*, s.[SuppName], d.[DeptName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID]".to_string()
        ),
        // tArd_AR 实际是手机端补货申请明细表（单表结构，无主从表分离）
        // 字段：RowID, StkID, EmpID, EDate, SaleDate, GDSID, Qty, Price, Amt, TelCode, ProvidersName, Used, SubscriberId
        // ★ SELECT g.PackCnvQty：直配单导入补货申请时需要带入包装量计算件数
        "tArd_AR" => (
            "t.*, sk.[StkName], e.[EmpName], g.[GDSNO] AS GoodsGDSNO, g.[GDSDesc] AS GoodsGDSDesc, g.[UnitNO] AS GoodsUnitNO, g.[PackCnvQty] AS PackCnvQty, u.[UnitName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tFin_Receivable" => (
            "t.*, c.[CustName], d.[DeptName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID]".to_string()
        ),
        // 付款单：关联供应商/业务员/部门/仓库
        "tFin_Payment" => (
            "t.*, s.[SuppName], e.[EmpName], d.[DeptName], k.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        // 收款单：关联客户/业务员/部门/仓库
        "tFin_Receipt" => (
            "t.*, c.[CustName], e.[EmpName], d.[DeptName], k.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] LEFT JOIN [tBas_Stock] k ON t.[StkID] = k.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tFin_CashFlow" => (
            "t.*, s.[SuppName], c.[CustName], e.[EmpName], d.[DeptName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tSys_RuleMenu" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_UserRule" => (
            "t.*, e.[EmpName], r.[RuleName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID]".to_string()
        ),
        "tSys_TableColumnConfig" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_UploadFile" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSal_Order" => (
            "t.*, c.[CustName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_StockCycle" => (
            "t.*, sk.[StkName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tSal_OrderDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_OrderDetail" => (
            // ★ tPur_OrderDetail 表本身已有 PackCnvQty/PackQty 字段，t.* 已包含，无需 JOIN
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        // 采购入库明细：同采购订单明细
        "tPur_InvDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        // 采购退货明细：同采购订单明细
        "tPur_ReturnDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_InvDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_IODetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote], io.[IONo], io.[Kind], io.[IoDate], io.[State] AS [IOState], io.[SuppID] AS [IOSuppID], io.[CustID] AS [IOCustID], io.[EmpID] AS [IOEmpID], io.[DeptID] AS [IODeptID], io.[StkID] AS [IOStkID], io.[Note] AS [IONote], s.[SuppName], c.[CustName], e.[EmpName], d.[DeptName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
             LEFT JOIN [tStk_IO] io ON t.[IOID] = io.[IOID] \
             LEFT JOIN [tBas_Supp] s ON io.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Cust] c ON io.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Emp] e ON io.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON io.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Stock] sk ON io.[StkID] = sk.[StkID]".to_string()
        ),
        "tStk_MoveDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_ReplenishApplyDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_ReplenishApplyDtl" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_StockCycleDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tStk_TranDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tPur_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_QuoteDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_AdjPriceDetail" => (
            "t.*, ISNULL(NULLIF(t.[GDSNO], ''), g.[GDSNO]) AS [GoodsGDSNO], ISNULL(NULLIF(t.[GDSDesc], ''), g.[GDSDesc]) AS [GoodsGDSDesc], u.[UnitName], g.[PackCnvQty] AS [PackCnvQty], g.[BrandID], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID]".to_string()
        ),
        "tBas_EmpCommission" => (
            "t.*, e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tBas_CustPrice" => (
            "t.*, c.[CustName], g.[GDSDesc] AS [GoodsGDSDesc], g.[GDSNO] AS [GoodsGDSNO]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID]".to_string()
        ),
        "tSal_SaleTask" => (
            "t.*, e.[EmpName], sk.[StkName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID]".to_string()
        ),
        "tSys_Rule" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_CommTemplate" => (
            "t.*, s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "tBas_PriceTemplate" => (
            "t.*, c.[CustName], b.[BrandName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Brand] b ON t.[BrandID] = b.[BrandID]".to_string()
        ),
        "tSal_VIP" => (
            "t.*, s.[StkName]".to_string(),
            "LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        // tSys_OperLog 实际字段为 EmpID（audit_log.rs 中 INSERT 使用 EmpID），LEFT JOIN 需用 EmpID
        "tSys_OperLog" => (
            "t.*, e.[EmpName] AS [OperatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        // tSys_OperHis 实际字段为 EmpID（不是 OperatorID），LEFT JOIN 需用 EmpID
        "tSys_OperHis" => (
            "t.*, e.[EmpName] AS [OperatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        "tSys_Menus" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Dictionary_DISABLED" => ( // tSys_Dictionary 表不存在
            "t.*".to_string(),
            "".to_string()
        ),
        "tPur_Inv" => (
            "t.*, s.[SuppName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        // 采购退货：同采购入库，关联供应商/部门/业务员/仓库
        "tPur_Return" => (
            "t.*, s.[SuppName], d.[DeptName], e.[EmpName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tPur_Quote" => (
            "t.*, s.[SuppName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Supp] s ON t.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tPur_AdjPrice" => (
            "t.*, e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tSal_Quote" => (
            "t.*, c.[CustName], e.[EmpName], d.[DeptName], sk.[StkName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Cust] c ON t.[CustID] = c.[CustID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tSal_AdjPrice" => (
            "t.*, e.[EmpName], d.[DeptName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Dept] d ON t.[DeptID] = d.[DeptID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_Tran" => (
            "t.*, sk.[StkName], e.[EmpName], \
             eu.[EmpName] AS [EUserName], au.[EmpName] AS [AUserName], su.[EmpName] AS [SUserName]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID] \
             LEFT JOIN [tBas_Emp] eu ON t.[EUser] = eu.[EmpID] \
             LEFT JOIN [tBas_Emp] au ON t.[AUser] = au.[EmpID] \
             LEFT JOIN [tBas_Emp] su ON t.[SUser] = su.[EmpID]".to_string()
        ),
        "tStk_Stock" => (
            "t.*, sk.[StkName], sk.[StkCode], sk.[IsDefault], sk.[Used] AS [StkUsed], \
             g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], \
             g.[AInPrice], g.[BPrice], g.[SPrice], g.[UnitNO], g.[TopStkQty], g.[BttomStkQty], g.[PackCnvQty], g.[GDSStateNO], g.[State], \
             g.[GDSTypeID], g.[BrandID], g.[SuppID], g.[StkID] AS [GoodsStkID], \
             gt.[GDSTypeName], b.[BrandName], b.[BrandABC], b.[Note] AS [BrandNote], u.[UnitName], s.[SuppName], \
             gsk.[StkName] AS [GoodsStkName], \
             (ISNULL(t.[Qty],0) * ISNULL(g.[AInPrice],0)) AS [StockAmt]".to_string(),
            "LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Stock] gsk ON g.[StkID] = gsk.[StkID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
             LEFT JOIN [tBas_Supp] s ON g.[SuppID] = s.[SuppID]".to_string()
        ),
        // tStk_Shortage：缺货记录（保存/审核单据时库存不足自动写入）
        // 关联 tBas_Goods 显示商品信息（编码/名称/规格/条码/品牌/分类/单位/供应商/包装量/成本价）
        // 关联 tBas_Stock 显示仓库名称，关联 tBas_Emp 显示上报人姓名
        // 实时当前库存用子查询读取 tStk_Stock.Qty（避免 LEFT JOIN 产生笛卡尔积）
        "tStk_Shortage" => (
            "t.*, sk.[StkName], sk.[StkCode], \
             g.[GDSNO], g.[GDSDesc], g.[GDSSpec], g.[BarCode], g.[GDSStateNO], \
             g.[GDSTypeID], g.[BrandID], g.[SuppID], g.[UnitNO], g.[PackCnvQty], g.[AInPrice], \
             g.[TopStkQty], g.[BttomStkQty], \
             gt.[GDSTypeName], b.[BrandName], u.[UnitName], s.[SuppName], \
             eu.[EmpName] AS [EUserName], \
             (ISNULL(t.[ShortQty],0) * ISNULL(g.[AInPrice],0)) AS [ShortAmt], \
             cur.[Qty] AS [CurStockQty], \
             (cur.[Qty] - ISNULL(cur.[QQty],0)) AS [CurAvailableQty]".to_string(),
            "LEFT JOIN [tBas_Goods] g ON t.[GDSID] = g.[GDSID] \
             LEFT JOIN [tBas_Stock] sk ON t.[StkID] = sk.[StkID] \
             LEFT JOIN [tBas_GDSType] gt ON g.[GDSTypeID] = gt.[GDSTypeID] \
             LEFT JOIN [tBas_Brand] b ON g.[BrandID] = b.[BrandID] \
             LEFT JOIN [tBas_Supp] s ON g.[SuppID] = s.[SuppID] \
             LEFT JOIN [tBas_Unit] u ON g.[UnitNO] = u.[UnitNO] \
             LEFT JOIN [tBas_Emp] eu ON t.[EmpID] = eu.[EmpID] \
             LEFT JOIN [tStk_Stock] cur ON cur.[GDSID] = t.[GDSID] AND cur.[StkID] = t.[StkID]".to_string()
        ),
        "tOA_Notice" => (
            "t.*, e.[EmpName] AS [CreatorName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[CreatorID] = e.[EmpID]".to_string()
        ),
        "tOA_Workflow" => (
            "t.*, ce.[EmpName] AS [CreatorName], ae.[EmpName] AS [ApproverName]".to_string(),
            "LEFT JOIN [tBas_Emp] ce ON t.[CreatorID] = ce.[EmpID] \
             LEFT JOIN [tBas_Emp] ae ON t.[ApproverID] = ae.[EmpID]".to_string()
        ),
        "tOA_Email" => (
            "t.*, e.[EmpName] AS [SenderName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[SenderID] = e.[EmpID]".to_string()
        ),
        "tSys_Params" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_ITReport" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tBas_Dict" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tRpt_Custom" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Permission" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_RulePermission" => (
            "t.*, r.[RuleName], p.[PermName]".to_string(),
            "LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID] \
             LEFT JOIN [tSys_Permission] p ON t.[PermissionID] = p.[PermissionID]".to_string()
        ),
        "tSys_RuleStock" => (
            "t.*, r.[RuleName], s.[StkName]".to_string(),
            "LEFT JOIN [tSys_Rule] r ON t.[RuleID] = r.[RuleID] \
             LEFT JOIN [tBas_Stock] s ON t.[StkID] = s.[StkID]".to_string()
        ),
        "vStk_IOFlow" => (
            "t.*".to_string(),
            "".to_string()
        ),
        "tSys_Notification" => (
            "t.*, e.[EmpName] AS [EmpName]".to_string(),
            "LEFT JOIN [tBas_Emp] e ON t.[EmpID] = e.[EmpID]".to_string()
        ),
        _ => (
            "t.*".to_string(),
            "".to_string()
        ),
    }
}

struct BuiltQuery {
    sql: String,
    params: Vec<Option<String>>,
}

fn get_field_prefix_for_table<'a>(table: &str, field: &str) -> &'a str {
    match table {
        "tBas_Goods" => {
            match field {
                "GDSTypeName" | "GDSTypeID" => "gt",
                "GDSKindName" => "gk",
                "BrandName" | "BrandABC" | "BrandNote" | "BrandID" => "b",
                "DeaTypeName" | "DeaTypeID" => "dt",
                "SuppName" | "SuppID" => "s",
                "UnitName" | "UnitNO" => "u",
                "StkName" | "StkID" => "sk",
                "GDSPropertyName" | "GDSPropertyID" => "gp",
                "EUserName" | "EUser" => "eu",
                _ => "t",
            }
        }
        "tBas_Stock" => {
            match field {
                "SalEmpName" | "SalEmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "ParentStkName" | "StkPID" => "p",
                _ => "t",
            }
        }
        "tStk_Qty" | "tStk_Stock" => {
            match field {
                "GDSNO" | "GDSDesc" | "GDSSpec" | "BarCode" | "GDSTypeID" | "BrandID" | "SuppID"
                | "UnitNO" | "AInPrice" | "BPrice" | "SPrice" | "VPrice" | "CPrice" | "WarnQty"
                | "GDSStateNO" | "State" | "TopStkQty" | "BttomStkQty" | "PackCnvQty" => "g",
                "StkName" | "StkCode" | "IsDefault" | "Used" | "NodeKind" => "sk",
                // 商品资料的默认仓库（g.StkID JOIN tBas_Stock gsk）
                "GoodsStkID" | "GoodsStkName" => "gsk",
                "GDSTypeName" => "gt",
                "BrandName" | "BrandABC" | "BrandNote" => "b",
                "UnitName" => "u",
                "SuppName" => "s",
                _ => "t",
            }
        }
        // tStk_Shortage：缺货记录，关联 tBas_Goods/sk/eu，搜索/排序字段路由到对应别名
        "tStk_Shortage" => {
            match field {
                "GDSNO" | "GDSDesc" | "GDSSpec" | "BarCode" | "GDSTypeID" | "BrandID" | "SuppID"
                | "UnitNO" | "AInPrice" | "GDSStateNO" | "TopStkQty" | "BttomStkQty" | "PackCnvQty" => "g",
                "StkName" | "StkCode" => "sk",
                "GDSTypeName" => "gt",
                "BrandName" => "b",
                "UnitName" => "u",
                "SuppName" => "s",
                "EUserName" => "eu",
                _ => "t",
            }
        }
        "tPur_OrderDetail" | "tSal_OrderDetail" | "tSal_InvDetail" | "tStk_IODetail"
        | "tStk_MoveDetail" | "tStk_ReplenishApplyDetail" | "tStk_ReplenishApplyDtl"
        | "tStk_StockCycleDetail" | "tStk_TranDetail" | "tPur_QuoteDetail"
        | "tPur_AdjPriceDetail" | "tSal_QuoteDetail" | "tSal_AdjPriceDetail" => {
            match field {
                "GDSNO" | "GDSDesc" | "GoodsGDSNO" | "GoodsGDSDesc" | "GDSSpec" | "BarCode"
                | "GDSTypeID" | "BrandID" | "SuppID" | "UnitNO" | "AInPrice" | "BPrice"
                | "SPrice" | "VPrice" | "CPrice" | "WarnQty" | "GDSStateNO" | "State" => "g",
                "UnitName" => "u",
                "BrandName" | "BrandABC" | "BrandNote" => "b",
                "IONo" | "Kind" | "IoDate" | "IOState" | "IOSuppID" | "IOCustID" | "IOEmpID"
                | "IODeptID" | "IOStkID" | "IONote" => "io",
                "SuppName" => "s",
                "CustName" => "c",
                "EmpName" => "e",
                "DeptName" => "d",
                "StkName" => "sk",
                _ => "t",
            }
        }
        "tArd_AR" => {
            match field {
                "StkName" => "sk",
                "EmpName" => "e",
                "GoodsGDSNO" | "GoodsGDSDesc" | "GoodsUnitNO" => "g",
                "UnitName" => "u",
                "BrandName" => "b",
                _ => "t",
            }
        }
        // 基础资料表：EUser → eu 别名（创建人姓名搜索路由）
        "tBas_Emp" | "tBas_Supp" | "tBas_Cust" => {
            match field {
                "EUserName" | "EUser" => "eu",
                _ => "t",
            }
        }
        // 单据主表：keyword 搜索路由到 LEFT JOIN 的关联表别名，
        // 使关键词能搜到供应商/客户/部门/业务员/仓库名称（JOIN 出来的实时值，非主表冗余列）。
        // 主表自带字段（单号、Note、EUser 等）回退到 "t"。
        // 采购类（tPur_Order/tPur_Inv）：JOIN s/d/e/sk
        "tPur_Order" | "tPur_Inv" | "tPur_Return" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "sk",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 采购报价：JOIN s/e
        "tPur_Quote" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 采购调价：JOIN e
        "tPur_AdjPrice" => {
            match field {
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 销售类（tSal_Order/tSal_Inv）：JOIN c/d/e/sk
        "tSal_Order" | "tSal_Inv" => {
            match field {
                "CustName" | "CustID" => "c",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "sk",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 销售报价：JOIN c/e/d/sk
        "tSal_Quote" => {
            match field {
                "CustName" | "CustID" => "c",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "sk",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 销售调价：JOIN e/d
        "tSal_AdjPrice" => {
            match field {
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 入出库单（tStk_IO）：JOIN s/c/d/e/sk
        "tStk_IO" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "CustName" | "CustID" => "c",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "sk",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 调拨单（tStk_Move）：JOIN fs/ts/e
        "tStk_Move" => {
            match field {
                "FromStkName" | "FromStkID" => "fs",
                "ToStkName" | "ToStkID" => "ts",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 补货申请 / 周期盘点：JOIN sk/e
        "tStk_ReplenishApply" | "tStk_StockCycle" => {
            match field {
                "StkName" | "StkID" => "sk",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 库存调整（tStk_Tran）：JOIN sk/e
        "tStk_Tran" => {
            match field {
                "StkName" | "StkID" => "sk",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 付款单（tAcc_PayOut）：JOIN s/e/d/k（注意仓库别名是 k 不是 sk）
        "tAcc_PayOut" | "tFin_Payment" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "k",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 收款单（tAcc_PayIn）：JOIN c/e/d/k
        "tAcc_PayIn" | "tFin_Receipt" => {
            match field {
                "CustName" | "CustID" => "c",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "StkName" | "StkID" => "k",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 现金流量（tFin_CashFlow）：JOIN s/c/e/d
        "tFin_CashFlow" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "CustName" | "CustID" => "c",
                "DeptName" | "DeptID" => "d",
                "EmpName" | "EmpID" => "e",
                "EUserName" | "EUser" => "eu",
                "AUserName" | "AUser" => "au",
                "SUserName" | "SUser" => "su",
                _ => "t",
            }
        }
        // 应付账款（tArd_PD）：JOIN s/d
        "tArd_PD" => {
            match field {
                "SuppName" | "SuppID" => "s",
                "DeptName" | "DeptID" => "d",
                _ => "t",
            }
        }
        _ => "t",
    }
}

/// keyword 搜索时，把前端传来的"显示别名"映射为 JOIN 表中的真实列名。
/// 例如 tStk_Move 的 keywordFields 包含 "FromStkName"，但 SELECT 中是
/// `fs.[StkName] AS [FromStkName]`，WHERE 子句不能用别名，必须用 `fs.[StkName]`。
/// 未在映射表中的字段，回退为原 field 名（适用主表自有字段，如 MoveNO/Note/EUser）。
fn get_real_column_for_keyword<'a>(table: &str, field: &'a str) -> &'a str {
    match table {
        "tStk_Move" => match field {
            "FromStkName" => "StkName",
            "ToStkName" => "StkName",
            _ => field,
        },
        // 其他表的 JOIN 字段名与 SELECT 别名一致，无需映射
        _ => field,
    }
}

/// Returns the list of field names that come from JOIN (not from the main table).
/// These fields should be excluded from INSERT/UPDATE to avoid overwriting
/// the main table's own redundant Name columns with wrong data.
fn get_join_fields_for_table(table: &str) -> Vec<&'static str> {
    match table {
        "tBas_Goods" => vec!["GDSTypeName", "GDSPropertyName", "BrandName", "BrandABC", "BrandNote", "GDSKindName", "DeaTypeName", "SuppName", "UnitName", "StkName", "EUserName"],
        "tBas_Supp" => vec!["SuppTypeName", "DeaTypeName", "EmpName", "EUserName"],
        "tBas_Cust" => vec!["CustTypeName", "AreaName", "EmpName", "EUserName"],
        "tBas_Emp" => vec!["DeptName", "DutyName", "StkName", "EUserName"],
        "tBas_Stock" => vec!["SalEmpName", "EUserName", "ParentStkName"],
        "tPur_Order" | "tPur_Inv" | "tPur_Return" => vec!["SuppName", "DeptName", "EmpName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tPur_Quote" => vec!["SuppName", "EmpName", "EUserName", "AUserName", "SUserName"],
        "tPur_AdjPrice" => vec!["EmpName", "EUserName", "AUserName", "SUserName"],
        "tSal_Order" | "tSal_Inv" => vec!["CustName", "DeptName", "EmpName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tSal_Quote" => vec!["CustName", "EmpName", "DeptName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tSal_AdjPrice" => vec!["EmpName", "DeptName", "EUserName", "AUserName", "SUserName"],
        "tStk_IO" => vec!["SuppName", "CustName", "DeptName", "EmpName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tStk_Move" => vec!["FromStkName", "ToStkName", "EmpName", "EUserName", "AUserName", "SUserName"],
        "tStk_ReplenishApply" | "tStk_StockCycle" => vec!["StkName", "EmpName", "EUserName", "AUserName", "SUserName"],
        "tStk_Qty" => vec!["StkName", "GDSDesc", "GDSSpec", "GDSNO", "BarCode", "AInPrice", "BPrice", "SPrice", "UnitNO", "GDSTypeName", "BrandName", "BrandABC", "BrandNote", "UnitName", "WarnQty"],
        "tStk_Stock" => vec!["StkName", "StkCode", "GDSNO", "GDSDesc", "GDSSpec", "BarCode", "AInPrice", "BPrice", "SPrice", "UnitNO", "WarnQty", "GDSStateNO", "State", "GDSTypeID", "BrandID", "SuppID", "GDSTypeName", "BrandName", "BrandABC", "BrandNote", "UnitName", "SuppName"],
        "tAcc_PayOut" => vec!["SuppName", "EmpName", "DeptName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tAcc_PayIn" => vec!["CustName", "EmpName", "DeptName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tFin_Payment" => vec!["SuppName", "EmpName", "DeptName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tFin_Receipt" => vec!["CustName", "EmpName", "DeptName", "StkName", "EUserName", "AUserName", "SUserName"],
        "tArd_PD" => vec!["SuppName", "DeptName"],
        "tArd_AR" => vec!["StkName", "EmpName", "GoodsGDSNO", "GoodsGDSDesc", "GoodsUnitNO", "UnitName", "BrandName"],
        "tFin_CashFlow" => vec!["SuppName", "CustName", "EmpName", "DeptName", "EUserName", "AUserName", "SUserName"],
        "tSys_OperHis" => vec!["OperatorName"], // tSys_OperLog 不存在
        "tSys_Msg" => vec!["ToUserName", "FromUserName"],
        "tSys_UserRule" => vec!["EmpName", "RuleName"],
        "tSys_RulePermission" => vec!["RuleName", "PermName"],
        "tSys_RuleStock" => vec!["RuleName", "StkName"],
        "tBas_CustPriceTac" => vec!["CustName", "BrandName"],
        "tBas_PriceTemplate" => vec!["CustName", "BrandName"],
        "tBas_CommTemplate" => vec!["StkName"],
        "tBas_EmpCommission" | "tSal_SaleTask" => vec!["EmpName", "StkName"],
        "tBas_CustPrice" => vec!["CustName", "GDSDesc", "GDSNO"],
        "tSal_EmpSales" => vec!["DeptName", "BrandName"],
        "tSal_VIP" => vec!["StkName"],
        "tStk_Tran" => vec!["StkName", "EmpName", "EUserName", "AUserName", "SUserName"],
        // 白名单使用 SQL 返回给前端的字段名（含 AS 别名），不是数据库原字段名
        "tOnline_Goods" => vec!["GoodsGDSDesc", "GoodsGDSNO", "GoodsGDSSpec", "GoodsBarCode", "StkName"],
        "tOnline_Order" => vec!["EmpName"],
        "tOnline_OrderDetail" => vec!["GoodsGDSDesc", "GoodsGDSNO"],
        "tPur_OrderDetail" | "tSal_OrderDetail" | "tSal_InvDetail" | "tStk_IODetail" | "tStk_MoveDetail"
        | "tStk_ReplenishApplyDetail" | "tStk_StockCycleDetail" | "tPur_QuoteDetail"
        | "tPur_AdjPriceDetail" | "tSal_AdjPriceDetail" | "tSal_QuoteDetail" | "tStk_ReplenishApplyDtl"
        | "tStk_TranDetail" => vec!["GDSDesc", "GDSNO", "GDSSpec", "UnitName", "BrandID", "BrandName", "BrandABC", "BrandNote", "GoodsGDSNO", "GoodsGDSDesc", "IONo", "Kind", "IoDate", "IOState", "IOSuppID", "IOCustID", "IOEmpID", "IODeptID", "IOStkID", "IONote", "SuppName", "CustName", "EmpName", "DeptName", "StkName"],
        "tOA_Notice" => vec!["CreatorName"],
        "tOA_Workflow" => vec!["CreatorName", "ApproverName"],
        "tOA_Email" => vec!["SenderName"],
        _ => vec![],
    }
}

#[cfg(test)]
fn get_identity_columns_for_table(table: &str) -> Vec<&'static str> {
    match table {
        "tBas_Goods" => vec!["gdsSD"],
        "tBas_Supp" => vec!["suppSD"],
        "tBas_Cust" => vec!["custSD"],
        "tBas_Emp" => vec!["empSD"],
        "tBas_Stock" => vec!["stkSD"],
        _ => vec![],
    }
}

/// Returns the list of column names that cannot be set in INSERT/UPDATE
/// (IDENTITY columns and computed columns). The list is fetched live from
/// `sys.columns` so it works for any table — no hard-coding required.
async fn fetch_readonly_columns(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
) -> std::collections::HashSet<String> {
    let mut result: std::collections::HashSet<String> = std::collections::HashSet::new();
    // SQL Server identifier safety: brackets only — table comes from the
    // request body so we must guard against injection.
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return result;
    }
    let sql = format!(
        "SELECT name FROM sys.columns \
         WHERE object_id = OBJECT_ID('[{}]') \
           AND (is_identity = 1 OR is_computed = 1)",
        table
    );
    if let Ok(rows) = conn.query(&sql, &[]).await {
        match rows.into_first_result().await {
            Ok(vec) => {
                for row in vec {
                    let name: Option<&str> = row.try_get("name").ok().flatten();
                    if let Some(n) = name {
                        // 统一存为小写，避免 tiberius 返回的列名大小写与
                        // sys.columns.name 不一致时比较失败
                        result.insert(n.to_lowercase());
                    }
                }
            }
            Err(_) => {}
        }
    }
    result
}

/// Returns the list of BIT column names for a given table.
/// Used by generic_update/generic_create to convert 'Y'/'N' strings to 1/0
/// for BIT columns — SQL Server rejects nvarchar 'N' → bit conversion.
/// The list is fetched live from `sys.columns` so it works for any table.
async fn fetch_bit_columns(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
) -> std::collections::HashSet<String> {
    let mut result: std::collections::HashSet<String> = std::collections::HashSet::new();
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return result;
    }
    // system_type_id = 104 是 bit 类型（sys.types.user_type_id 对应 sys.columns.system_type_id）
    let sql = format!(
        "SELECT name FROM sys.columns \
         WHERE object_id = OBJECT_ID('[{}]') \
           AND system_type_id = 104",
        table
    );
    if let Ok(rows) = conn.query(&sql, &[]).await {
        match rows.into_first_result().await {
            Ok(vec) => {
                for row in vec {
                    let name: Option<&str> = row.try_get("name").ok().flatten();
                    if let Some(n) = name {
                        result.insert(n.to_lowercase());
                    }
                }
            }
            Err(_) => {}
        }
    }
    result
}

/// 检查表是否存在指定列（不区分大小写）
async fn has_column(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
    column: &str,
) -> bool {
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let sql = format!(
        "SELECT 1 FROM sys.columns WHERE object_id = OBJECT_ID('[{}]') AND name = @p1",
        table
    );
    if let Ok(rows) = conn.query(&sql, &[&column]).await {
        if let Ok(vec) = rows.into_first_result().await {
            return !vec.is_empty();
        }
    }
    false
}

/// 根据员工编号（EmpNo）查找其 UUID（用于自动填充 EUser 等审计字段）
async fn lookup_user_uuid(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    emp_no: &str,
) -> Option<String> {
    if emp_no.is_empty() {
        return None;
    }
    let sql = "SELECT TOP 1 CAST(EmpID AS NVARCHAR(36)) AS EmpID FROM tBas_Emp WHERE EmpNo = @p1";
    if let Ok(rows) = conn.query(sql, &[&emp_no]).await {
        if let Ok(vec) = rows.into_first_result().await {
            if let Some(row) = vec.into_iter().next() {
                let id: Option<&str> = row.try_get("EmpID").ok().flatten();
                if let Some(s) = id {
                    let s = s.trim();
                    if !s.is_empty() {
                        return Some(s.to_string());
                    }
                }
            }
        }
    }
    None
}

/// 根据员工编号查找其 UUID（跨模块公开入口，自行获取连接）
/// 供 handlers::print 等模块在无显式 conn 的调用点使用
pub async fn cached_lookup_user_uuid(emp_no: &str) -> Option<String> {
    if emp_no.is_empty() {
        return None;
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(_) => return None,
    };
    lookup_user_uuid(&mut conn, emp_no).await
}

/// 校验 SQL 标识符（表名/字段名）：仅允许字母、数字、下划线，禁止 ] 空格 ; -- 等
/// 返回 true 表示合法
fn is_valid_identifier(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// P2-19 修复：敏感字段黑名单
///   原仅校验字段名是否合法标识符，未限制敏感字段
///   攻击者可通过 wheres 探测 PassWordStr/Salt/JWT_SECRET 等字段是否存在
///   黑名单覆盖密码、密钥、令牌、内部审计字段等
fn is_sensitive_field(field: &str) -> bool {
    let f = field.to_ascii_lowercase();
    const SENSITIVE_PATTERNS: &[&str] = &[
        "password", "passwd", "pwd", "pwdstr",
        "secret", "jwt_secret", "api_key", "apikey",
        "salt", "token", "accesstoken", "refreshtoken",
        "private_key", "privatekey",
    ];
    SENSITIVE_PATTERNS.iter().any(|p| f.contains(p))
}

/// 校验查询参数的表名、keyword_fields、wheres.field，防止 SQL 注入。
fn validate_query_params(table: &str, keyword_fields: &Option<Vec<String>>, wheres: &Option<Vec<WhereCondition>>) -> std::result::Result<(), String> {
    if table.is_empty() { return Err(String::from("表名不能为空")); }
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err(String::from("表名只能包含字母、数字、下划线")); }
    if let Some(fields) = keyword_fields {
        for f in fields {
            if !is_valid_identifier(f) { return Err(format!("非法字段名: {}", f)); }
            // P2-19：禁止查询敏感字段（防止通过 keyword 搜索探测密码字段值）
            if is_sensitive_field(f) { return Err(format!("字段 {} 不允许查询", f)); }
        }
    }
    if let Some(wc_list) = wheres {
        for wc in wc_list {
            if !is_valid_identifier(&wc.field) { return Err(format!("非法字段名: {}", wc.field)); }
            if is_sensitive_field(&wc.field) { return Err(format!("字段 {} 不允许查询", wc.field)); }
        }
    }
    Ok(())
}

/// 校验 primary_key / state_field 等标识符参数，防止 SQL 注入。
fn validate_identifiers(idents: &[&str]) -> std::result::Result<(), String> {
    for s in idents {
        if s.is_empty() { return Err(String::from("主键/状态字段名不能为空")); }
        if !is_valid_identifier(s) { return Err(format!("非法字段名: {}", s)); }
    }
    Ok(())
}

fn build_base_query(
    table: &str,
    keyword: &Option<String>,
    keyword_fields: &Option<Vec<String>>,
    wheres: &Option<Vec<WhereCondition>>,
    include_deleted: bool,
    only_deleted: bool,
    warehouse_id: &Option<String>,
) -> BuiltQuery {
    let mut conditions = Vec::new();
    let mut params: Vec<Option<String>> = Vec::new();
    let mut param_idx = 1;

    if only_deleted {
        // 只显示已删除/已停用行
        if let Some(state_field) = get_state_field_for_table(table) {
            // tStk_Stock / tStk_Qty 的 State 来自 tBas_Goods (别名 g)
            let prefix = if table == "tStk_Stock" || table == "tStk_Qty" { "g" } else { "t" };
            match state_field {
                "Used" => {
                    conditions.push("t.[Used] = 'N'".to_string());
                }
                _ => {
                    conditions.push(format!("{}.[State] = 'D'", prefix));
                }
            }
        }
    } else if !include_deleted {
        if let Some(state_field) = get_state_field_for_table(table) {
            // tStk_Stock / tStk_Qty 的 State 来自 tBas_Goods (别名 g)
            let prefix = if table == "tStk_Stock" || table == "tStk_Qty" { "g" } else { "t" };
            match state_field {
                "Used" => {
                    conditions.push("t.[Used] <> 'N'".to_string());
                }
                _ => {
                    // ★ State=NULL 的记录也要显示（SQL Server 中 NULL <> 'D' 结果为 NULL，会被过滤）
                    //   否则会出现"编码已存在但查不到"的脏数据问题（唯一索引仍拦截 INSERT）
                    conditions.push(format!("({}.[State] <> 'D' OR {}.[State] IS NULL)", prefix, prefix));
                }
            }
        }
    }

    // ★ 过滤孤儿库存记录：tStk_Stock / tStk_Qty 中 GDSID 为零 UUID 或 NULL 的记录是脏数据
    //   （LEFT JOIN tBas_Goods 后 g.* 全为 NULL，前端显示为空行）
    //   数据库已删除历史脏数据，此处加防御性过滤防止再次出现
    if table == "tStk_Stock" || table == "tStk_Qty" {
        conditions.push("(t.[GDSID] IS NOT NULL AND t.[GDSID] <> '00000000-0000-0000-0000-000000000000')".to_string());
    }

    if let Some(kw) = keyword {
        if !kw.is_empty() {
            if let Some(fields) = keyword_fields {
                if !fields.is_empty() {
                    let kw_conditions: Vec<String> = fields.iter()
                        .filter(|f| is_valid_identifier(f))
                        .map(|f| {
                            let pidx = param_idx;
                            param_idx += 1;
                            params.push(Some(format!("%{}%", kw)));
                            let prefix = get_field_prefix_for_table(table, f);
                            // keyword 搜索时使用 JOIN 表的真实列名（如 fs.[StkName]），不能用 SELECT 别名（如 fs.[FromStkName]）
                            let real_col = get_real_column_for_keyword(table, f);
                            format!("CAST({}.[{}] AS varchar(max)) LIKE @p{}", prefix, real_col, pidx)
                        })
                        .collect();
                    if !kw_conditions.is_empty() {
                        conditions.push(format!("({})", kw_conditions.join(" OR ")));
                    }
                }
            }
        }
    }

    if let Some(wc_list) = wheres {
        for wc in wc_list {
            // 防御性校验：跳过非法字段名，避免 SQL 注入
            if !is_valid_identifier(&wc.field) {
                continue;
            }

            // ★ 库存预警筛选：WarnType 是虚拟字段，生成字段间比较的 SQL
            //   注意：Qty 来自 t（tStk_Stock），TopStkQty/BttomStkQty 来自 g（tBas_Goods）
            //   用户要求：只分"低于下限"和"超过上限"两类，零库存归入低于下限（0 < 下限）
            //   all_goods = 全部商品（不应用预警条件，仅依赖 fixedWheres 过滤品态）
            //   high      = 超过上限（Qty > TopStkQty 且 TopStkQty > 0）
            //   low       = 低于下限（BttomStkQty > 0 且 Qty < BttomStkQty）—— 包含零库存
            //   all/alert = 低于下限 + 超过上限
            if table == "tStk_Stock" && wc.field == "WarnType" {
                let warn_val = wc.value.as_str().unwrap_or("");
                let cond = match warn_val {
                    "high" => "(t.[Qty] > g.[TopStkQty] AND g.[TopStkQty] > 0)".to_string(),
                    "low" => "(g.[BttomStkQty] > 0 AND t.[Qty] < g.[BttomStkQty])".to_string(),
                    "all" | "alert" => "((t.[Qty] > g.[TopStkQty] AND g.[TopStkQty] > 0) OR (g.[BttomStkQty] > 0 AND t.[Qty] < g.[BttomStkQty]))".to_string(),
                    // all_goods / 空 / 其他：不应用预警条件（全部商品，仅依赖 fixedWheres 过滤品态）
                    _ => String::new(),
                };
                if !cond.is_empty() {
                    conditions.push(cond);
                }
                continue;
            }

            // ★ 商品资料默认仓库筛选：IsDefaultInGoods 是虚拟字段
            //   仅显示商品资料默认仓库 = 系统默认仓库(IsDefault=1 的 tBas_Stock) 的商品
            //   避免商品默认仓库是 109 赠品仓 等非默认仓库的商品干扰预警
            if table == "tStk_Stock" && wc.field == "IsDefaultInGoods" {
                let cond = "(g.[StkID] IN (SELECT [StkID] FROM [tBas_Stock] WHERE [IsDefault] = 1))".to_string();
                conditions.push(cond);
                continue;
            }

            let op = match wc.op.as_str() {
                "eq" | "=" => "=",
                "ne" | "<>" | "!=" => "<>",
                "gt" | ">" => ">",
                "lt" | "<" => "<",
                "gte" | ">=" => ">=",
                "lte" | "<=" => "<=",
                "like" | "LIKE" => "LIKE",
                "in" | "IN" => "IN",
                _ => "=",
            };

            // 别名 → 实际数据库列名映射（SELECT 中用别名，WHERE 中必须用真实列名）
            let real_field = match (table, wc.field.as_str()) {
                ("tStk_IODetail", "IOState") => "State",
                _ => wc.field.as_str(),
            };

            let prefix = get_field_prefix_for_table(table, &wc.field);
            let col_expr = format!("{}.[{}]", prefix, real_field);

            if op == "IN" {
                // IN 操作符：value 可以是数组或逗号分隔的字符串
                // 不预分配 pidx，按需分配参数索引
                let values: Vec<String> = match &wc.value {
                    serde_json::Value::Array(arr) => {
                        arr.iter().filter_map(|v| {
                            match v {
                                serde_json::Value::String(s) => Some(s.clone()),
                                serde_json::Value::Number(n) => Some(n.to_string()),
                                _ => None,
                            }
                        }).collect()
                    }
                    serde_json::Value::String(s) => {
                        s.split(',').map(|x| x.trim().trim_matches('\'').trim_matches('"').to_string())
                            .filter(|x| !x.is_empty())
                            .collect()
                    }
                    _ => Vec::new(),
                };
                if values.is_empty() {
                    conditions.push("1=0".to_string());
                } else {
                    let placeholders: Vec<String> = values.iter().map(|v| {
                        params.push(Some(v.clone()));
                        let p = param_idx;
                        param_idx += 1;
                        format!("@p{}", p)
                    }).collect();
                    conditions.push(format!("{} IN ({})", col_expr, placeholders.join(", ")));
                }
            } else {
                let pidx = param_idx;
                param_idx += 1;
                if op == "LIKE" {
                    if let serde_json::Value::String(s) = &wc.value {
                        params.push(Some(format!("%{}%", s)));
                    } else {
                        params.push(json_to_sql_value(&wc.value));
                    }
                    conditions.push(format!("{} LIKE @p{}", col_expr, pidx));
                } else {
                    params.push(json_to_sql_value(&wc.value));
                    // ★ 当值为 'Y'/'N' 时，强制把列 CAST 成 nvarchar 再比较
                    //   防止 SQL Server 把 bit 列隐式转换失败（error 245）
                    let col_expr = match &wc.value {
                        serde_json::Value::String(s) if s == "Y" || s == "N" => {
                            format!("CAST({}.[{}] AS nvarchar(1))", prefix, real_field)
                        }
                        _ => format!("{}.[{}]", prefix, real_field),
                    };
                    conditions.push(format!("{} {} @p{}", col_expr, op, pidx));
                }
            }
        }
    }

    let (select_cols, join_clause) = get_joins_for_table(table);

    // 查询 tBas_Goods 且传了 warehouse_id 时，LEFT JOIN tStk_Stock 返回 StockQty/QQty
    // 用于盘点单等场景：选择商品时自动带出当前仓库的账存数量
    let (select_cols, join_clause) = if table == "tBas_Goods" {
        if let Some(wid) = warehouse_id {
            if !wid.is_empty() {
                let extra_select = format!("{}, ISNULL(st.[Qty], 0) AS [StockQty], ISNULL(st.[QQty], 0) AS [QQty]", select_cols);
                let extra_join = format!("{} LEFT JOIN [tStk_Stock] st ON t.[GDSID] = st.[GDSID] AND st.[StkID] = @p{}", join_clause, param_idx);
                params.push(Some(wid.clone()));
                (extra_select, extra_join)
            } else {
                (select_cols, join_clause)
            }
        } else {
            (select_cols, join_clause)
        }
    } else {
        (select_cols, join_clause)
    };

    let sql = if conditions.is_empty() {
        if join_clause.is_empty() {
            format!("SELECT {} FROM [{}] t", select_cols, table)
        } else {
            format!("SELECT {} FROM [{}] t {}", select_cols, table, join_clause)
        }
    } else {
        if join_clause.is_empty() {
            format!("SELECT {} FROM [{}] t WHERE {}", select_cols, table, conditions.join(" AND "))
        } else {
            format!("SELECT {} FROM [{}] t {} WHERE {}", select_cols, table, join_clause, conditions.join(" AND "))
        }
    };

    BuiltQuery { sql, params }
}

pub async fn generic_query(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericQueryParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    if params.table.is_empty() {
        return Ok(Json(ApiResponse::err_with_code("表名不能为空", VALIDATION_TABLE_INVALID)));
    }
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID)));
    }
    // 系统表黑名单校验（admin 放行）
    if is_table_blacklisted(&params.table, &claims) {
        return Ok(Json(ApiResponse::err_with_code(
            &format!("系统表 [{}] 禁止通过通用接口查询，请使用专用接口", params.table),
            "PERMISSION_DENIED",
        )));
    }
    // 字段名安全校验：防止 keyword_fields / wheres.field 注入
    if let Some(ref fields) = params.keyword_fields {
        for f in fields {
            if !is_valid_identifier(f) {
                return Ok(Json(ApiResponse::err(&format!("非法字段名: {}", f))));
            }
            // P2-19：禁止查询敏感字段
            if is_sensitive_field(f) {
                return Ok(Json(ApiResponse::err(&format!("字段 {} 不允许查询", f))));
            }
        }
    }
    if let Some(ref wc_list) = params.wheres {
        for wc in wc_list {
            if !is_valid_identifier(&wc.field) {
                return Ok(Json(ApiResponse::err(&format!("非法字段名: {}", wc.field))));
            }
            if is_sensitive_field(&wc.field) {
                return Ok(Json(ApiResponse::err(&format!("字段 {} 不允许查询", wc.field))));
            }
        }
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Ok(Json(ApiResponse::err(&format!("数据库连接失败: {}", e)))),
    };

    // ★ 调试日志：打印收到的 where 条件（临时，排查 IsDefault 过滤问题）
    if params.table == "tStk_Stock" {
        if let Some(ref wc_list) = params.wheres {
            for wc in wc_list {
                eprintln!("[DEBUG tStk_Stock] where: field={}, op={}, value={}", wc.field, wc.op, wc.value);
            }
        } else {
            eprintln!("[DEBUG tStk_Stock] no wheres");
        }
    }

    // Auto-create tStk_Qty table if it doesn't exist
    if params.table == "tStk_Qty" {
        let create_sql = "IF NOT EXISTS (SELECT * FROM sysobjects WHERE name='tStk_Qty' AND xtype='U') \
                          CREATE TABLE [tStk_Qty] ([QtyID] uniqueidentifier PRIMARY KEY DEFAULT NEWID(), \
                          [GDSID] uniqueidentifier NULL, [StkID] uniqueidentifier NULL, \
                          [Qty] decimal(18,4) DEFAULT 0, [LUTime] datetime DEFAULT GETDATE())";
        let _ = conn.execute(create_sql, &[]).await;
    }

    let page = params.page.unwrap_or(1);
    // 导出场景需较大 page_size 一次拉完；列表查询默认 50 不受影响
    // 上限与前端 MAX_EXPORT_ROWS 对齐，避免分页多次请求的深分页开销
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 50000);

    // 性能诊断：记录各阶段耗时
    let t_start = std::time::Instant::now();
    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false), &params.warehouse_id);
    let t_build = t_start.elapsed();

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    let t_count_start = std::time::Instant::now();
    match conn.query(&count_sql, &param_refs).await {
        Ok(count_stream) => {
            match count_stream.into_row().await {
                Ok(Some(row)) => {
                    let v = try_get_value(&row, "cnt");
                    total = match v {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0) as i32,
                        serde_json::Value::String(s) => s.parse::<i32>().unwrap_or(0),
                        _ => 0,
                    };
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!("[generic_query] COUNT 失败 table={} err={} sql={}", params.table, e, count_sql);
                    let err_msg = format!("查询表 [{}] 失败（可能是表不存在或字段错误），详情见服务端日志", params.table);
                    return Ok(Json(ApiResponse::err(&err_msg)));
                }
            }
        }
        Err(e) => {
            tracing::error!("[generic_query] COUNT 失败 table={} err={} sql={}", params.table, e, count_sql);
            let err_msg = format!("查询表 [{}] 失败（请确认表是否存在），详情见服务端日志", params.table);
            return Ok(Json(ApiResponse::err(&err_msg)));
        }
    }
    let t_count = t_count_start.elapsed();

    let paginated_sql = build_pagination_sql_with_sort(&built.sql, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let t_data_start = std::time::Instant::now();
    let result = match conn.query(&paginated_sql, &param_refs).await {
        Ok(data_stream) => {
            match data_stream.into_first_result().await {
                Ok(rows) => {
                    let row_count = rows.len();
                    let t_serialize_start = std::time::Instant::now();
                    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
                    let t_serialize = t_serialize_start.elapsed();
                    let t_data = t_data_start.elapsed();
                    tracing::info!(
                        "[generic_query] 性能 table={} page={} page_size={} total={} rows={} | build={:?} count={:?} data={:?} (含序列化 {:?})",
                        params.table, page, page_size, total, row_count,
                        t_build, t_count, t_data, t_serialize
                    );
                    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
                }
                Err(e) => {
                    tracing::error!("[generic_query] 数据读取失败 table={} err={} sql={}", params.table, e, paginated_sql);
                    Ok(Json(ApiResponse::err("读取数据失败，详情见服务端日志")))
                }
            }
        }
        Err(e) => {
            tracing::error!("[generic_query] 数据查询失败 table={} err={} sql={}", params.table, e, paginated_sql);
            Ok(Json(ApiResponse::err(&format!("执行查询失败（表[{}]可能不存在），详情见服务端日志", params.table))))
        }
    };
    result
}

pub async fn generic_export(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericQueryParams>,
) -> Json<ApiResponse<Vec<serde_json::Value>>> {
    if let Err(msg) = validate_query_params(&params.table, &params.keyword_fields, &params.wheres) {
        return Json(ApiResponse::err(&msg));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过导出接口读取系统表敏感数据
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("表 [{}] 为系统敏感表，禁止通过通用接口导出", params.table),
            PERMISSION_DENIED_TABLE,
        ));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false), &params.warehouse_id);
    let export_sql = format!("SELECT TOP 10000 * FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let data_stream = match conn.query(&export_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return Json(ApiResponse::err(&format!("导出查询失败 [{}]: {}", params.table, e))),
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return Json(ApiResponse::err(&format!("导出读取数据失败: {}", e))),
    };
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    // 记录导出日志（行数 = 实际返回行数；KeyValue 留空，主键对导出无意义）
    let row_count = data.len();
    let export_remark = format!("导出{}条记录", row_count);
    let _ = inventory_ledger::record_oper(
        &mut conn, "EXPORT", &params.table, "",
        &claims.user_code, None, Some(&export_remark),
    ).await;

    Json(ApiResponse::ok(data))
}

/// 清理孤儿库存记录
/// 当商品要硬删除但 tStk_Stock 中残留 Qty=0 的孤儿记录时，前端弹窗调用此接口清理
/// 安全保障：
/// 1. 只能清理 tStk_Stock 表（硬编码，不接受前端传表名）
/// 2. 只能清理 ABS(Qty) <= 0.5 的记录（即数量为 0 的孤儿记录）
/// 3. 必须传 GDSID 列表，只清理这些商品的库存记录
/// 4. 同时清理 tStk_Qty 快照表中的对应记录（保持一致性）
pub async fn generic_cleanup_orphan_stock(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericDeleteParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    // 只允许清理 tStk_Stock 表（硬编码，防止前端传其他表名）
    if params.table != "tStk_Stock" {
        return Json(ApiResponse::err_with_code(
            "此接口仅用于清理 tStk_Stock 孤儿库存记录",
            "PERMISSION_DENIED",
        ));
    }
    // 系统表黑名单校验（admin 放行）
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            "系统表禁止通过通用接口删除",
            "PERMISSION_DENIED",
        ));
    }
    if params.ids.is_empty() {
        return Json(ApiResponse::err("缺少待清理的商品 ID"));
    }

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    // 参数化 IN 子句
    let placeholders: Vec<String> = (1..=params.ids.len())
        .map(|i| format!("@p{}", i))
        .collect();
    let in_clause = placeholders.join(",");

    // 先查询将要清理的记录（用于操作日志和返回给前端）
    let select_sql = format!(
        "SELECT s.StkID, s.GDSID, s.Qty, k.StkName, g.GDSDesc \
         FROM tStk_Stock s \
         LEFT JOIN tBas_Stock k ON s.StkID = k.StkID \
         LEFT JOIN tBas_Goods g ON s.GDSID = g.GDSID \
         WHERE s.GDSID IN ({}) AND ABS(ISNULL(s.Qty,0)) <= 0.5",
        in_clause
    );
    let id_params: Vec<Option<String>> = params.ids.iter().map(|s| Some(s.clone())).collect();
    let id_param_refs: Vec<&dyn tiberius::ToSql> = id_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut orphan_records: Vec<serde_json::Value> = Vec::new();
    let mut orphan_keys: Vec<(String, String)> = Vec::new(); // (StkID, GDSID)
    match conn.query(&select_sql, &id_param_refs).await {
        Ok(stream) => {
            match stream.into_results().await {
                Ok(result_sets) => {
                    for result_set in result_sets {
                        for row in result_set {
                            let stk_id = match try_get_value(&row, "StkID") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let gds_id = match try_get_value(&row, "GDSID") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let stk_name = match try_get_value(&row, "StkName") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let gds_name = match try_get_value(&row, "GDSDesc") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let qty = match try_get_value(&row, "Qty") {
                                serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            orphan_keys.push((stk_id.clone(), gds_id.clone()));
                            orphan_records.push(serde_json::json!({
                                "StkID": stk_id,
                                "GDSID": gds_id,
                                "StkName": stk_name,
                                "GDSDesc": gds_name,
                                "Qty": qty,
                            }));
                        }
                    }
                }
                Err(e) => {
                    return Json(ApiResponse::err(&format!(
                        "查询孤儿库存记录失败: {}",
                        e
                    )))
                }
            }
        }
        Err(e) => {
            return Json(ApiResponse::err(&format!(
                "查询孤儿库存记录失败: {}",
                e
            )))
        }
    }

    if orphan_keys.is_empty() {
        return Json(ApiResponse::ok(serde_json::json!({
            "cleaned": 0,
            "message": "未找到可清理的孤儿库存记录（所有记录的库存数量都不为 0）"
        })));
    }

    // 执行清理：删除 tStk_Stock 中 Qty=0 的孤儿记录
    let delete_stock_sql = format!(
        "DELETE FROM tStk_Stock WHERE GDSID IN ({}) AND ABS(ISNULL(Qty,0)) <= 0.5",
        in_clause
    );
    match conn.execute(&delete_stock_sql, &id_param_refs).await {
        Ok(_) => {
            tracing::info!(
                "[cleanup_orphan_stock] 清理 tStk_Stock 孤儿记录: {} 条, GDSID={:?}",
                orphan_keys.len(),
                params.ids
            );
        }
        Err(e) => {
            return Json(ApiResponse::err(&format!(
                "清理 tStk_Stock 记录失败: {}",
                e
            )))
        }
    }

    // 同步清理 tStk_Qty 快照表（保持一致性）
    let delete_qty_sql = format!(
        "DELETE FROM tStk_Qty WHERE GDSID IN ({}) AND ABS(ISNULL(Qty,0)) <= 0.5",
        in_clause
    );
    match conn.execute(&delete_qty_sql, &id_param_refs).await {
        Ok(_) => {
            tracing::info!(
                "[cleanup_orphan_stock] 同步清理 tStk_Qty 快照记录, GDSID={:?}",
                params.ids
            );
        }
        Err(e) => {
            tracing::warn!(
                "[cleanup_orphan_stock] 同步清理 tStk_Qty 快照失败（非阻塞）: {}",
                e
            );
            // tStk_Qty 是快照表，清理失败不阻塞主流程
        }
    }

    // 记录操作日志
    let _ = crate::services::inventory_ledger::record_oper(
        &mut conn,
        "STOCK_ORPHAN_CLEANUP",
        "tStk_Stock",
        &params.ids.join(","),
        &claims.user_code,
        None,
        Some(&format!(
            "清理孤儿库存记录 {} 条（Qty=0 的残留记录），商品 ID: {}",
            orphan_keys.len(),
            params.ids.join(",")
        )),
    )
    .await;

    Json(ApiResponse::ok(serde_json::json!({
        "cleaned": orphan_keys.len(),
        "records": orphan_records,
        "message": format!("已清理 {} 条孤儿库存记录", orphan_keys.len())
    })))
}

pub async fn generic_delete(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericDeleteParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    // 系统表黑名单校验（admin 放行）
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("系统表 [{}] 禁止通过通用接口删除，请使用专用接口", params.table),
            "PERMISSION_DENIED",
        ));
    }
    if let Err(msg) = validate_identifiers(&[&params.primary_key]) {
        return Json(ApiResponse::err(&msg));
    }
    if let Some(ref sf) = params.state_field { if !sf.is_empty() { if let Err(msg) = validate_identifiers(&[sf]) { return Json(ApiResponse::err(&msg)); } } }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.ids.is_empty() {
        return Json(ApiResponse::err("请选择要操作的记录"));
    }

    // admin 账号保护：禁止删除/作废工号为 admin 的员工，避免系统锁死
    if params.table.eq_ignore_ascii_case("tBas_Emp") {
        for id in &params.ids {
            if is_admin_employee(&mut conn, id).await {
                return Json(ApiResponse::err("禁止删除/作废 admin 账号（系统管理员），避免系统无法登录"));
            }
        }
    }

    // P0-S4 记录级越权防护：业务单据表（含 EUser 列）只能删除/作废自己创建的记录
    //   admin 全放行；基础资料表（无 EUser）不做此限制
    if let Err(msg) = check_record_ownership(&mut conn, &params.table, &params.primary_key, &params.ids, &claims).await {
        return Json(ApiResponse::err_with_code(&msg, PERMISSION_DENIED_RECORD));
    }

    // physicalDelete = true：执行物理删除（DELETE FROM）
    // physicalDelete = false：执行软删除（UPDATE State='D' / Used='N'）
    let physical_delete = params.permanent.unwrap_or(false);

    if physical_delete {
        tracing::info!("[彻底删除 v2] permanent=true ids={:?} table={}", params.ids, params.table);
        // 物理删除前：引用检查。若商品已被其它单据/库存引用，强制阻止物理删除，
        // 避免破坏外键完整性。返回被引用的表名 + 引用条数，便于用户清理。
        match check_references_blocking(&mut conn, &params.table, &params.ids, true).await {
            Ok(hits) => {
                if !hits.is_empty() {
                    let obj_name = table_cn_name(&params.table);
                    return Json(ApiResponse::err_with_data(
                        &format!("该{}已被以下数据引用，无法彻底删除，请先清理引用数据后再试。", obj_name),
                        "HARD_DELETE_REFERENCED",
                        serde_json::Value::Array(hits),
                    ));
                }
            }
            Err(e) => return Json(ApiResponse::err(&format!("引用检查失败: {}", e))),
        }

        // 引用检查通过，执行物理删除
        // 先查询每条记录的修改前数据快照（删除后无法再查）
        let mut before_snapshots: Vec<(String, Option<String>)> = Vec::new();
        for id in &params.ids {
            let snap = query_row_snapshot_json(&mut conn, &params.table, &params.primary_key, id).await;
            before_snapshots.push((id.clone(), snap));
        }
        // 对于单据表（doc_graph 中定义），调用 hard_delete_doc：已审核单据先回滚库存再删除
        let is_doc_table = crate::metadata::doc_graph::get_doc_meta(&params.table).is_some();
        if is_doc_table {
            for id in &params.ids {
                if let Err(e) = crate::services::doc_service::hard_delete_doc(&mut conn, &params.table, &params.primary_key, id).await {
                    return Json(ApiResponse::err(&format!("彻底删除失败 [{}]: {}", params.table, e)));
                }
            }
        } else {
            // 非单据表：直接物理删除
            for id in &params.ids {
                let sql = format!(
                    "DELETE FROM [{}] WHERE [{}] = @p1",
                    params.table, params.primary_key
                );
                let id_str = id.as_str();
                if let Err(e) = conn.execute(&sql, &[&id_str]).await {
                    return Json(ApiResponse::err(&format!("彻底删除失败 [{}]: {}（可能存在 SQL Server 外键约束）", params.table, e)));
                }
            }
        }
        // 记录操作日志：每条 ID 一条（含修改前数据快照）
        for (id, before_json) in &before_snapshots {
            inventory_ledger::record_oper_with_data(
                &mut conn, "DELETE", &params.table, id,
                &claims.user_code, None, Some("彻底删除记录"),
                before_json.as_deref(), None,
            ).await;
        }
        return Json(ApiResponse::msg(&format!("成功彻底删除 {} 条记录", params.ids.len())));
    }

    // 软删除：更新状态字段
    let state_field = params.state_field.as_deref().unwrap_or("State");
    // void=true 时 State 置 'C'（业务作废，保留可查）；否则按字段类型取默认（Used='N' 停用，State='D' 软删）
    let void_flag = params.void.unwrap_or(false);
    let delete_value = if state_field == "Used" {
        "N"
    } else if void_flag {
        "C"
    } else {
        "D"
    };

    // 软删时检查业务引用但不阻止（有库存的商品可以停用，不影响库存数据）
    // 引用信息作为警告附加到成功消息中，便于用户感知
    let mut ref_warnings: Vec<String> = Vec::new();
    if let Ok(hits) = check_references_blocking(&mut conn, &params.table, &params.ids, false).await {
        if !hits.is_empty() {
            let label = if state_field == "Used" { "停用" } else { "作废" };
            let obj_name = table_cn_name(&params.table);
            let hits_str = hits.iter()
                .map(|h| format!("  · {} ({}): {} 条",
                    h.get("label").and_then(|v| v.as_str()).unwrap_or(""),
                    h.get("table").and_then(|v| v.as_str()).unwrap_or(""),
                    h.get("count").and_then(|v| v.as_i64()).unwrap_or(0)))
                .collect::<Vec<_>>()
                .join("\n");
            ref_warnings.push(format!("该{}已被以下数据引用（{}不影响现有数据）：\n{}", obj_name, label, hits_str));
        }
    }

    // 先查询每条记录的修改前数据快照
    let mut before_snapshots: Vec<(String, Option<String>)> = Vec::new();
    for id in &params.ids {
        let snap = query_row_snapshot_json(&mut conn, &params.table, &params.primary_key, id).await;
        before_snapshots.push((id.clone(), snap));
    }

    for id in &params.ids {
        // 对于单据表（doc_graph 中定义），如果单据已审核（State='S'），软删前先回滚库存，
        // 避免出现"单据软删但库存未回滚"的悬空数据
        let is_doc_table = crate::metadata::doc_graph::get_doc_meta(&params.table).is_some();
        if is_doc_table {
            if let Err(e) = crate::services::doc_service::soft_delete_doc(&mut conn, &params.table, &params.primary_key, id).await {
                return Json(ApiResponse::err(&format!("软删除失败 [{}]: 库存回滚出错 - {}", params.table, e)));
            }
        }
        let sql = format!(
            "UPDATE [{}] SET [{}] = @p1 WHERE [{}] = @p2",
            params.table, state_field, params.primary_key
        );
        let id_str = id.as_str();
        if let Err(e) = conn.execute(&sql, &[&delete_value, &id_str]).await {
            return Json(ApiResponse::err(&format!("删除失败 [{}]: {}", params.table, e)));
        }
    }

    let label = if state_field == "Used" { "停用" } else { "作废" };
    let mut msg = format!("成功{}{}条记录", label, params.ids.len());
    if !ref_warnings.is_empty() {
        msg.push_str("\n\n");
        msg.push_str(&ref_warnings.join("\n\n"));
    }
    // 记录操作日志：每条 ID 一条（含修改前数据快照）
    let oper_type = if state_field == "Used" { "DISABLE" } else if void_flag { "VOID" } else { "DELETE" };
    let remark_str = format!("{}记录", label);
    for (id, before_json) in &before_snapshots {
        let after_json = serde_json::json!({ state_field: delete_value }).to_string();
        inventory_ledger::record_oper_with_data(
            &mut conn, oper_type, &params.table, id,
            &claims.user_code, None, Some(&remark_str),
            before_json.as_deref(), Some(&after_json),
        ).await;
    }
    Json(ApiResponse::msg(&msg))
}

/// 恢复软删除的记录：Used='Y' / State='N'
#[derive(Deserialize)]
pub struct GenericRestoreParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub state_field: Option<String>,
}

pub async fn generic_restore(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericRestoreParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') == false {
        return Json(ApiResponse::err_with_code("表名非法", VALIDATION_TABLE_INVALID));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过恢复接口绕过软删状态保护
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("表 [{}] 为系统敏感表，禁止通过通用接口恢复", params.table),
            PERMISSION_DENIED_TABLE,
        ));
    }
    if let Err(msg) = validate_identifiers(&[&params.primary_key]) {
        return Json(ApiResponse::err(&msg));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    // 自动选状态字段：先看客户端传入，再按表查找，最后兜底
    let sf: String = if let Some(s) = params.state_field.as_deref() {
        if !s.is_empty() { s.to_string() } else {
            match get_state_field_for_table(&params.table) { Some(x) => x.to_string(), None => return Json(ApiResponse::err(&format!("表 [{}] 没有软删字段，无法恢复", params.table))) }
        }
    } else {
        match get_state_field_for_table(&params.table) { Some(x) => x.to_string(), None => return Json(ApiResponse::err(&format!("表 [{}] 没有软删字段，无法恢复", params.table))) }
    };
    // 区分基础资料表和单据表：
    // - 基础资料（Used 字段）：Used=N→Y（停用→启用）
    // - 基础资料（State 字段）：State=D→Y（删除→启用）
    // - 单据表（doc_graph 中定义）：State=D→N（删除→新建），需用户重新审核才会重新过账库存
    //   绝对不能直接恢复到 S（已审核），否则库存对不上（软删时已回滚库存）
    let is_doc_table = crate::metadata::doc_graph::get_doc_meta(&params.table).is_some();
    let restore_value = if sf == "Used" {
        "Y"
    } else if is_doc_table {
        "N"
    } else {
        "Y"
    };

    // 先查询每条记录的修改前数据快照（用于操作日志）
    let mut before_snapshots: Vec<(String, Option<String>)> = Vec::new();
    for id in &params.ids {
        let snap = query_row_snapshot_json(&mut conn, &params.table, &params.primary_key, id).await;
        before_snapshots.push((id.clone(), snap));
    }

    let mut ok = 0usize;
    for id in &params.ids {
        let sql = format!(
            "UPDATE [{}] SET [{}] = @p1 WHERE [{}] = @p2",
            params.table, sf, params.primary_key
        );
        let id_str = id.as_str();
        match conn.execute(&sql, &[&restore_value, &id_str]).await {
            Ok(r) => ok += r.rows_affected().len(),
            Err(e) => return Json(ApiResponse::err(&format!("恢复失败 [{}]: {}", params.table, e))),
        }
    }
    // 记录操作日志：每条 ID 一条（含修改前数据快照和修改后状态）
    let label = if sf == "Used" { "启用" } else if is_doc_table { "恢复为新建" } else { "恢复" };
    let remark_str = format!("{}记录", label);
    for (id, before_json) in &before_snapshots {
        let after_json = serde_json::json!({ &sf: restore_value }).to_string();
        inventory_ledger::record_oper_with_data(
            &mut conn, "RESTORE", &params.table, id,
            &claims.user_code, None, Some(&remark_str),
            before_json.as_deref(), Some(&after_json),
        ).await;
    }
    Json(ApiResponse::msg(&format!("成功{}{}条记录", label, ok)))
}

/// 软删除/物理删除前的业务引用检查
/// 返回被引用的表名+条数+跳转路由（结构化数据，便于前端渲染可点击链接）
/// `strict=true` 物理删模式：所有引用都阻止（避免破坏外键完整性）
/// `strict=false` 软删模式：默认不阻止，返回引用清单作为参考信息（不阻塞操作）
///                       物理引用（如 tStk_Stock 有真实库存）只在 strict=true 时阻止
async fn check_references_blocking(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    table: &str,
    ids: &[String],
    strict: bool,
) -> std::result::Result<Vec<serde_json::Value>, Box<dyn std::error::Error + Send + Sync>> {
    let references = get_references_for_table(table);
    if references.is_empty() {
        return Ok(vec![]);
    }

    let mut hits: Vec<serde_json::Value> = Vec::new();
    // ============================================================
    // 特殊处理：tStk_Stock（商品库存余额）
    // ============================================================
    // 设计原则：tStk_Stock 中的 Qty=0 记录是"孤儿库存"（无业务意义的残留），
    //   不应阻塞商品硬删除。strict 模式下自动清理这些记录（包括 tStk_Qty 快照），
    //   用户无感知。只有 Qty≠0 的实际库存才阻塞，提示用户先反审/删除相关单据。
    //
    // 流程：
    //   1. strict 模式下，先 DELETE tStk_Stock 和 tStk_Qty 中 ABS(Qty)<=0.5 的孤儿记录
    //   2. 然后正常检查 tStk_Stock 的引用 count（此时只剩 Qty≠0 的记录）
    //   3. 如果还有引用（说明有实际库存），正常阻塞并返回引用信息
    if strict && ids.iter().any(|_| true) {
        let placeholders: Vec<String> = (1..=ids.len())
            .map(|i| format!("@p{}", i))
            .collect();
        let in_clause = placeholders.join(",");
        let id_params: Vec<Option<String>> = ids.iter().map(|s| Some(s.clone())).collect();
        let id_param_refs: Vec<&dyn tiberius::ToSql> = id_params
            .iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();
        // 自动清理 tStk_Stock 中 Qty=0 的孤儿记录
        let clean_stock_sql = format!(
            "DELETE FROM tStk_Stock WHERE GDSID IN ({}) AND ABS(ISNULL(Qty,0)) <= 0.5",
            in_clause
        );
        match conn.execute(&clean_stock_sql, &id_param_refs).await {
            Ok(_) => {
                tracing::info!(
                    "[check_references_blocking] 自动清理 tStk_Stock 孤儿库存 (Qty=0), GDSID={:?}",
                    ids
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[check_references_blocking] 自动清理 tStk_Stock 孤儿库存失败（非阻塞）: {}",
                    e
                );
            }
        }
        // 同步清理 tStk_Qty 快照表
        let clean_qty_sql = format!(
            "DELETE FROM tStk_Qty WHERE GDSID IN ({}) AND ABS(ISNULL(Qty,0)) <= 0.5",
            in_clause
        );
        match conn.execute(&clean_qty_sql, &id_param_refs).await {
            Ok(_) => {
                tracing::info!(
                    "[check_references_blocking] 同步清理 tStk_Qty 孤儿快照 (Qty=0), GDSID={:?}",
                    ids
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[check_references_blocking] 同步清理 tStk_Qty 孤儿快照失败（非阻塞）: {}",
                    e
                );
            }
        }
    }

    // 第一阶段：查询每个引用表的 count
    // 用 Vec<(ref_table, ref_col, ref_label, count)> 记录有引用的表，避免在 stream 还活着时借用 conn
    let mut ref_counts: Vec<(&'static str, &'static str, &'static str, i64)> = Vec::new();
    for (ref_table, ref_col, ref_label) in &references {
        // 参数化 IN 子句，防止 SQL 注入（ids 来自用户输入）
        let placeholders: Vec<String> = (1..=ids.len())
            .map(|i| format!("@p{}", i))
            .collect();
        let total_sql = format!(
            "SELECT COUNT(*) AS cnt FROM [{}] WHERE [{}] IN ({})",
            ref_table, ref_col, placeholders.join(",")
        );
        let ref_params: Vec<Option<String>> = ids.iter().map(|s| Some(s.clone())).collect();
        let ref_param_refs: Vec<&dyn tiberius::ToSql> = ref_params
            .iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();
        match conn.query(&total_sql, &ref_param_refs).await {
            Ok(stream) => {
                if let Ok(Some(row)) = stream.into_row().await {
                    let v = try_get_value(&row, "cnt");
                    let cnt = match v {
                        serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
                        serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
                        _ => 0,
                    };
                    if cnt > 0 {
                        ref_counts.push((ref_table, ref_col, ref_label, cnt));
                    }
                }
            }
            Err(e) => {
                tracing::warn!("引用检查跳过 [{}].[{}]: {}", ref_table, ref_col, e);
            }
        }
    }
    // 第二阶段：对有引用的表，strict 模式下额外查询前 10 条详情（单据号/仓库名）
    for (ref_table, ref_col, ref_label, cnt) in ref_counts {
        let route = ref_table_route(ref_table);
        let details = if strict {
            query_ref_details(conn, ref_table, ref_col, ids).await
        } else {
            vec![]
        };
        hits.push(serde_json::json!({
            "table": ref_table,
            "column": ref_col,
            "label": ref_label,
            "count": cnt,
            "route": route,
            "details": details,
        }));
    }
    Ok(hits)
}

/// 查询引用详情（前 10 条），返回具体被引用的单据号/仓库名等可读信息
/// 用于硬删除弹窗中展示"具体是哪些单据引用了该商品"，便于用户直接点击跳转清理
///
/// 返回每条记录：{ id, label, route, focus_field, focus_value, extra }
/// - label: 单据号或仓库名（用户可读）
/// - route: 跳转路由
/// - focus_field/focus_value: 跳转时带的 query 参数（DataPage 支持 ?focus=单据号 自动搜索）
async fn query_ref_details(
    conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>,
    ref_table: &str,
    ref_col: &str,
    ids: &[String],
) -> Vec<serde_json::Value> {
    if ids.is_empty() {
        return vec![];
    }
    let placeholders: Vec<String> = (1..=ids.len())
        .map(|i| format!("@p{}", i))
        .collect();
    let in_clause = placeholders.join(",");

    // 为每个引用表构造 JOIN 查询，返回前 10 条单据号/仓库名
    // 对于明细表引用，JOIN 主表获取单据号；对于主表/库存表引用，直接查
    let sql = match ref_table {
        // ===== 库存表（显示仓库名 + 库存数量）=====
        "tStk_Stock" => format!(
            "SELECT TOP 10 k.StkName AS label, k.StkID AS focus_value, 'StkID' AS focus_field, '/inventory/stock' AS route, CAST(s.QQty AS VARCHAR(50)) AS extra \
             FROM tStk_Stock s LEFT JOIN tBas_Stock k ON s.StkID = k.StkID \
             WHERE s.[{}] IN ({})",
            ref_col, in_clause
        ),
        // tStk_Qty 是 tStk_Stock 的物化快照，已从引用检查中移除（不作为独立阻塞条件）
        // ===== 明细表（JOIN 主表获取单据号）=====
        // tStk_IODetail：tStk_IO 是统一表，多种 Kind 共用，需根据 Kind 动态决定路由
        //   PD=采购入库→/purchase?tab=receipt, PR=采购退货→/purchase?tab=return,
        //   SD=销售出库→/sales?tab=outbound, SR=销售退货→/sales?tab=return,
        //   RI=领用单→/inventory/misc?tab=requisition, OTI=零散入库→/inventory/misc?tab=oti-inbound,
        //   OTO=零散出库→/inventory/misc?tab=oto-outbound, 其他→/inventory/misc
        // SQL 中通过 CASE WHEN 直接根据 Kind 计算好 route，避免前端二次处理
        "tStk_IODetail" => format!(
            "SELECT TOP 10 m.IONo AS label, m.IONo AS focus_value, 'focus' AS focus_field, \
                CASE m.Kind \
                    WHEN 'PD' THEN '/purchase?tab=receipt' \
                    WHEN 'PR' THEN '/purchase?tab=return' \
                    WHEN 'SD' THEN '/sales?tab=outbound' \
                    WHEN 'SR' THEN '/sales?tab=return' \
                    WHEN 'RI' THEN '/inventory/misc?tab=requisition' \
                    WHEN 'OTI' THEN '/inventory/misc?tab=oti-inbound' \
                    WHEN 'OTO' THEN '/inventory/misc?tab=oto-outbound' \
                    ELSE '/inventory/misc' \
                END AS route, m.Kind AS extra \
             FROM tStk_IODetail d LEFT JOIN tStk_IO m ON d.IOID = m.IOID \
             WHERE d.[{}] IN ({})",
            ref_col, in_clause
        ),
        // tStk_MoveDetail：tStk_Move 是统一表，DB/TH/ZP 共用
        //   DB=内部调拨→/inventory/move-tabs?tab=move, TH=门店退仓→/inventory/move-tabs?tab=store-return,
        //   ZP=门店直配→/inventory/move-tabs?tab=zp
        "tStk_MoveDetail" => format!(
            "SELECT TOP 10 m.MoveNO AS label, m.MoveNO AS focus_value, 'focus' AS focus_field, \
                CASE m.Kind \
                    WHEN 'DB' THEN '/inventory/move-tabs?tab=move' \
                    WHEN 'TH' THEN '/inventory/move-tabs?tab=store-return' \
                    WHEN 'ZP' THEN '/inventory/move-tabs?tab=zp' \
                    ELSE '/inventory/move-tabs' \
                END AS route, m.Kind AS extra \
             FROM tStk_MoveDetail d LEFT JOIN tStk_Move m ON d.MoveID = m.MoveID \
             WHERE d.[{}] IN ({})",
            ref_col, in_clause
        ),
        // tSal_InvDetail：JOIN tSal_Inv 获取销售出库单号
        // tSal_Inv 也是统一表（现款销售/门店销售），但前端都在 /sales?tab=outbound 下
        "tSal_InvDetail" => format!(
            "SELECT TOP 10 m.SINo AS label, m.SINo AS focus_value, 'focus' AS focus_field, '/sales?tab=outbound' AS route, '' AS extra \
             FROM tSal_InvDetail d LEFT JOIN tSal_Inv m ON d.SIID = m.SIID \
             WHERE d.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tPur_OrderDetail" => format!(
            "SELECT TOP 10 m.PoNo AS label, m.PoNo AS focus_value, 'focus' AS focus_field, '/purchase?tab=order' AS route, '' AS extra \
             FROM tPur_OrderDetail d LEFT JOIN tPur_Order m ON d.POID = m.POID \
             WHERE d.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tStk_ReplenishApplyDtl" => format!(
            "SELECT TOP 10 m.ReplenishApplyNo AS label, m.ReplenishApplyNo AS focus_value, 'focus' AS focus_field, '/inventory/replenish' AS route, '' AS extra \
             FROM tStk_ReplenishApplyDtl d LEFT JOIN tStk_ReplenishApply m ON d.ReplenishApplyID = m.ReplenishApplyID \
             WHERE d.[{}] IN ({})",
            ref_col, in_clause
        ),
        // ===== 主表引用（直接查单据号）=====
        "tStk_ReplenishApply" => format!(
            "SELECT TOP 10 m.ReplenishApplyNo AS label, m.ReplenishApplyNo AS focus_value, 'focus' AS focus_field, '/inventory/replenish' AS route, '' AS extra \
             FROM tStk_ReplenishApply m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tStk_IO" => format!(
            "SELECT TOP 10 m.IONo AS label, m.IONo AS focus_value, 'focus' AS focus_field, '/inventory/misc' AS route, m.Kind AS extra \
             FROM tStk_IO m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tStk_Move" => format!(
            "SELECT TOP 10 m.MoveNO AS label, m.MoveNO AS focus_value, 'focus' AS focus_field, '/inventory/move-tabs' AS route, m.Kind AS extra \
             FROM tStk_Move m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tPur_Order" => format!(
            "SELECT TOP 10 m.PoNo AS label, m.PoNo AS focus_value, 'focus' AS focus_field, '/purchase?tab=order' AS route, '' AS extra \
             FROM tPur_Order m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tPur_Quote" => format!(
            "SELECT TOP 10 m.PQNo AS label, m.PQNo AS focus_value, 'focus' AS focus_field, '/purchase?tab=order' AS route, '' AS extra \
             FROM tPur_Quote m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tPur_AdjPrice" => format!(
            "SELECT TOP 10 m.PAPNo AS label, m.PAPNo AS focus_value, 'focus' AS focus_field, '/purchase?tab=order' AS route, '' AS extra \
             FROM tPur_AdjPrice m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tSal_Inv" => format!(
            "SELECT TOP 10 m.SINo AS label, m.SINo AS focus_value, 'focus' AS focus_field, '/sales?tab=outbound' AS route, '' AS extra \
             FROM tSal_Inv m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tSal_Order" => format!(
            "SELECT TOP 10 m.SoNo AS label, m.SoNo AS focus_value, 'focus' AS focus_field, '/sales?tab=outbound' AS route, '' AS extra \
             FROM tSal_Order m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tSal_Quote" => format!(
            "SELECT TOP 10 m.SQNo AS label, m.SQNo AS focus_value, 'focus' AS focus_field, '/sales?tab=outbound' AS route, '' AS extra \
             FROM tSal_Quote m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tArd_PD" => format!(
            "SELECT TOP 10 m.PDNo AS label, m.PDNo AS focus_value, 'focus' AS focus_field, '/finance/payable' AS route, '' AS extra \
             FROM tArd_PD m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tFin_Receivable" => format!(
            "SELECT TOP 10 m.RecNo AS label, m.RecNo AS focus_value, 'focus' AS focus_field, '/finance/receivable' AS route, '' AS extra \
             FROM tFin_Receivable m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tFin_Receipt" => format!(
            "SELECT TOP 10 m.RecNO AS label, m.RecNO AS focus_value, 'focus' AS focus_field, '/finance/receipts' AS route, '' AS extra \
             FROM tFin_Receipt m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tFin_Payment" | "tAcc_PayOut" => format!(
            "SELECT TOP 10 m.PayNO AS label, m.PayNO AS focus_value, 'focus' AS focus_field, '/finance/payments' AS route, '' AS extra \
             FROM [{}] m WHERE m.[{}] IN ({})",
            ref_table, ref_col, in_clause
        ),
        // ===== 基础资料引用 =====
        "tBas_Goods" => format!(
            "SELECT TOP 10 m.GDSDesc AS label, m.GDSNO AS focus_value, 'keyword' AS focus_field, '/base?tab=product' AS route, '' AS extra \
             FROM tBas_Goods m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tBas_Supp" => format!(
            "SELECT TOP 10 m.SuppName AS label, m.SuppNo AS focus_value, 'keyword' AS focus_field, '/base/supp?tab=supplier' AS route, '' AS extra \
             FROM tBas_Supp m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tBas_Cust" => format!(
            "SELECT TOP 10 m.CustName AS label, m.CustNo AS focus_value, 'keyword' AS focus_field, '/base/cust?tab=customer' AS route, '' AS extra \
             FROM tBas_Cust m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        "tBas_Emp" => format!(
            "SELECT TOP 10 m.EmpName AS label, m.EmpNo AS focus_value, 'keyword' AS focus_field, '/base/usr?tab=employee' AS route, '' AS extra \
             FROM tBas_Emp m WHERE m.[{}] IN ({})",
            ref_col, in_clause
        ),
        _ => return vec![],
    };

    let ref_params: Vec<Option<String>> = ids.iter().map(|s| Some(s.clone())).collect();
    let ref_param_refs: Vec<&dyn tiberius::ToSql> = ref_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut details: Vec<serde_json::Value> = Vec::new();
    match conn.query(&sql, &ref_param_refs).await {
        Ok(stream) => {
            match stream.into_results().await {
                Ok(result_sets) => {
                    for result_set in result_sets {
                        for row in result_set {
                            let label = match crate::handlers::base_data::try_get_value(&row, "label") {
                                serde_json::Value::String(s) => s,
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => String::new(),
                            };
                            if label.is_empty() {
                                continue;
                            }
                            let route = match crate::handlers::base_data::try_get_value(&row, "route") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let focus_field = match crate::handlers::base_data::try_get_value(&row, "focus_field") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            let focus_value = match crate::handlers::base_data::try_get_value(&row, "focus_value") {
                                serde_json::Value::String(s) => s,
                                serde_json::Value::Number(n) => n.to_string(),
                                _ => String::new(),
                            };
                            let extra = match crate::handlers::base_data::try_get_value(&row, "extra") {
                                serde_json::Value::String(s) => s,
                                _ => String::new(),
                            };
                            details.push(serde_json::json!({
                                "label": label,
                                "route": route,
                                "focus_field": focus_field,
                                "focus_value": focus_value,
                                "extra": extra,
                            }));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("读取引用详情失败 [{}]: {}", ref_table, e);
                }
            }
        }
        Err(e) => {
            tracing::warn!("查询引用详情失败 [{}]: {}", ref_table, e);
        }
    }
    details
}

/// 引用表 → 前端路由映射（用于硬删除引用提示中的跳转链接）
/// 返回 None 表示该表无对应前端页面（如系统内部表）
fn ref_table_route(ref_table: &str) -> Option<&'static str> {
    match ref_table {
        // 库存类
        "tStk_Stock" | "tStk_Qty" | "tStk_Reserve" => Some("/inventory/stock"),
        // tStk_IODetail / tStk_IO 是统一表（PD/PR/SD/SR/RI/OTI/OTO 等），
        // 具体路由在 query_ref_details 中根据 Kind 动态计算，这里返回通用兜底
        "tStk_IODetail" | "tStk_IO" => Some("/inventory/misc"),
        // tStk_MoveDetail / tStk_Move 是统一表（DB/TH/ZP），
        // 具体路由在 query_ref_details 中根据 Kind 动态计算，这里返回通用兜底
        "tStk_MoveDetail" | "tStk_Move" => Some("/inventory/move-tabs"),
        "tStk_ReplenishApply" | "tStk_ReplenishApplyDtl" => Some("/inventory/replenish"),
        // 采购类
        "tPur_Order" | "tPur_OrderDetail" => Some("/purchase?tab=order"),
        "tPur_Quote" => Some("/purchase?tab=order"),
        "tPur_AdjPrice" => Some("/purchase?tab=order"),
        // 销售类（tSal_Inv 是独立表，不是 tStk_IO）
        "tSal_Inv" | "tSal_InvDetail" => Some("/sales?tab=outbound"),
        "tSal_Order" => Some("/sales?tab=outbound"),
        "tSal_Quote" => Some("/sales?tab=outbound"),
        // 财务类
        "tArd_PD" => Some("/finance/payable"),
        "tFin_Receivable" => Some("/finance/receivable"),
        "tFin_Receipt" => Some("/finance/receipts"),
        "tFin_Payment" | "tAcc_PayOut" => Some("/finance/payments"),
        // 线上商城
        "tOnline_Goods" => Some("/online/goods"),
        "tOnline_Order" | "tOnline_OrderDetail" => Some("/online/order"),
        // 基础资料
        "tBas_Goods" => Some("/base?tab=product"),
        "tBas_Supp" => Some("/base/supp?tab=supplier"),
        "tBas_Cust" => Some("/base/cust?tab=customer"),
        "tBas_Emp" => Some("/base/usr?tab=employee"),
        _ => None,
    }
}

/// 表名 → 中文名映射（用于硬删除/软删除引用提示中的对象名称）
/// 仅列出在 get_references_for_table 中有引用关系的基础资料表，
/// 其他表返回"记录"作为兜底。
fn table_cn_name(table: &str) -> &'static str {
    match table {
        "tBas_Goods" => "商品",
        "tBas_Supp" => "供应商",
        "tBas_Cust" => "客户",
        "tBas_Stock" => "仓库",
        "tBas_Brand" => "品牌",
        "tBas_Unit" => "单位",
        "tBas_Emp" => "员工",
        "tBas_GDSType" => "商品类型",
        "tBas_GDSProperty" => "商品属性",
        "tBas_GDSKind" => "商品小类",
        "tBas_DeaType" => "结算方式",
        "tBas_SuppType" => "供应商分类",
        "tBas_CustType" => "客户分类",
        "tBas_Area" => "区域",
        "tBas_Dept" => "部门",
        "tBas_Duty" => "职务",
        "tBas_Payment" => "支付方式",
        _ => "记录",
    }
}

/// 维护「哪些表通过哪个字段引用了哪张主表」的关系。
/// 物理删除前会逐一查询这些引用表，统计引用条数，
/// 若有任何引用就阻止物理删除（避免破坏外键完整性）。
/// 格式：(引用表, 引用字段, 友好名称)
fn get_references_for_table(table: &str) -> Vec<(&'static str, &'static str, &'static str)> {
    match table {
        "tBas_Goods" => vec![
            ("tStk_Stock", "GDSID", "商品库存余额"),
            ("tStk_Reserve", "GDSID", "库存预留"),
            ("tStk_IODetail", "GDSID", "出入库明细"),
            ("tStk_MoveDetail", "GDSID", "调拨明细"),
            ("tStk_ReplenishApply", "GDSID", "补货申请"),
            ("tStk_ReplenishApplyDtl", "GDSID", "补货申请明细"),
            ("tSal_InvDetail", "GDSID", "销售发票明细"),
            ("tPur_OrderDetail", "GDSID", "采购订单明细"),
            ("tOnline_Goods", "GDSID", "线上商城商品"),
            ("tOnline_OrderDetail", "GDSID", "线上订单明细"),
        ],
        "tBas_Supp" => vec![
            ("tBas_Goods", "SuppID", "商品资料"),
            ("tPur_Order", "SuppID", "采购订单"),
            ("tPur_Quote", "SuppID", "采购报价"),
            ("tPur_AdjPrice", "SuppID", "采购调价"),
            ("tArd_PD", "SuppID", "应付款"),
            ("tAcc_PayOut", "SuppID", "付款单"),
            ("tStk_IO", "SuppID", "出入库单"),
        ],
        "tBas_Cust" => vec![
            ("tSal_Inv", "CustID", "销售发票"),
            ("tSal_InvDetail", "CustID", "销售发票明细"),
            ("tSal_Order", "CustID", "销售订单"),
            ("tSal_Quote", "CustID", "销售报价"),
            ("tFin_Receivable", "CustID", "应收款"),
            ("tFin_Receipt", "CustID", "收款单"),
            ("tOnline_Order", "CustID", "线上订单"),
            ("tBas_Goods", "CustID", "商品资料"),
        ],
        "tBas_Stock" => vec![
            ("tStk_Stock", "StkID", "商品库存余额"),
            ("tStk_Qty", "StkID", "商品即时库存"),
            ("tStk_IO", "StkID", "出入库单"),
            ("tStk_Move", "FromStkID", "调拨单(发出)"),
            ("tStk_Move", "ToStkID", "调拨单(接收)"),
            ("tBas_Emp", "StkID", "员工"),
            ("tBas_Goods", "StkID", "商品资料"),
        ],
        "tBas_Brand" => vec![
            ("tBas_Goods", "BrandID", "商品资料"),
        ],
        "tBas_Unit" => vec![
            ("tBas_Goods", "UnitNO", "商品资料"),
        ],
        "tBas_Emp" => vec![
            ("tStk_IO", "EmpID", "出入库单"),
            ("tStk_Move", "EmpID", "调拨单"),
            ("tPur_Order", "EmpID", "采购订单"),
            ("tSal_Order", "EmpID", "销售订单"),
            ("tSal_Inv", "EmpID", "销售发票"),
        ],
        "tBas_GDSType" => vec![
            ("tBas_Goods", "GDSTypeID", "商品资料"),
        ],
        "tBas_GDSProperty" => vec![
            ("tBas_Goods", "GDSPropertyID", "商品资料"),
        ],
        "tBas_GDSKind" => vec![
            ("tBas_Goods", "GDSKindID", "商品资料"),
        ],
        "tBas_DeaType" => vec![],
        "tBas_SuppType" => vec![
            ("tBas_Supp", "SuppTypeID", "供应商资料"),
        ],
        "tBas_CustType" => vec![
            ("tBas_Cust", "CustTypeID", "客户资料"),
        ],
        "tBas_Area" => vec![
            ("tBas_Cust", "AreaID", "客户资料"),
            ("tBas_Supp", "AreaID", "供应商资料"),
        ],
        "tBas_Dept" => vec![
            ("tBas_Emp", "DeptID", "员工资料"),
        ],
        "tBas_Duty" => vec![
            ("tBas_Emp", "DutyID", "员工资料"),
        ],
        "tBas_Payment" => vec![
            ("tBas_Cust", "PLID", "客户资料"),
        ],
        _ => vec![],
    }
}

/// 树形查询参数
#[derive(Deserialize)]
pub struct GenericTreeParams {
    pub table: String,
    pub primary_key: Option<String>,
    pub parent_field: Option<String>,
    pub name_field: Option<String>,
    pub count_table: Option<String>,
    pub count_field: Option<String>,
    pub extra_fields: Option<String>,
    pub state_field: Option<String>,
}

/// 树形查询：将扁平的父子关系数据组装成树形结构返回
pub async fn generic_tree(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericTreeParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    // 修复 SQL 注入：校验所有用户可控的标识符（表名/字段名）
    let table = params.table.as_str();
    if !is_valid_identifier(table) {
        return Json(ApiResponse::err_with_code("非法表名", VALIDATION_FIELD_INVALID));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过树形接口读取系统表层级数据
    if is_table_blacklisted(table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("表 [{}] 为系统敏感表，禁止通过通用接口查询", table),
            PERMISSION_DENIED_TABLE,
        ));
    }
    let pk = params.primary_key.as_deref().unwrap_or("ID");
    if !is_valid_identifier(pk) {
        return Json(ApiResponse::err_with_code("非法主键字段名", VALIDATION_FIELD_INVALID));
    }
    let pf = params.parent_field.as_deref().unwrap_or("ParentID");
    if !is_valid_identifier(pf) {
        return Json(ApiResponse::err_with_code("非法父级字段名", VALIDATION_FIELD_INVALID));
    }
    let nf = params.name_field.as_deref().unwrap_or("Name");
    if !is_valid_identifier(nf) {
        return Json(ApiResponse::err_with_code("非法名称字段名", VALIDATION_FIELD_INVALID));
    }
    let sf = params.state_field.as_deref().unwrap_or("State");
    if !is_valid_identifier(sf) {
        return Json(ApiResponse::err_with_code("非法状态字段名", VALIDATION_FIELD_INVALID));
    }
    let extra = params.extra_fields.as_deref().unwrap_or("");
    if !extra.is_empty() {
        for field in extra.split(',') {
            let f = field.trim();
            if !f.is_empty() && !is_valid_identifier(f) {
                return Json(ApiResponse::err_with_code("非法额外字段名", VALIDATION_FIELD_INVALID));
            }
        }
    }
    // count_table / count_field 同样来自前端，拼接进 SQL 前必须校验
    if let Some(ct) = &params.count_table {
        if !is_valid_identifier(ct) {
            return Json(ApiResponse::err_with_code("非法计数表名", VALIDATION_FIELD_INVALID));
        }
    }
    if let Some(cf) = &params.count_field {
        if !is_valid_identifier(cf) {
            return Json(ApiResponse::err_with_code("非法计数字段名", VALIDATION_FIELD_INVALID));
        }
    }

    let mut select_parts = vec![
        format!("[{}]", pk),
        format!("[{}]", pf),
        format!("[{}]", nf),
        format!("[{}]", sf),
    ];
    if !extra.is_empty() {
        for f in extra.split(',') {
            let f = f.trim();
            if !f.is_empty() && !select_parts.iter().any(|s| s == &format!("[{}]", f)) {
                select_parts.push(format!("[{}]", f));
            }
        }
    }

    // 根据状态字段类型决定过滤条件
    // Used 字段: Y=启用, N=停用 → 过滤 <> 'N'
    // State 字段: S=复审, D=删除, Y=已审核, N=新建 → 过滤 <> 'D'
    // ★ State=NULL 的记录也要显示（SQL Server 中 NULL <> 'D' 结果为 NULL 会被过滤）
    //   否则会出现"编码已存在但查不到"的脏数据问题（唯一索引仍拦截 INSERT）
    let state_filter = if sf == "Used" {
        "<> 'N'".to_string()
    } else {
        "<> 'D' OR [State] IS NULL".to_string()
    };

    let sql = format!(
        "SELECT {} FROM [{}] WHERE [{}] ({}) ORDER BY [{}]",
        select_parts.join(", "),
        params.table,
        sf,
        state_filter,
        pk
    );

    let flat_rows = match conn.query(&sql, &[]).await {
        Ok(s) => match s.into_first_result().await {
            Ok(rows) => rows,
            Err(e) => return Json(ApiResponse::err(&format!("读取数据失败: {}", e))),
        },
        Err(e) => return Json(ApiResponse::err(&format!("查询失败: {}", e))),
    };

    let mut flat: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
    for row in &flat_rows {
        flat.push(row_to_json_map(row));
    }

    // 统计关联数量
    let mut count_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let (Some(ct), Some(cf)) = (&params.count_table, &params.count_field) {
        let count_sql = format!(
            "SELECT [{}], COUNT(*) AS cnt FROM [{}] WHERE [{}] IS NOT NULL GROUP BY [{}]",
            cf, ct, cf, cf
        );
        if let Ok(count_stream) = conn.query(&count_sql, &[]).await {
            if let Ok(count_rows) = count_stream.into_first_result().await {
                for row in &count_rows {
                    // 使用 try_get_value 安全提取，避免类型不匹配时 panic
                    let key = value_to_string(&try_get_value(row, cf));
                    if key.is_empty() || key == "null" {
                        continue;
                    }
                    let cnt = value_to_string(&try_get_value(row, "cnt"))
                        .parse::<i64>()
                        .unwrap_or(0);
                    count_map.insert(key, cnt);
                }
            }
        }
    }

    // 建立节点索引
    let pk_lower = pk.to_lowercase();
    let pf_lower = pf.to_lowercase();

    let mut node_map: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    for item in &flat {
        if let Some(id_val) = item.get(&pk_lower) {
            let id_str = value_to_string(id_val);
            let mut node = serde_json::Map::new();
            for (k, v) in item {
                node.insert(k.clone(), v.clone());
            }
            let count = count_map.get(&id_str).copied().unwrap_or(0);
            node.insert("product_count".to_string(), serde_json::Value::Number(count.into()));
            node.insert("children".to_string(), serde_json::Value::Array(vec![]));
            node_map.insert(id_str, serde_json::Value::Object(node));
        }
    }

    // 组装父子关系
    let mut root_ids: Vec<String> = Vec::new();
    let mut children_map: std::collections::HashMap<String, Vec<serde_json::Value>> = std::collections::HashMap::new();

    for item in &flat {
        let id_str = item.get(&pk_lower)
            .map(|v| value_to_string(v))
            .unwrap_or_default();

        let parent_str = item.get(&pf_lower)
            .map(|v| value_to_string(v))
            .unwrap_or_default();

        if !parent_str.is_empty() && parent_str != "null" {
            children_map.entry(parent_str).or_default().push(
                node_map.get(&id_str).cloned().unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
            );
        } else {
            root_ids.push(id_str);
        }
    }

    // 递归构建树
    fn build_tree(
        id: &str,
        node_map: &std::collections::HashMap<String, serde_json::Value>,
        children_map: &std::collections::HashMap<String, Vec<serde_json::Value>>,
        pk_lower: &str,
    ) -> serde_json::Value {
        let mut node = match node_map.get(id) {
            Some(n) => n.clone(),
            None => return serde_json::Value::Null,
        };

        if let Some(children) = children_map.get(id) {
            let child_trees: Vec<serde_json::Value> = children.iter()
                .filter_map(|c| {
                    let cid = c.as_object()
                        .and_then(|obj| obj.get(pk_lower))
                        .map(|v| value_to_string(v))
                        .unwrap_or_default();
                    if cid.is_empty() { return None; }
                    Some(build_tree(&cid, node_map, children_map, pk_lower))
                })
                .filter(|v| !v.is_null())
                .collect();

            if let Some(obj) = node.as_object_mut() {
                let self_count = obj.get("product_count").and_then(|v| v.as_i64()).unwrap_or(0);
                let children_count: i64 = child_trees.iter()
                    .filter_map(|c| c.get("product_count").and_then(|v| v.as_i64()))
                    .sum();
                obj.insert("product_count".to_string(), serde_json::Value::Number((self_count + children_count).into()));

                if !child_trees.is_empty() {
                    obj.insert("children".to_string(), serde_json::Value::Array(child_trees));
                } else {
                    obj.remove("children");
                }
            }
        }

        node
    }

    let tree: Vec<serde_json::Value> = root_ids.iter()
        .filter_map(|id| {
            let v = build_tree(id, &node_map, &children_map, &pk_lower);
            if v.is_null() { None } else { Some(v) }
        })
        .collect();

    Json(ApiResponse::ok(serde_json::Value::Array(tree)))
}

/// 辅助：serde_json::Value -> String（用于 ID 比较）
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => v.to_string().trim_matches('"').to_string(),
    }
}

/// 辅助：Row -> serde_json::Map
fn row_to_json_map(row: &Row) -> serde_json::Map<String, serde_json::Value> {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" { continue; }
        let val = try_get_value(row, &name);
        map.insert(name.to_lowercase(), val);
    }
    map
}

/// 查询单行数据快照并序列化为 JSON 字符串（用于操作日志 before_data）
async fn query_row_snapshot_json(
    conn: &mut crate::services::inventory_ledger::Conn,
    table: &str,
    primary_key: &str,
    id: &str,
) -> Option<String> {
    let sql = format!("SELECT * FROM [{}] WHERE [{}] = @p1", table, primary_key);
    match conn.query(&sql, &[&id]).await {
        Ok(stream) => match stream.into_row().await {
            Ok(Some(row)) => {
                let val = crate::handlers::base_data::row_to_json(&row);
                serde_json::to_string(&val).ok()
            }
            _ => None,
        },
        Err(_) => None,
    }
}

pub async fn generic_create(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericCreateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !is_valid_identifier(&params.table) {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    // 系统表黑名单校验（admin 放行）
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("系统表 [{}] 禁止通过通用接口新增，请使用专用接口", params.table),
            "PERMISSION_DENIED",
        ));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供新增数据"));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let pk_col = get_primary_key_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    // BIT 字段列表：用于把 'Y'/'N' 字符串转成 1/0，避免 SQL Server 转换失败
    let bit_columns = fetch_bit_columns(&mut conn, &params.table).await;
    let empty_str_cols = default_empty_string_cols();
    // ★ 自动补充审计字段：EUser（按当前登录用户 EmpNo 查 UUID）/ EDate（当前时间）
    // 仅当表存在这些列且客户端未传值时才填充。业务字段一律不补。
    let user_uuid = lookup_user_uuid(&mut conn, &claims.user_code).await;
    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let mut columns = Vec::new();
    let mut placeholders = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    // If the PK is omitted by the client (or sent as an empty string),
    // auto-generate a fresh UUID so SQL Server does not have to convert ''.
    let mut generated_pk: Option<String> = None;
    if let Some(pk) = pk_col {
        let needs_generate = match params.data.get(pk) {
            None => true,
            Some(serde_json::Value::String(s)) if s.trim().is_empty() => true,
            Some(serde_json::Value::Null) => true,
            _ => false,
        };
        if needs_generate {
            let new_id = uuid::Uuid::new_v4().to_string();
            generated_pk = Some(new_id.clone());
            columns.push(format!("[{}]", pk));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(new_id));
        }
    }

    for (key, val) in params.data.iter() {
        let key_lc = key.to_lowercase();
        // 字段名安全校验：防止 SQL 注入
        if !is_valid_identifier(key) { continue; }
        // Skip fields that come from JOIN (not own columns)
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip IDENTITY / computed columns — SQL Server rejects explicit inserts
        // (would need IDENTITY_INSERT ON). The DB will auto-fill them.
        if readonly_fields.contains(&key_lc) { continue; }
        // If PK was already auto-generated above, ignore whatever the client sent.
        if let Some(pk) = pk_col {
            if key_lc == pk.to_lowercase() && generated_pk.is_some() { continue; }
        }
        columns.push(format!("[{}]", key));
        placeholders.push(format!("@p{}", columns.len()));
        // BIT 字段特殊处理：把 'Y'/'N' 字符串转成 1/0，避免 SQL Server 转换失败
        let val = if bit_columns.contains(&key_lc) {
            normalize_bit_value(val)
        } else {
            val.clone()
        };
        let mut v = json_to_sql_value(&val);
        // tBas_Emp 表密码字段 bcrypt 加密（空密码不加密，已加密的不重复）
        if params.table == "tBas_Emp" && key == "PassWordStr" {
            if let Some(ref s) = v {
                if !s.is_empty() && !s.starts_with("BCRYPT:") {
                    if let Some(hashed) = hash_password(s) {
                        v = Some(hashed);
                    }
                }
            }
        }
        // NOT NULL 文本列：空值用 '' 写入而不是 NULL
        if v.is_none() && empty_str_cols.contains(key) {
            v = Some(String::new());
        }
        values.push(v);
    }

    // ★ 审计字段自动填充：仅当表存在 EDate / EUser / LUTime 列，且客户端未提供时追加
    let provided_keys: std::collections::HashSet<String> = params.data.keys()
        .map(|k| k.to_lowercase())
        .collect();
    let mut pushed_audit = false;
    if !provided_keys.contains("edate") && has_column(&mut conn, &params.table, "EDate").await {
        columns.push("[EDate]".to_string());
        placeholders.push(format!("@p{}", columns.len()));
        values.push(Some(now_str.clone()));
        pushed_audit = true;
    }
    if !provided_keys.contains("euser") && has_column(&mut conn, &params.table, "EUser").await {
        if let Some(ref uid) = user_uuid {
            columns.push("[EUser]".to_string());
            placeholders.push(format!("@p{}", columns.len()));
            values.push(Some(uid.clone()));
            pushed_audit = true;
        }
    }
    // LUTime（修改时间）：新增时即写入创建时间，避免列表"修改时间"列为空
    if !provided_keys.contains("lutime") && has_column(&mut conn, &params.table, "LUTime").await {
        columns.push("[LUTime]".to_string());
        placeholders.push(format!("@p{}", columns.len()));
        values.push(Some(now_str.clone()));
        pushed_audit = true;
    }
    let _ = pushed_audit;

    // ★ NOT NULL 字段自动补全：前端表单隐藏或未填写的 NOT NULL 字段，根据类型填充默认值
    //   避免因 NOT NULL 约束导致 INSERT 失败（如 tBas_Goods 的 CPrice/gdsSD 等字段）
    //   排除：已自动填充的审计字段（EDate/EUser/LUTime）、已生成的主键、IDENTITY/计算列
    //   ★ 包含两种情况：
    //     1. 前端未提供该字段（provided_keys 不含） → 补默认值
    //     2. 前端提供了但值为空（None，如空字符串被 json_to_sql_value 转换） → 补默认值
    //   ★ 兜底：nullable 的 State/Used 关键字段也补默认值
    //     原因：tBas_* 表的 State/Used 列均为 nullable 且无 DEFAULT 约束，
    //     若前端未提供（如用户在列设置里把 State 移出表单），数据库会写入 NULL，
    //     导致前端 stateMap[null] 不存在，列表显示"未知"
    //     State='N'（新建），Used='Y'（启用）—— 与 default_value_for_type 的兜底分支一致
    let table_cols = fetch_table_columns(&mut conn, &params.table).await;
    for col_info in &table_cols {
        let col_lc = col_info.name.to_lowercase();
        // nullable 字段：只对 State/Used 关键字段补默认值，其他 nullable 字段跳过
        if col_info.is_nullable {
            let is_state_or_used = col_lc == "state" || col_lc == "used";
            if !is_state_or_used { continue; }
        }
        // 跳过已自动追加的审计字段
        if col_lc == "edate" || col_lc == "euser" || col_lc == "lutime" { continue; }
        // 跳过主键（前面已处理）
        if let Some(pk) = pk_col {
            if col_lc == pk.to_lowercase() { continue; }
        }
        // 跳过 IDENTITY / 计算列（由 DB 自动填充，且 fetch_table_columns 无法识别）
        // 通过 readonly_fields 集合判断
        if readonly_fields.contains(&col_lc) { continue; }

        // 判断是否需要补默认值：未提供 或 提供了但值为空
        let need_default = if provided_keys.contains(col_lc.as_str()) {
            // 已提供：检查实际值是否为 None（在前面的遍历中已计算）
            // 找到该字段在 columns 中的索引，检查 values 中对应位置是否为 None
            // ★ 简化实现：直接从 params.data 取原始值判断
            let raw_val = params.data.iter()
                .find(|(k, _)| k.to_lowercase() == col_lc)
                .map(|(_, v)| v);
            match raw_val {
                None => true,
                Some(serde_json::Value::Null) => true,
                Some(serde_json::Value::String(s)) if s.trim().is_empty() => true,
                _ => false,
            }
        } else {
            true  // 未提供
        };

        if need_default {
            // nullable 的 State/Used 字段：data_type 通常是 char/nvarchar，
            // default_value_for_type 会返回空字符串，需要按字段名兜底
            let default_val = if col_info.is_nullable && col_lc == "state" {
                Some("Y".to_string())
            } else if col_info.is_nullable && col_lc == "used" {
                Some("Y".to_string())
            } else {
                default_value_for_type(&col_info.data_type, &col_info.name)
            };
            if let Some(default_val) = default_val {
                // 如果字段已在 columns 中（前端提供了空值），更新对应位置的值
                // 否则追加新列
                let col_name_brk = format!("[{}]", col_info.name);
                if let Some(idx) = columns.iter().position(|c| c.eq_ignore_ascii_case(&col_name_brk)) {
                    values[idx] = Some(default_val);
                } else {
                    columns.push(col_name_brk);
                    placeholders.push(format!("@p{}", columns.len()));
                    values.push(Some(default_val));
                }
            }
        }
    }

    if columns.is_empty() {
        return Json(ApiResponse::err("没有可插入的字段"));
    }

    let sql = format!(
        "INSERT INTO [{}] ({}) VALUES ({})",
        params.table,
        columns.join(", "),
        placeholders.join(", ")
    );

    let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    // IsDefault 互斥：tBas_Stock 新增默认仓库时，先清除其他仓库的默认标记
    if params.table == "tBas_Stock" {
        if let Some(is_default) = params.data.get("IsDefault") {
            let is_true = match is_default {
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::Number(n) => n.as_i64().map_or(false, |v| v != 0),
                serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
                _ => false,
            };
            if is_true {
                let clear_sql = "UPDATE [tBas_Stock] SET [IsDefault] = 0 WHERE [IsDefault] = 1";
                if let Err(e) = conn.execute(clear_sql, &[]).await {
                    return Json(ApiResponse::err(&format!("清除默认仓库失败: {}", e)));
                }
            }
        }
    }

    match conn.execute(&sql, &param_refs).await {
        Ok(_) => {
            // 新增后的数据快照（用于操作日志变更明细）
            let after_json = serde_json::to_string(&params.data).ok();
            // Echo back the (possibly generated) primary key so the frontend
            // can use it for follow-up detail inserts / navigation.
            if let Some(pk) = pk_col {
                let id_value = generated_pk.clone()
                    .or_else(|| {
                        params.data.get(pk)
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(|s| s.to_string())
                    });
                if let Some(ref id) = id_value {
                    // 记录操作日志（含数据快照）
                    inventory_ledger::record_oper_with_data(
                        &mut conn, "CREATE", &params.table, id,
                        &claims.user_code, None, Some("新增记录"),
                        None, after_json.as_deref(),
                    ).await;
                    return Json(ApiResponse::ok(serde_json::json!({
                        pk: id,
                        "id": id,
                    })));
                }
            }
            // 无主键回显时也记录日志
            inventory_ledger::record_oper_with_data(
                &mut conn, "CREATE", &params.table, "",
                &claims.user_code, None, Some("新增记录"),
                None, after_json.as_deref(),
            ).await;
            Json(ApiResponse::msg("新增成功"))
        }
        Err(e) => {
            tracing::error!("[generic_create] 失败 table={} err={} sql={}", params.table, e, sql);
            // ★ 根据常见的 SQL Server 错误码返回有意义的提示，避免笼统的"请确认表和字段是否存在"
            let err_str = e.to_string();
            let user_msg = if err_str.contains("code: 2601") || err_str.contains("不能在具有唯一索引") {
                // 唯一索引冲突：提取重复键值
                let dup_val = err_str
                    .split("重复键值为 (")
                    .nth(1)
                    .and_then(|s| s.split(')').next())
                    .unwrap_or("");
                format!("新增失败：编码 [{}] 已存在，请勿重复录入", dup_val)
            } else if err_str.contains("code: 547") || err_str.contains("FOREIGN KEY") || err_str.contains("外键") {
                "新增失败：关联数据不存在（外键约束冲突），请检查关联字段".to_string()
            } else if err_str.contains("code: 515") || err_str.contains("不能将值 NULL") {
                "新增失败：存在必填字段为空，请检查表单".to_string()
            } else if err_str.contains("code: 245") || err_str.contains("转换失败") {
                "新增失败：字段类型不匹配，请检查输入值".to_string()
            } else {
                format!("新增数据到表 [{}] 失败：{}", params.table, err_str)
            };
            Json(ApiResponse::err(&user_msg))
        }
    }
}

pub async fn generic_update(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericUpdateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    // 系统表黑名单校验（admin 放行）
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("系统表 [{}] 禁止通过通用接口修改，请使用专用接口", params.table),
            "PERMISSION_DENIED",
        ));
    }
    if let Err(msg) = validate_identifiers(&[&params.primary_key]) {
        return Json(ApiResponse::err(&msg));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供更新数据"));
    }

    // P0-S4 记录级越权防护：业务单据表（含 EUser 列）只能更新自己创建的记录
    //   admin 全放行；基础资料表（无 EUser）不做此限制
    let id_for_check = vec![params.id.clone()];
    if let Err(msg) = check_record_ownership(&mut conn, &params.table, &params.primary_key, &id_for_check, &claims).await {
        return Json(ApiResponse::err_with_code(&msg, PERMISSION_DENIED_RECORD));
    }

    // admin 账号保护：禁止修改 admin 的工号、停用登录权限、设为离职/删除状态
    if params.table.eq_ignore_ascii_case("tBas_Emp") {
        if is_admin_employee(&mut conn, &params.id).await {
            // 1. 禁止修改 EmpNo（工号是 admin 身份标识，改了会导致 is_admin 判断失效）
            for (key, _) in params.data.iter() {
                if key.eq_ignore_ascii_case("EmpNo") {
                    return Json(ApiResponse::err("禁止修改 admin 账号的工号（系统管理员标识），避免系统无法登录"));
                }
            }
            // 2. 检查是否试图停用登录、设为离职/删除
            let danger_keys = ["AllowLogin", "State", "WorkState"];
            for (key, val) in params.data.iter() {
                if danger_keys.iter().any(|k| key.eq_ignore_ascii_case(k)) {
                    let val_str: String = match val {
                        serde_json::Value::String(s) => s.clone(),
                        _ => val.to_string(),
                    };
                    // AllowLogin=N / State=D / WorkState=3 都会阻止 admin 登录
                    if val_str.eq_ignore_ascii_case("N") || val_str.eq_ignore_ascii_case("D") || val_str == "3" {
                        return Json(ApiResponse::err("禁止停用 admin 账号的登录权限或设为离职/删除状态，避免系统无法登录"));
                    }
                }
            }
        }
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    // BIT 字段列表：用于把 'Y'/'N' 字符串转成 1/0，避免 SQL Server 转换失败
    let bit_columns = fetch_bit_columns(&mut conn, &params.table).await;
    let mut set_clauses = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    for (key, val) in params.data.iter() {
        let key_lc = key.to_lowercase();
        // 字段名安全校验：防止 SQL 注入
        if !is_valid_identifier(key) { continue; }
        if key_lc == params.primary_key.to_lowercase() { continue; }
        // Skip fields that come from JOIN (not own columns) to avoid overwriting redundant Name columns
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip identity / computed columns — SQL Server rejects updates to them
        if readonly_fields.contains(&key_lc) { continue; }
        // ★ 可清空关联字段白名单：允许用户显式置 NULL（如客户解绑定价模板）
        //   这些列即使值为 null/'' 也必须写入，否则前端清空操作无效
        let clearable = is_clearable_nullable(&params.table, key);
        // 防御性跳过 null/空值：避免 NOT NULL 列被误设为 NULL（例如表单联动字段未及时回填）
        // 如果业务确实需要将某列置为 NULL，请走专用接口（或加入 clearable_nullable_cols 白名单）
        if !clearable && val.is_null() {
            continue;
        }
        // BIT 字段特殊处理：把 'Y'/'N'/'true'/'false' 字符串转成 '1'/'0'
        // SQL Server 不接受 nvarchar 'N' → bit 的隐式转换，会报错
        let is_bit_col = bit_columns.contains(&key_lc);
        let val = if is_bit_col {
            normalize_bit_value(val)
        } else {
            val.clone()
        };
        if !clearable {
            if let serde_json::Value::String(s) = &val {
                if s.trim().is_empty() {
                    continue;
                }
            }
        }
        set_clauses.push(format!("[{}] = @p{}", key, set_clauses.len() + 1));
        let mut v = json_to_sql_value(&val);
        // tBas_Emp 表密码字段 bcrypt 加密（空密码已被前面 continue 跳过，已加密的不重复）
        if params.table == "tBas_Emp" && key == "PassWordStr" {
            if let Some(ref s) = v {
                if !s.is_empty() && !s.starts_with("BCRYPT:") {
                    if let Some(hashed) = hash_password(s) {
                        v = Some(hashed);
                    }
                }
            }
        }
        values.push(v);
    }

    if set_clauses.is_empty() {
        return Json(ApiResponse::err("没有提供需要更新的字段"));
    }

    // ★ 自动更新 LUTime（修改时间）：仅当表存在 LUTime 列且客户端未显式提供时
    // 避免 UPDATE 后列表"修改时间"列仍是旧值
    let provided_keys: std::collections::HashSet<String> = params.data.keys()
        .map(|k| k.to_lowercase())
        .collect();
    let mut auto_lutime = false;
    if !provided_keys.contains("lutime") && has_column(&mut conn, &params.table, "LUTime").await {
        set_clauses.push(format!("[LUTime] = @p{}", set_clauses.len() + 1));
        let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        values.push(Some(now_str));
        auto_lutime = true;
    }
    let _ = auto_lutime;

    let pk_param_idx = values.len() + 1;
    let sql = format!(
        "UPDATE [{}] SET {} WHERE [{}] = @p{}",
        params.table,
        set_clauses.join(", "),
        params.primary_key,
        pk_param_idx
    );

    values.push(Some(params.id.clone()));

    // IsDefault 互斥：tBas_Stock 设为默认仓库时，先清除其他仓库的默认标记
    if params.table == "tBas_Stock" {
        if let Some(is_default) = params.data.get("IsDefault") {
            let is_true = match is_default {
                serde_json::Value::Bool(b) => *b,
                serde_json::Value::Number(n) => n.as_i64().map_or(false, |v| v != 0),
                serde_json::Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
                _ => false,
            };
            if is_true {
                let clear_sql = "UPDATE [tBas_Stock] SET [IsDefault] = 0 WHERE [IsDefault] = 1";
                if let Err(e) = conn.execute(clear_sql, &[]).await {
                    return Json(ApiResponse::err(&format!("清除默认仓库失败: {}", e)));
                }
            }
        }
    }

    let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    // 修改前查旧数据快照（用于操作日志变更明细对比）
    let before_snapshot: Option<serde_json::Value> = {
        let snap_sql = format!("SELECT * FROM [{}] WHERE [{}] = @p1", params.table, params.primary_key);
        match conn.query(&snap_sql, &[&params.id]).await {
            Ok(stream) => match stream.into_row().await {
                Ok(Some(row)) => Some(crate::handlers::base_data::row_to_json(&row)),
                _ => None,
            },
            Err(_) => None,
        }
    };

    match conn.execute(&sql, &param_refs).await {
        Ok(_) => {
            // 记录操作日志（含数据快照）
            // after 快照重新查询 DB：保证 before/after 都是完整行，避免请求体只有部分字段导致对比失真
            let after_snapshot: Option<serde_json::Value> = {
                let snap_sql = format!("SELECT * FROM [{}] WHERE [{}] = @p1", params.table, params.primary_key);
                match conn.query(&snap_sql, &[&params.id]).await {
                    Ok(stream) => match stream.into_row().await {
                        Ok(Some(row)) => Some(crate::handlers::base_data::row_to_json(&row)),
                        _ => None,
                    },
                    Err(_) => None,
                }
            };
            let before_json = before_snapshot.as_ref().and_then(|v| serde_json::to_string(v).ok());
            let after_json = after_snapshot.as_ref().and_then(|v| serde_json::to_string(v).ok());
            inventory_ledger::record_oper_with_data(
                &mut conn, "UPDATE", &params.table, &params.id,
                &claims.user_code, None, Some("修改记录"),
                before_json.as_deref(), after_json.as_deref(),
            ).await;
            Json(ApiResponse::msg("更新成功"))
        }
        Err(e) => Json(ApiResponse::err(&format!("更新表 [{}] 数据失败: {}", params.table, e)))
    }
}

#[derive(Deserialize)]
pub struct GenericImportParams {
    pub table: String,
    pub data: Vec<serde_json::Map<String, serde_json::Value>>,
}

pub async fn generic_import(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenericImportParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止向系统表导入恶意数据
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code("系统敏感表禁止导入数据", PERMISSION_DENIED_TABLE));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.data.is_empty() {
        return Json(ApiResponse::err("没有提供导入数据"));
    }

    // 限制单次导入行数，避免事务过大
    const MAX_IMPORT_ROWS: usize = 5000;
    if params.data.len() > MAX_IMPORT_ROWS {
        return Json(ApiResponse::err(&format!(
            "单次导入数据不能超过 {} 行，当前 {} 行，请分批导入",
            MAX_IMPORT_ROWS, params.data.len()
        )));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    let pk_col = get_primary_key_for_table(&params.table);
    // 查询表结构以补全 NOT NULL 默认值
    let table_columns = fetch_table_columns(&mut conn, &params.table).await;

    // 识别唯一字段（编码类字段，如 *No, *Code, BarCode）用于查重
    let unique_fields: Vec<String> = table_columns.iter()
        .filter(|c| {
            let name = c.name.to_lowercase();
            name.ends_with("no") || name.ends_with("code") || name == "barcode"
        })
        .map(|c| c.name.clone())
        .collect();

    // 预查询已存在的唯一值（避免逐行查询）
    let mut existing_unique: std::collections::HashMap<String, std::collections::HashSet<String>> = std::collections::HashMap::new();
    for field in &unique_fields {
        let sql = format!("SELECT [{}] FROM [{}]", field, params.table);
        if let Ok(stream) = conn.query(&sql, &[]).await {
            if let Ok(rows) = stream.into_first_result().await {
                let field_name = field.as_str();
                let set: std::collections::HashSet<String> = rows.iter()
                    .filter_map(|r| {
                        // 用 try_get 避免类型不匹配 panic（返回 Result<Option<T>, Error>）
                        if let Ok(Some(s)) = r.try_get::<&str, _>(field_name) {
                            return Some(s.to_lowercase());
                        }
                        if let Ok(Some(i)) = r.try_get::<i32, _>(field_name) {
                            return Some(i.to_string().to_lowercase());
                        }
                        if let Ok(Some(i)) = r.try_get::<i64, _>(field_name) {
                            return Some(i.to_string().to_lowercase());
                        }
                        None
                    })
                    .collect();
                existing_unique.insert(field.clone(), set);
            }
        }
    }

    // 开启事务：保证全部成功或全部回滚
    if let Err(e) = inventory_ledger::begin_tran(&mut conn).await {
        return Json(ApiResponse::err(&format!("开启事务失败: {}", e)));
    }

    let mut success_count = 0u32;
    let mut error_msgs: Vec<String> = Vec::new();
    let mut imported_unique: std::collections::HashMap<String, std::collections::HashSet<String>> = std::collections::HashMap::new();

    for (row_idx, row) in params.data.iter().enumerate() {
        let row_no = row_idx + 1;  // 1-based 行号
        // 克隆行数据，便于补充默认值
        let mut row_data = row.clone();

        // 唯一性校验（编码字段）
        let mut dup_error: Option<String> = None;
        for field in &unique_fields {
            if let Some(val) = row_data.get(field) {
                let val_str = match val {
                    serde_json::Value::String(s) => s.trim().to_string(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                if val_str.is_empty() { continue; }
                let val_lower = val_str.to_lowercase();
                // 检查数据库已存在
                if let Some(set) = existing_unique.get(field) {
                    if set.contains(&val_lower) {
                        dup_error = Some(format!("第{}行: {}={} 已存在于系统中", row_no, field, val_str));
                        break;
                    }
                }
                // 检查本次导入已存在
                let imported_set = imported_unique.entry(field.clone()).or_insert_with(std::collections::HashSet::new);
                if imported_set.contains(&val_lower) {
                    dup_error = Some(format!("第{}行: {}={} 与本次导入的其他行重复", row_no, field, val_str));
                    break;
                }
                imported_set.insert(val_lower);
            }
        }
        if let Some(err) = dup_error {
            error_msgs.push(err);
            continue;
        }

        // 自动生成主键 UUID（如果主键为空且该表有已知 PK）
        if let Some(pk) = pk_col {
            let need_gen = match row_data.get(pk) {
                None => true,
                Some(serde_json::Value::Null) => true,
                Some(serde_json::Value::String(s)) => s.trim().is_empty(),
                _ => false,
            };
            if need_gen {
                row_data.insert(pk.to_string(), serde_json::Value::String(uuid::Uuid::new_v4().to_string()));
            }
        }

        // 补全 NOT NULL 字段默认值（排除主键，主键已处理）
        for col_info in &table_columns {
            if col_info.is_nullable { continue; }
            if col_info.name.eq_ignore_ascii_case("LUTime") { continue; }
            if let Some(pk) = pk_col {
                if col_info.name.eq_ignore_ascii_case(pk) { continue; }
            }
            // 如果用户已提供非空值，跳过
            let has_value = match row_data.get(&col_info.name) {
                Some(serde_json::Value::Null) => false,
                Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
                Some(_) => true,
                None => false,
            };
            if has_value { continue; }
            // 根据数据类型补默认值
            let default_val = default_value_for_type(&col_info.data_type, &col_info.name);
            if let Some(dv) = default_val {
                row_data.insert(col_info.name.clone(), serde_json::Value::String(dv));
            }
        }

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut values: Vec<Option<String>> = Vec::new();

        for (key, val) in row_data.iter() {
            // Skip fields that come from JOIN (not own columns)
            if join_fields.contains(&key.as_str()) { continue; }
            // Skip IDENTITY / computed columns（readonly_fields 统一小写，需忽略大小写比较）
            if readonly_fields.iter().any(|r| r.eq_ignore_ascii_case(key)) { continue; }
            // Skip fields not in table schema
            if !table_columns.iter().any(|c| c.name.eq_ignore_ascii_case(key)) { continue; }
            columns.push(format!("[{}]", key));
            placeholders.push(format!("@p{}", columns.len()));
            values.push(json_to_sql_value(val));
        }

        if columns.is_empty() {
            error_msgs.push(format!("第{}行: 无可插入字段", row_no));
            continue;
        }

        let sql = format!(
            "INSERT INTO [{}] ({}) VALUES ({})",
            params.table,
            columns.join(", "),
            placeholders.join(", ")
        );

        let param_refs: Vec<&dyn tiberius::ToSql> = values.iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                let err_str = format!("{}", e);
                // 友好化常见错误
                let friendly = if err_str.contains("Violation of PRIMARY KEY") {
                    format!("第{}行: 主键冲突", row_no)
                } else if err_str.contains("Violation of UNIQUE KEY constraint") {
                    let field_hint = err_str.split("'").nth(1).unwrap_or("唯一约束");
                    format!("第{}行: 唯一约束冲突（{}）", row_no, field_hint)
                } else if err_str.contains("The INSERT statement conflicted with the FOREIGN KEY constraint") {
                    let fk_hint = err_str.split("'").nth(1).unwrap_or("外键约束");
                    format!("第{}行: 关联数据不存在（{}），请检查编码是否正确", row_no, fk_hint)
                } else if err_str.contains("Conversion failed") {
                    format!("第{}行: 数据类型转换失败，请检查字段格式（{}）", row_no, err_str)
                } else if err_str.contains("Cannot insert the value NULL into column") {
                    let col_hint = err_str.split("'").nth(3).unwrap_or("未知列");
                    format!("第{}行: 必填字段 {} 不能为空", row_no, col_hint)
                } else if err_str.contains("String or binary data would be truncated") {
                    format!("第{}行: 数据长度超过字段限制，请检查文本长度", row_no)
                } else {
                    format!("第{}行: {}", row_no, err_str)
                };
                error_msgs.push(friendly);
            }
        }
    }

    // 根据结果决定提交或回滚
    if error_msgs.is_empty() {
        // 全部成功，提交事务
        if let Err(e) = inventory_ledger::commit_tran(&mut conn).await {
            let _ = inventory_ledger::rollback_tran(&mut conn).await;
            return Json(ApiResponse::err(&format!("提交事务失败: {}", e)));
        }
        // 写操作日志
        let _ = inventory_ledger::record_oper(
            &mut conn, "IMPORT", &params.table, "",
            &claims.user_code, None,
            Some(&format!("批量导入{}条记录", success_count)),
        ).await;
        Json(ApiResponse::msg(&format!("成功导入{}条记录", success_count)))
    } else {
        // 有错误，回滚事务
        inventory_ledger::rollback_tran(&mut conn).await;
        Json(ApiResponse::ok(serde_json::json!({
            "imported": 0,
            "failed": error_msgs.len(),
            "errors": error_msgs,
            "rolled_back": true,
        })))
    }
}

/// 表列信息（用于导入时补全 NOT NULL 默认值）
struct TableColInfo {
    name: String,
    data_type: String,
    is_nullable: bool,
}

/// 查询表列信息
async fn fetch_table_columns(conn: &mut bb8::PooledConnection<'static, bb8_tiberius::ConnectionManager>, table: &str) -> Vec<TableColInfo> {
    let sql = "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @p1 ORDER BY ORDINAL_POSITION";
    let stream = match conn.query(sql, &[&table]).await {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.iter().map(|row| {
        let name: String = row.get::<&str, _>("COLUMN_NAME").unwrap_or("").to_string();
        let data_type: String = row.get::<&str, _>("DATA_TYPE").unwrap_or("").to_string();
        let is_nullable: bool = row.get::<&str, _>("IS_NULLABLE").unwrap_or("NO").eq_ignore_ascii_case("YES");
        TableColInfo { name, data_type, is_nullable }
    }).collect()
}

/// 根据数据类型返回 NOT NULL 字段的默认值
fn default_value_for_type(data_type: &str, col_name: &str) -> Option<String> {
    let dt = data_type.to_lowercase();
    match dt.as_str() {
        "uniqueidentifier" => Some("00000000-0000-0000-0000-000000000000".to_string()),
        "bit" | "tinyint" | "smallint" | "int" | "bigint" => Some("0".to_string()),
        "decimal" | "numeric" | "money" | "smallmoney" | "float" | "real" => Some("0".to_string()),
        "char" | "varchar" | "nchar" | "nvarchar" | "text" | "ntext" => Some("".to_string()),
        "datetime" | "datetime2" | "smalldatetime" | "date" => {
            Some(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string())
        }
        "time" => Some("00:00:00".to_string()),
        _ => {
            // 对于未知类型，如果是 State 字段给 'N'，Used 字段给 'Y'
            if col_name.eq_ignore_ascii_case("State") { Some("N".to_string()) }
            else if col_name.eq_ignore_ascii_case("Used") { Some("Y".to_string()) }
            else if col_name.eq_ignore_ascii_case("ScanMode") { Some("N".to_string()) }
            else if col_name.eq_ignore_ascii_case("AccCheckFlg") { Some("0".to_string()) }
            else { None }
        }
    }
}

#[derive(Deserialize)]
pub struct BatchUpdateParams {
    pub table: String,
    pub primary_key: String,
    pub ids: Vec<String>,
    pub updates: serde_json::Map<String, serde_json::Value>,
}

pub async fn generic_batch_update(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<BatchUpdateParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    if !is_valid_identifier(&params.table) {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    if !is_valid_identifier(&params.primary_key) {
        return Json(ApiResponse::err_with_code("主键字段名只能包含字母、数字、下划线", VALIDATION_FIELD_INVALID));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过批量更新接口篡改系统表
    if is_table_blacklisted(&params.table, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("表 [{}] 为系统敏感表，禁止通过通用接口批量更新", params.table),
            PERMISSION_DENIED_TABLE,
        ));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    if params.ids.is_empty() {
        return Json(ApiResponse::err("请选择要更新的记录"));
    }
    if params.updates.is_empty() {
        return Json(ApiResponse::err("没有提供更新数据"));
    }

    // P0-S4 记录级越权防护：业务单据表（含 EUser 列）只能更新自己创建的记录
    if let Err(msg) = check_record_ownership(&mut conn, &params.table, &params.primary_key, &params.ids, &claims).await {
        return Json(ApiResponse::err_with_code(&msg, PERMISSION_DENIED_RECORD));
    }

    let join_fields = get_join_fields_for_table(&params.table);
    let readonly_fields = fetch_readonly_columns(&mut conn, &params.table).await;
    // BIT 字段列表：用于把 'Y'/'N' 字符串转成 1/0，避免 SQL Server 转换失败
    let bit_columns = fetch_bit_columns(&mut conn, &params.table).await;
    let mut set_clauses = Vec::new();
    let mut values: Vec<Option<String>> = Vec::new();

    for (key, val) in params.updates.iter() {
        // 字段名安全校验：防止 SQL 注入
        if !is_valid_identifier(key) { continue; }
        if key == &params.primary_key {
            continue;
        }
        // Skip fields that come from JOIN (not own columns)
        if join_fields.contains(&key.as_str()) { continue; }
        // Skip identity / computed columns
        if readonly_fields.iter().any(|r| r.eq_ignore_ascii_case(key)) { continue; }
        // BIT 字段特殊处理：把 'Y'/'N' 字符串转成 1/0，避免 SQL Server 转换失败
        let key_lc = key.to_lowercase();
        let val = if bit_columns.contains(&key_lc) {
            normalize_bit_value(val)
        } else {
            val.clone()
        };
        set_clauses.push(format!("[{}] = @p{}", key, set_clauses.len() + 1));
        values.push(json_to_sql_value(&val));
    }

    if set_clauses.is_empty() {
        return Json(ApiResponse::err("没有提供需要更新的字段"));
    }

    // 构建 IN (...) 占位符：WHERE pk IN (@pN, @pN+1, ...)
    let pk_start = values.len() + 1;
    let pk_placeholders: Vec<String> = (0..params.ids.len())
        .map(|i| format!("@p{}", pk_start + i))
        .collect();
    let sql = format!(
        "UPDATE [{}] SET {} WHERE [{}] IN ({})",
        params.table,
        set_clauses.join(", "),
        params.primary_key,
        pk_placeholders.join(", ")
    );

    let mut all_values = values.clone();
    for id in &params.ids {
        all_values.push(Some(id.clone()));
    }

    let param_refs: Vec<&dyn tiberius::ToSql> = all_values.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    // 显式事务：单条 UPDATE ... IN (...) 本身是原子的，
    // 但加上事务可确保异常时完全回滚，避免部分字段已写入的脏状态
    // 先查询每条记录的修改前数据快照
    let mut before_snapshots: Vec<(String, Option<String>)> = Vec::new();
    for id in &params.ids {
        let snap = query_row_snapshot_json(&mut conn, &params.table, &params.primary_key, id).await;
        before_snapshots.push((id.clone(), snap));
    }

    if let Err(e) = inventory_ledger::begin_tran(&mut conn).await {
        return Json(ApiResponse::err(&format!("开启事务失败: {}", e)));
    }

    let updated_count: i64 = match conn.execute(&sql, &param_refs).await {
        Ok(result) => result.rows_affected().iter().map(|&n| n as i64).sum(),
        Err(e) => {
            inventory_ledger::rollback_tran(&mut conn).await;
            tracing::warn!("批量更新失败: {:?}", e);
            return Json(ApiResponse::err(&format!("批量更新失败: {}", e)));
        }
    };

    if let Err(e) = inventory_ledger::commit_tran(&mut conn).await {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Json(ApiResponse::err(&format!("提交事务失败: {}", e)));
    }

    // 事务提交成功后记录操作日志：每条 ID 一条，含修改前数据快照
    // 日志写入失败不影响主操作结果（record_oper 内部已吞错）
    let field_names: Vec<&str> = params.updates.keys().map(|k| k.as_str()).collect();
    let remark = format!("批量修改 {} 个字段: {}", params.updates.len(), field_names.join("、"));
    let after_json = serde_json::to_string(&params.updates).ok();
    for (id, before_json) in &before_snapshots {
        inventory_ledger::record_oper_with_data(
            &mut conn,
            "BATCH_UPDATE",
            &params.table,
            id,
            &claims.user_code,
            None,
            Some(&remark),
            before_json.as_deref(),
            after_json.as_deref(),
        ).await;
    }

    Json(ApiResponse::ok(serde_json::json!({
        "updated_count": updated_count,
        "requested_count": params.ids.len(),
    })))
}

#[derive(Deserialize)]
pub struct ImportTemplateParams {
    pub table: String,
}

pub async fn generic_import_template(
    State(_config): State<Config>,
    Json(params): Json<ImportTemplateParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("数据库连接失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let sql = format!(
        "SELECT COLUMN_NAME, DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME = @p1 ORDER BY ORDINAL_POSITION"
    );
    let table_name = params.table.trim_start_matches('[').trim_start_matches(']');
    let stream = match conn.query(&sql, &[&table_name]).await {
        Ok(s) => s,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("查询表结构失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };
    let rows = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("读取列信息失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let mut headers = Vec::new();
    for row in &rows {
        let v = try_get_value(row, "COLUMN_NAME");
        if let serde_json::Value::String(s) = v {
            if !s.is_empty() {
                headers.push(s);
            }
        }
    }

    let csv = format!("\u{FEFF}{}\n", headers.join(","));

    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "text/csv; charset=utf-8")
        .header("Content-Disposition", format!("attachment; filename={}_template.csv", params.table))
        .body(axum::body::Body::from(csv))
        .unwrap()
}

pub async fn generic_import_excel(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    mut multipart: Multipart,
) -> Json<ApiResponse<serde_json::Value>> {
    let mut table_name = String::new();
    let mut file_data: Vec<u8> = Vec::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(f) => f,
            Err(e) => return Json(ApiResponse::err(&format!("读取上传文件失败: {}", e))),
        };
        let Some(field) = field else { break; };
        let name = field.name().unwrap_or("").to_string();
        if name == "table" {
            table_name = field.text().await.unwrap_or_default();
        } else if name == "file" {
            let bytes = match field.bytes().await {
                Ok(b) => b,
                Err(e) => return Json(ApiResponse::err(&format!("读取文件内容失败: {}", e))),
            };
            file_data = bytes.to_vec();
        }
    }

    if table_name.is_empty() || file_data.is_empty() {
        return Json(ApiResponse::err("缺少表名或文件"));
    }
    if !table_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过 Excel 导入向系统表写入数据
    if is_table_blacklisted(&table_name, &claims) {
        return Json(ApiResponse::err_with_code(
            &format!("表 [{}] 为系统敏感表，禁止通过通用接口导入", table_name),
            PERMISSION_DENIED_TABLE,
        ));
    }

    let text = match String::from_utf8(file_data.clone()) {
        Ok(s) => s,
        Err(_) => return Json(ApiResponse::err("文件编码不支持，请使用UTF-8编码的CSV文件")),
    };

    let lines: Vec<&str> = text.split('\n').filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return Json(ApiResponse::err("文件内容为空"));
    }

    let header_line = lines[0].trim().trim_start_matches('\u{FEFF}');
    let headers: Vec<&str> = parse_csv_line(header_line);

    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("数据库连接失败: {}", e))),
    };

    let join_fields = get_join_fields_for_table(&table_name);
    let readonly_fields = fetch_readonly_columns(&mut conn, &table_name).await;
    // 修复 SQL 注入：用表结构白名单过滤 CSV 表头，避免任意字符串拼入 INSERT 列名
    let table_columns = fetch_table_columns(&mut conn, &table_name).await;
    let mut success_count = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for (row_idx, line) in lines.iter().skip(1).enumerate() {
        let values = parse_csv_line(line.trim());
        let mut row_data = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            // 修复 SQL 注入：仅接受表中真实存在的列名，且必须是合法标识符
            if !is_valid_identifier(header) { continue; }
            if !table_columns.iter().any(|c| c.name.eq_ignore_ascii_case(header)) { continue; }
            let val = values.get(i).unwrap_or(&"");
            if !val.is_empty() {
                row_data.insert(header.to_string(), serde_json::Value::String(val.to_string()));
            }
        }

        if row_data.is_empty() {
            continue;
        }

        let mut columns = Vec::new();
        let mut placeholders = Vec::new();
        let mut sql_values: Vec<Option<String>> = Vec::new();

        for (key, val) in row_data.iter() {
            // Skip fields that come from JOIN (not own columns)
            if join_fields.contains(&key.as_str()) { continue; }
            // Skip IDENTITY / computed columns
            if readonly_fields.iter().any(|r| r.eq_ignore_ascii_case(key)) { continue; }
            columns.push(format!("[{}]", key));
            placeholders.push(format!("@p{}", columns.len()));
            sql_values.push(json_to_sql_value(val));
        }

        if columns.is_empty() {
            errors.push(format!("第{}行: 无可插入字段", row_idx + 2));
            continue;
        }

        let sql = format!(
            "INSERT INTO [{}] ({}) VALUES ({})",
            table_name,
            columns.join(", "),
            placeholders.join(", ")
        );

        let param_refs: Vec<&dyn tiberius::ToSql> = sql_values.iter()
            .map(|v| v as &dyn tiberius::ToSql)
            .collect();

        match conn.execute(&sql, &param_refs).await {
            Ok(_) => success_count += 1,
            Err(e) => {
                errors.push(format!("第{}行: {}", row_idx + 2, e));
            }
        }
    }

    // 写操作日志
    if success_count > 0 {
        let _ = inventory_ledger::record_oper(
            &mut conn, "IMPORT", &table_name, "",
            &claims.user_code, None,
            Some(&format!("Excel导入{}条记录", success_count)),
        ).await;
    }
    Json(ApiResponse::ok(serde_json::json!({
        "success_count": success_count,
        "error_count": errors.len(),
        "errors": errors
    })))
}

fn parse_csv_line(line: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;

    for (i, ch) in line.char_indices() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == ',' && !in_quotes {
            result.push(line[start..i].trim().trim_matches('"'));
            start = i + 1;
        }
    }
    result.push(line[start..].trim().trim_matches('"'));
    result
}

/// 表结构查询：从 INFORMATION_SCHEMA.COLUMNS 返回列信息
/// 供前端导入模板下载、列配置等场景使用
#[derive(Deserialize)]
pub struct GenericSchemaParams {
    pub table: String,
}

pub async fn generic_table_schema(
    State(_config): State<Config>,
    Json(params): Json<GenericSchemaParams>,
) -> Json<ApiResponse<serde_json::Value>> {
    // SQL Server 标识符安全：仅允许字母数字+下划线
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Json(ApiResponse::err_with_code("表名只能包含字母、数字、下划线", VALIDATION_TABLE_INVALID));
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return Json(ApiResponse::err(&format!("获取数据库连接失败: {}", e))),
    };
    let sql = "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, CHARACTER_MAXIMUM_LENGTH \
               FROM INFORMATION_SCHEMA.COLUMNS \
               WHERE TABLE_NAME = @p1 ORDER BY ORDINAL_POSITION";
    let rows = match conn.query(sql, &[&params.table]).await {
        Ok(stream) => match stream.into_first_result().await {
            Ok(r) => r,
            Err(e) => return Json(ApiResponse::err(&format!("查询表结构失败: {}", e))),
        },
        Err(e) => return Json(ApiResponse::err(&format!("查询表结构失败: {}", e))),
    };
    let columns: Vec<serde_json::Value> = rows.iter().map(|row| {
        let name = row.get::<&str, _>("COLUMN_NAME").unwrap_or("").to_string();
        let data_type = row.get::<&str, _>("DATA_TYPE").unwrap_or("").to_string();
        let is_nullable = row.get::<&str, _>("IS_NULLABLE").unwrap_or("NO").eq_ignore_ascii_case("YES");
        let max_len: Option<i32> = row.try_get("CHARACTER_MAXIMUM_LENGTH").ok().flatten();
        serde_json::json!({
            "name": name,
            "data_type": data_type,
            "is_nullable": is_nullable,
            "max_length": max_len,
        })
    }).collect();
    Json(ApiResponse::ok(serde_json::json!({
        "table": params.table,
        "columns": columns,
    })))
}

#[derive(Deserialize)]
pub struct ExportExcelParams {
    pub table: String,
    pub keyword: Option<String>,
    // 兼容前端 camelCase（keywordFields）和 snake_case（keyword_fields）两种写法
    #[serde(alias = "keywordFields")]
    pub keyword_fields: Option<Vec<String>>,
    pub wheres: Option<Vec<WhereCondition>>,
    pub include_deleted: Option<bool>,
    pub only_deleted: Option<bool>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub async fn generic_export_excel(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<ExportExcelParams>,
) -> Response {
    if !params.table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        let body = serde_json::json!({"success":false,"message":"表名只能包含字母、数字、下划线"}).to_string();
        return axum::response::Response::builder()
            .status(400)
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
    }
    // P0-S2 修复：补 is_table_blacklisted 校验，防止通过 Excel 导出读取系统表敏感数据
    if is_table_blacklisted(&params.table, &claims) {
        let body = serde_json::json!({"success":false,"code":PERMISSION_DENIED_TABLE,"message":format!("表 [{}] 为系统敏感表，禁止通过通用接口导出", params.table)}).to_string();
        return axum::response::Response::builder()
            .status(403)
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap();
    }
    if let Err(msg) = validate_query_params(&params.table, &params.keyword_fields, &params.wheres) {
        let body = serde_json::json!({"success":false,"message":msg.as_str()}).to_string();
        return axum::response::Response::builder().status(400).header("Content-Type", "application/json").body(axum::body::Body::from(body)).unwrap();
    }
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("数据库连接失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    let built = build_base_query(&params.table, &params.keyword, &params.keyword_fields, &params.wheres, params.include_deleted.unwrap_or(false), params.only_deleted.unwrap_or(false), &None);
    let export_sql = format!("SELECT TOP 50000 * FROM ({}) t", built.sql);
    let param_refs: Vec<&dyn tiberius::ToSql> = built.params.iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let data_stream = match conn.query(&export_sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("导出查询失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };
    let rows = match data_stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => {
            let body = serde_json::json!({"success":false,"message":&format!("读取数据失败: {}", e)}).to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap();
        }
    };

    if rows.is_empty() {
        // 空结果也记录导出日志（便于审计"谁点了导出"）
        let _ = inventory_ledger::record_oper(
            &mut conn, "EXPORT", &params.table, "",
            &claims.user_code, None, Some("导出0条记录"),
        ).await;
        let csv = "\u{FEFF}\n".to_string();
        return axum::response::Response::builder()
            .status(200)
            .header("Content-Type", "text/csv; charset=utf-8")
            .header("Content-Disposition", format!("attachment; filename={}_export.csv", params.table))
            .body(axum::body::Body::from(csv))
            .unwrap();
    }

    let columns = rows[0].columns();
    let headers: Vec<String> = columns.iter()
        .filter(|c| c.name() != "_rn")
        .map(|c| c.name().to_string())
        .collect();

    let mut csv = format!("\u{FEFF}{}\n", headers.join(","));

    for row in &rows {
        let vals: Vec<String> = headers.iter().map(|h| {
            let v = try_get_value(row, h);
            match v {
                serde_json::Value::String(s) => {
                    if s.contains(',') || s.contains('"') || s.contains('\n') {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s
                    }
                }
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            }
        }).collect();
        csv.push_str(&format!("{}\n", vals.join(",")));
    }

    // 记录导出日志（含行数，便于审计大量数据导出场景）
    let export_remark = format!("导出{}条记录(CSV)", rows.len());
    let _ = inventory_ledger::record_oper(
        &mut conn, "EXPORT", &params.table, "",
        &claims.user_code, None, Some(&export_remark),
    ).await;

    let resp = axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "text/csv; charset=utf-8")
        .header("Content-Disposition", format!("attachment; filename={}_export.csv", params.table))
        .body(axum::body::Body::from(csv))
        .unwrap();
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_valid_identifier_ok() {
        assert!(is_valid_identifier("GDSID"));
        assert!(is_valid_identifier("tBas_Goods"));
        assert!(is_valid_identifier("abc123"));
        assert!(is_valid_identifier("_under_score"));
    }

    #[test]
    fn test_is_valid_identifier_reject_empty() {
        assert!(!is_valid_identifier(""));
    }

    #[test]
    fn test_is_valid_identifier_reject_special() {
        assert!(!is_valid_identifier("GDSID]; DROP TABLE"));
        assert!(!is_valid_identifier("name--"));
        assert!(!is_valid_identifier("a b"));
        assert!(!is_valid_identifier("a;b"));
        assert!(!is_valid_identifier("a.b"));
        assert!(!is_valid_identifier("中文"));
        assert!(!is_valid_identifier("a'OR'1'='1"));
    }

    #[test]
    fn test_validate_query_params_ok() {
        assert!(validate_query_params("tBas_Goods", &None, &None).is_ok());
        let fields = Some(vec!["GDSNO".to_string(), "GDSDesc".to_string()]);
        assert!(validate_query_params("tBas_Goods", &fields, &None).is_ok());
    }

    #[test]
    fn test_validate_query_params_empty_table() {
        assert!(validate_query_params("", &None, &None).is_err());
    }

    #[test]
    fn test_validate_query_params_invalid_table() {
        let err = validate_query_params("tBas_Goods; DROP TABLE tBas_Goods", &None, &None);
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_query_params_invalid_keyword_field() {
        let fields = Some(vec!["GDSNO".to_string(), "bad name".to_string()]);
        let err = validate_query_params("tBas_Goods", &fields, &None);
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("bad name"));
    }

    #[test]
    fn test_validate_query_params_invalid_where_field() {
        let wc = WhereCondition {
            field: "col; --".to_string(),
            op: "eq".to_string(),
            value: json!("x"),
        };
        let err = validate_query_params("tBas_Goods", &None, &Some(vec![wc]));
        assert!(err.is_err());
    }

    #[test]
    fn test_validate_identifiers_ok() {
        assert!(validate_identifiers(&["GDSID", "State"]).is_ok());
        assert!(validate_identifiers(&[]).is_ok());
    }

    #[test]
    fn test_validate_identifiers_empty() {
        assert!(validate_identifiers(&[""]).is_err());
    }

    #[test]
    fn test_validate_identifiers_invalid() {
        assert!(validate_identifiers(&["GDSID", "bad name"]).is_err());
    }

    #[test]
    fn test_json_to_sql_value_null() {
        assert_eq!(json_to_sql_value(&json!(null)), None);
    }

    #[test]
    fn test_json_to_sql_value_string() {
        assert_eq!(json_to_sql_value(&json!("hello")), Some("hello".to_string()));
        assert_eq!(json_to_sql_value(&json!("")), None);
        assert_eq!(json_to_sql_value(&json!("   ")), None);
        assert_eq!(json_to_sql_value(&json!("\t\n")), None);
    }

    #[test]
    fn test_json_to_sql_value_number() {
        assert_eq!(json_to_sql_value(&json!(123)), Some("123".to_string()));
        assert_eq!(json_to_sql_value(&json!(12.34)), Some("12.34".to_string()));
        assert_eq!(json_to_sql_value(&json!(0)), Some("0".to_string()));
    }

    #[test]
    fn test_json_to_sql_value_bool() {
        assert_eq!(json_to_sql_value(&json!(true)), Some("1".to_string()));
        assert_eq!(json_to_sql_value(&json!(false)), Some("0".to_string()));
    }

    #[test]
    fn test_value_to_string_string() {
        assert_eq!(value_to_string(&json!("abc")), "abc");
        assert_eq!(value_to_string(&json!("")), "");
    }

    #[test]
    fn test_value_to_string_number() {
        assert_eq!(value_to_string(&json!(123)), "123");
        assert_eq!(value_to_string(&json!(12.5)), "12.5");
    }

    #[test]
    fn test_default_value_for_type_uniqueidentifier() {
        let v = default_value_for_type("uniqueidentifier", "X").unwrap();
        assert_eq!(v, "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn test_default_value_for_type_integers() {
        for t in &["bit", "tinyint", "smallint", "int", "bigint"] {
            assert_eq!(default_value_for_type(t, "X"), Some("0".to_string()));
        }
    }

    #[test]
    fn test_default_value_for_type_numeric() {
        for t in &["decimal", "numeric", "money", "smallmoney", "float", "real"] {
            assert_eq!(default_value_for_type(t, "X"), Some("0".to_string()));
        }
    }

    #[test]
    fn test_default_value_for_type_string() {
        for t in &["char", "varchar", "nchar", "nvarchar", "text", "ntext"] {
            assert_eq!(default_value_for_type(t, "X"), Some("".to_string()));
        }
    }

    #[test]
    fn test_default_value_for_type_datetime() {
        let v = default_value_for_type("datetime", "X").unwrap();
        assert!(v.len() >= 19);
        assert_eq!(v.chars().nth(4), Some('-'));
        assert_eq!(v.chars().nth(10), Some('T'));
    }

    #[test]
    fn test_default_value_for_type_unknown_state() {
        assert_eq!(default_value_for_type("unknown_type", "State"), Some("N".to_string()));
        assert_eq!(default_value_for_type("unknown_type", "Used"), Some("Y".to_string()));
        assert_eq!(default_value_for_type("unknown_type", "ScanMode"), Some("N".to_string()));
        assert_eq!(default_value_for_type("unknown_type", "AccCheckFlg"), Some("0".to_string()));
        assert_eq!(default_value_for_type("unknown_type", "Other"), None);
    }

    #[test]
    fn test_parse_csv_line_simple() {
        assert_eq!(parse_csv_line("a,b,c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_parse_csv_line_quoted() {
        assert_eq!(parse_csv_line("\"a,b\",c"), vec!["a,b", "c"]);
    }

    #[test]
    fn test_parse_csv_line_trimmed() {
        assert_eq!(parse_csv_line(" a , b , c "), vec!["a", "b", "c"]);
    }

    #[test]
    fn test_parse_csv_line_single() {
        assert_eq!(parse_csv_line("single"), vec!["single"]);
    }

    #[test]
    fn test_parse_csv_line_empty() {
        assert_eq!(parse_csv_line(""), vec![""]);
    }

    #[test]
    fn test_get_primary_key_known() {
        assert_eq!(get_primary_key_for_table("tPur_Order"), Some("POID"));
        assert_eq!(get_primary_key_for_table("tBas_Goods"), Some("GDSID"));
        assert_eq!(get_primary_key_for_table("tBas_Supp"), Some("SuppID"));
        assert_eq!(get_primary_key_for_table("tStk_IO"), Some("IOID"));
        assert_eq!(get_primary_key_for_table("tSys_Rule"), Some("RuleID"));
    }

    #[test]
    fn test_get_primary_key_alias() {
        assert_eq!(get_primary_key_for_table("tBas_Dictionary"), Some("DictID"));
        assert_eq!(get_primary_key_for_table("tBas_Dict"), Some("DictID"));
    }

    #[test]
    fn test_get_primary_key_unknown() {
        assert_eq!(get_primary_key_for_table("tNonExistent"), None);
    }

    #[test]
    fn test_get_state_field_used() {
        assert_eq!(get_state_field_for_table("tBas_Brand"), Some("Used"));
        assert_eq!(get_state_field_for_table("tBas_Stock"), Some("Used"));
        assert_eq!(get_state_field_for_table("tBas_GDSType"), Some("Used"));
        assert_eq!(get_state_field_for_table("tBas_Dept"), Some("Used"));
        assert_eq!(get_state_field_for_table("tSys_Menus"), Some("Used"));
    }

    #[test]
    fn test_get_state_field_state() {
        assert_eq!(get_state_field_for_table("tBas_Goods"), Some("State"));
        assert_eq!(get_state_field_for_table("tPur_Order"), Some("State"));
        assert_eq!(get_state_field_for_table("tStk_IO"), Some("State"));
        assert_eq!(get_state_field_for_table("tSys_Rule"), Some("State"));
    }

    #[test]
    fn test_get_state_field_none() {
        assert_eq!(get_state_field_for_table("tSys_OperHis"), None);
        assert_eq!(get_state_field_for_table("tSys_OperLog"), None);
        assert_eq!(get_state_field_for_table("tNonExistent"), None);
    }

    #[test]
    fn test_get_identity_columns() {
        assert_eq!(get_identity_columns_for_table("tBas_Goods"), vec!["gdsSD"]);
        assert_eq!(get_identity_columns_for_table("tBas_Supp"), vec!["suppSD"]);
        assert_eq!(get_identity_columns_for_table("tBas_Cust"), vec!["custSD"]);
        assert_eq!(get_identity_columns_for_table("tBas_Emp"), vec!["empSD"]);
        assert_eq!(get_identity_columns_for_table("tBas_Stock"), vec!["stkSD"]);
    }

    #[test]
    fn test_get_identity_columns_empty() {
        assert!(get_identity_columns_for_table("tPur_Order").is_empty());
        assert!(get_identity_columns_for_table("tNonExistent").is_empty());
    }

    #[test]
    fn test_default_empty_string_cols() {
        let s = default_empty_string_cols();
        assert!(s.contains("PHelp"));
        assert!(s.contains("PValue"));
        assert!(s.contains("CheckSQL"));
        assert!(s.contains("PTerm"));
        assert_eq!(s.len(), 4);
        assert!(!s.contains("GDSNO"));
    }

    #[test]
    fn test_get_field_prefix_tbas_goods() {
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "GDSTypeName"), "gt");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "BrandName"), "b");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "SuppName"), "s");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "UnitName"), "u");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "StkName"), "sk");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "GDSKindName"), "gk");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "GDSNO"), "t");
        assert_eq!(get_field_prefix_for_table("tBas_Goods", "GDSDesc"), "t");
    }

    #[test]
    fn test_get_field_prefix_unknown_table() {
        assert_eq!(get_field_prefix_for_table("tNonExistent", "Foo"), "t");
    }

    #[test]
    fn test_get_field_prefix_detail_tables() {
        assert_eq!(get_field_prefix_for_table("tStk_IODetail", "GDSNO"), "g");
        assert_eq!(get_field_prefix_for_table("tPur_OrderDetail", "GDSDesc"), "g");
        assert_eq!(get_field_prefix_for_table("tStk_IODetail", "UnitName"), "u");
        assert_eq!(get_field_prefix_for_table("tStk_IODetail", "BrandName"), "b");
    }

    #[test]
    fn test_build_base_query_simple_no_state() {
        // tSys_RptPrintHis 既无 state_field 也无 JOIN，是最简洁的 SELECT
        let q = build_base_query("tSys_RptPrintHis", &None, &None, &None, false, false, &None);
        assert!(q.sql.starts_with("SELECT t.* FROM [tSys_RptPrintHis] t"));
        assert!(!q.sql.contains("WHERE"));
        assert!(q.params.is_empty());
    }

    #[test]
    fn test_build_base_query_exclude_deleted_state() {
        let q = build_base_query("tPur_Order", &None, &None, &None, false, false, &None);
        // ★ State=NULL 也要显示：过滤条件为 (t.[State] <> 'D' OR t.[State] IS NULL)
        assert!(q.sql.contains("t.[State] <> 'D' OR t.[State] IS NULL"));
        assert!(q.sql.contains("WHERE"));
    }

    #[test]
    fn test_build_base_query_include_deleted_no_filter() {
        let q = build_base_query("tPur_Order", &None, &None, &None, true, false, &None);
        assert!(!q.sql.contains("State <> 'D'"));
        assert!(!q.sql.contains("WHERE"));
    }

    #[test]
    fn test_build_base_query_only_deleted() {
        let q = build_base_query("tPur_Order", &None, &None, &None, false, true, &None);
        assert!(q.sql.contains("t.[State] = 'D'"));
    }

    #[test]
    fn test_build_base_query_used_field_table() {
        let q = build_base_query("tBas_Brand", &None, &None, &None, false, false, &None);
        assert!(q.sql.contains("t.[Used] <> 'N'"));
    }

    #[test]
    fn test_build_base_query_used_field_only_deleted() {
        let q = build_base_query("tBas_Brand", &None, &None, &None, false, true, &None);
        assert!(q.sql.contains("t.[Used] = 'N'"));
    }

    #[test]
    fn test_build_base_query_keyword_like() {
        let kw = Some("洗发水".to_string());
        let fields = Some(vec!["GDSNO".to_string(), "GDSDesc".to_string()]);
        let q = build_base_query("tBas_Goods", &kw, &fields, &None, false, false, &None);
        assert!(q.sql.contains("LIKE @p1"));
        assert!(q.sql.contains("LIKE @p2"));
        assert!(q.sql.contains("OR"));
        assert!(q.sql.contains("t.[GDSNO]"));
        assert!(q.sql.contains("t.[GDSDesc]"));
        assert_eq!(q.params.len(), 2);
        assert_eq!(q.params[0], Some("%洗发水%".to_string()));
        assert_eq!(q.params[1], Some("%洗发水%".to_string()));
    }

    #[test]
    fn test_build_base_query_keyword_with_join_prefix() {
        let kw = Some("X".to_string());
        let fields = Some(vec!["GDSNO".to_string()]);
        let q = build_base_query("tStk_Qty", &kw, &fields, &None, false, false, &None);
        assert!(q.sql.contains("g.[GDSNO]"));
    }

    #[test]
    fn test_build_base_query_where_eq() {
        let wc = WhereCondition {
            field: "GDSStateNO".to_string(),
            op: "eq".to_string(),
            value: json!(1),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), false, false, &None);
        assert!(q.sql.contains("t.[GDSStateNO] = @p1"));
        assert_eq!(q.params, vec![Some("1".to_string())]);
    }

    #[test]
    fn test_build_base_query_where_ne() {
        let wc = WhereCondition {
            field: "State".to_string(),
            op: "ne".to_string(),
            value: json!("D"),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(q.sql.contains("t.[State] <> @p1"));
    }

    #[test]
    fn test_build_base_query_where_in_array() {
        let wc = WhereCondition {
            field: "GDSID".to_string(),
            op: "in".to_string(),
            value: json!(["a", "b", "c"]),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(q.sql.contains("IN (@p1, @p2, @p3)"));
        assert_eq!(q.params.len(), 3);
        assert_eq!(q.params[0], Some("a".to_string()));
        assert_eq!(q.params[1], Some("b".to_string()));
        assert_eq!(q.params[2], Some("c".to_string()));
    }

    #[test]
    fn test_build_base_query_where_in_string_csv() {
        let wc = WhereCondition {
            field: "GDSID".to_string(),
            op: "in".to_string(),
            value: json!("a,b,c"),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(q.sql.contains("IN (@p1, @p2, @p3)"));
        assert_eq!(q.params.len(), 3);
    }

    #[test]
    fn test_build_base_query_where_in_empty_array() {
        let wc = WhereCondition {
            field: "GDSID".to_string(),
            op: "in".to_string(),
            value: json!([]),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(q.sql.contains("1=0"));
        assert!(q.params.is_empty());
    }

    #[test]
    fn test_build_base_query_where_like() {
        let wc = WhereCondition {
            field: "GDSDesc".to_string(),
            op: "like".to_string(),
            value: json!("洗发"),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(q.sql.contains("t.[GDSDesc] LIKE @p1"));
        assert_eq!(q.params, vec![Some("%洗发%".to_string())]);
    }

    #[test]
    fn test_build_base_query_where_value_y_n_cast() {
        let wc = WhereCondition {
            field: "Used".to_string(),
            op: "eq".to_string(),
            value: json!("Y"),
        };
        let q = build_base_query("tBas_Brand", &None, &None, &Some(vec![wc]), false, false, &None);
        assert!(q.sql.contains("CAST(t.[Used] AS nvarchar(1)) = @p"));
    }

    #[test]
    fn test_build_base_query_where_invalid_field_skipped() {
        let wc = WhereCondition {
            field: "bad name".to_string(),
            op: "eq".to_string(),
            value: json!("x"),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc]), true, false, &None);
        assert!(!q.sql.contains("bad name"));
    }

    #[test]
    fn test_build_base_query_warehouse_id_triggers_join() {
        let wid = Some("warehouse-uuid-1".to_string());
        let q = build_base_query("tBas_Goods", &None, &None, &None, true, false, &wid);
        assert!(q.sql.contains("StockQty"));
        assert!(q.sql.contains("QQty"));
        assert!(q.sql.contains("LEFT JOIN [tStk_Stock] st ON t.[GDSID] = st.[GDSID] AND st.[StkID] = @p1"));
        assert_eq!(q.params, vec![Some("warehouse-uuid-1".to_string())]);
    }

    #[test]
    fn test_build_base_query_warehouse_id_empty_skipped() {
        let wid = Some("".to_string());
        let q = build_base_query("tBas_Goods", &None, &None, &None, true, false, &wid);
        assert!(!q.sql.contains("StockQty"));
        assert!(q.params.is_empty());
    }

    #[test]
    fn test_build_base_query_warehouse_id_other_table_skipped() {
        let wid = Some("warehouse-uuid-1".to_string());
        let q = build_base_query("tBas_Supp", &None, &None, &None, true, false, &wid);
        assert!(!q.sql.contains("StockQty"));
        assert!(q.params.is_empty());
    }

    #[test]
    fn test_build_base_query_multiple_conditions_joined_with_and() {
        let wc1 = WhereCondition {
            field: "GDSStateNO".to_string(),
            op: "eq".to_string(),
            value: json!(1),
        };
        let wc2 = WhereCondition {
            field: "GDSDesc".to_string(),
            op: "like".to_string(),
            value: json!("洗发"),
        };
        let q = build_base_query("tBas_Goods", &None, &None, &Some(vec![wc1, wc2]), false, false, &None);
        let and_count = q.sql.matches("AND").count();
        assert_eq!(and_count, 2);
    }

    #[test]
    fn test_build_base_query_join_clause_present() {
        let q = build_base_query("tPur_Order", &None, &None, &None, true, false, &None);
        assert!(q.sql.contains("LEFT JOIN [tBas_Supp] s"));
        assert!(q.sql.contains("LEFT JOIN [tBas_Dept] d"));
        assert!(q.sql.contains("SuppName"));
    }

    #[test]
    fn test_build_base_query_param_indexing_sequential() {
        let kw = Some("X".to_string());
        let fields = Some(vec!["GDSNO".to_string(), "GDSDesc".to_string()]);
        let wc = WhereCondition {
            field: "GDSStateNO".to_string(),
            op: "eq".to_string(),
            value: json!(1),
        };
        let q = build_base_query("tBas_Goods", &kw, &fields, &Some(vec![wc]), false, false, &None);
        assert_eq!(q.params.len(), 3);
        assert!(q.sql.contains("@p1"));
        assert!(q.sql.contains("@p2"));
        assert!(q.sql.contains("@p3"));
        assert!(!q.sql.contains("@p4"));
    }

    #[test]
    fn test_get_references_for_tbas_goods() {
        let refs = get_references_for_table("tBas_Goods");
        assert!(!refs.is_empty());
        for (ref_table, ref_field, label) in &refs {
            assert!(!ref_table.is_empty());
            assert!(!ref_field.is_empty());
            assert!(!label.is_empty());
        }
    }

    #[test]
    fn test_get_references_for_unknown_table_empty() {
        let refs = get_references_for_table("tNonExistent");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_get_join_fields_for_tbas_goods() {
        let fields = get_join_fields_for_table("tBas_Goods");
        assert!(fields.contains(&"GDSTypeName"));
        assert!(fields.contains(&"BrandName"));
        assert!(fields.contains(&"SuppName"));
        assert!(fields.contains(&"UnitName"));
    }

    #[test]
    fn test_get_join_fields_for_unknown_table_empty() {
        let fields = get_join_fields_for_table("tNonExistent");
        assert!(fields.is_empty());
    }
}
