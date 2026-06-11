use axum::{extract::State, Extension, Json};
use serde::Deserialize;
use bb8::PooledConnection;
use bb8_tiberius::ConnectionManager;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};
use crate::middleware::auth::Claims;
use crate::handlers::doc_state;

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

pub type Conn = PooledConnection<'static, ConnectionManager>;

/// 公共操作日志写入器
/// OperType: APPROVE / UNAPPROVE / PRINT / CREATE / UPDATE / DELETE / POST
pub async fn write_oper_log(
    conn: &mut Conn,
    oper_type: &str,
    table_name: &str,
    key_value: &str,
    user_code: &str,
    remark: Option<&str>,
) {
    let sql = "INSERT INTO tSys_OperHis (OperType, TableName, KeyValue, OperUser, OperDate, Remark) \
               VALUES (@p1, @p2, @p3, @p4, @p5, @p6)";
    let now = chrono::Local::now().naive_local();
    let remark_owned = remark.unwrap_or("").to_string();
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &oper_type,
        &table_name,
        &key_value,
        &user_code,
        &now,
        &remark_owned,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        eprintln!("[write_oper_log] 写入 tSys_OperHis 失败: table={}, key={}, err={}", table_name, key_value, e);
    }
}

#[derive(Deserialize)]
pub struct ApproveParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub doc_type: Option<String>,
}

#[derive(Deserialize)]
pub struct PrintLogParams {
    pub table: String,
    pub primary_key: String,
    pub id: String,
    pub copies: Option<i32>,
}

fn resolve_detail_meta(table: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match table {
        "tSal_Order" => Some(("tSal_OrderDetail", "SODetailID", "SOID")),
        "tSal_Inv" => Some(("tSal_InvDetail", "SIDetailID", "SIID")),
        // 销售退货实际存于 tStk_IO + tStk_IODetail，Kind='SR'
        "sales_return" | "tStk_IO:sr" => Some(("tStk_IODetail", "IODetailID", "IOID")),
        "tPur_Order" => Some(("tPur_OrderDetail", "PODetailID", "POID")),
        "tPur_Inv" => Some(("tPur_InvDetail", "PIDetailID", "PIID")),
        "tPur_Return" => Some(("tPur_ReturnDetail", "PRDetailID", "PRID")),
        "tStk_IO" => Some(("tStk_IODetail", "IODetailID", "IOID")),
        "tStk_Move" => Some(("tStk_MoveDetail", "MoveDetailID", "MoveID")),
        "tStk_Tran" => Some(("tStk_TranDetail", "TranDetailID", "TranID")),
        "tStk_StockCycle" => Some(("tStk_StockCycleDetail", "CycleDetailID", "CycleID")),
        "tStk_ReplenishApply" => Some(("tStk_ReplenishApplyDetail", "ApplyDetailID", "ApplyID")),
        _ => None,
    }
}

/// 部分表的"业务单号"和"主表 PK"不同名，需要先做转换
/// 例如 tStk_IO：IONo (业务) → IOID (PK uniqueidentifier)
/// 返回 (主表名, 业务单号字段, 主表 PK 字段)
fn resolve_id_transform(table: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match table {
        "tStk_IO" => Some(("tStk_IO", "IONo", "IOID")),
        "tSal_Order" => Some(("tSal_Order", "SoNo", "SOID")),
        "tSal_Inv" => Some(("tSal_Inv", "SINo", "SIID")),
        // 销售退货单据号在 tStk_IO.IONo 上，复用 tStk_IO 的转换
        // "tSal_Return" 业务单据已统一到 tStk_IO (Kind='SR')
        "tPur_Order" => Some(("tPur_Order", "PoNo", "POID")),
        "tPur_Inv" => Some(("tPur_Inv", "PiNo", "PIID")),
        "tPur_Return" => Some(("tPur_Return", "PrNo", "PRID")),
        "tStk_Move" => Some(("tStk_Move", "MoveNo", "MoveID")),
        "tStk_Tran" => Some(("tStk_Tran", "TranNo", "TranID")),
        "tStk_StockCycle" => Some(("tStk_StockCycle", "CycleNo", "CycleID")),
        "tStk_ReplenishApply" => Some(("tStk_ReplenishApply", "ApplyNo", "ApplyID")),
        _ => None,
    }
}

/// 主表仓库字段（当 detail.StkID 为 NULL 时 fallback）
fn resolve_master_stk_field(table: &str) -> Option<(&'static str, &'static str)> {
    match table {
        "tStk_IO" => Some(("tStk_IO", "StkID")),
        "tSal_Order" => Some(("tSal_Order", "StkID")),
        "tSal_Inv" => Some(("tSal_Inv", "StkID")),
        "tPur_Order" => Some(("tPur_Order", "StkID")),
        _ => None,
    }
}

/// 盘点单：TranNo → TranID (UUID)
async fn resolve_tran_id(conn: &mut Conn, tran_no: &str) -> Option<String> {
    let sql = "SELECT CAST(TranID AS NVARCHAR(40)) AS T FROM tStk_Tran WHERE TranNo = @p1";
    let p: Vec<&dyn tiberius::ToSql> = vec![&tran_no];
    match conn.query(sql, &p).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(r)) => r.get::<&str, _>("T").map(|s| s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// 盘点单：TranID → StkID
async fn get_tran_stk_by_id(conn: &mut Conn, tran_id: &str) -> String {
    let sql = "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_Tran WHERE TranID = @p1";
    let p: Vec<&dyn tiberius::ToSql> = vec![&tran_id];
    match conn.query(sql, &p).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(r)) => r.get::<&str, _>("S").unwrap_or("").to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

/// 从单据详情抓取 (GDSID, StkID, Qty, DetailID)
async fn fetch_doc_detail_rows(
    conn: &mut Conn,
    table: &str,
    id: &str,
) -> Vec<(String, String, f64, String, String)> {
    let Some((detail_table, detail_pk, fk)) = resolve_detail_meta(table) else {
        return Vec::new();
    };
    // 业务单号 → 主表 uniqueidentifier 转换
    // 假设 fk 是主表 PK（uniqueidentifier），params.id 是业务单号
    let lookup_id = if let Some((master_table, business_key, master_pk)) = resolve_id_transform(table) {
        if master_pk == fk {
            // fk 是主表 PK，需要先查 IOID WHERE IONo = id
            let qsql = format!("SELECT CAST({} AS NVARCHAR(40)) AS PK FROM [{}] WHERE [{}] = @p1", master_pk, master_table, business_key);
            let qp: Vec<&dyn tiberius::ToSql> = vec![&id];
            match conn.query(&qsql, &qp).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("PK").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            }
        } else {
            id.to_string()
        }
    } else {
        id.to_string()
    };
    if lookup_id.is_empty() {
        eprintln!("[approve] fetch_doc_detail_rows: lookup_id 为空, table={}, id={}", table, id);
        return Vec::new();
    }
    let sql = format!(
        "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID, \
         CAST(Qty AS NVARCHAR(50)) AS Qty, CAST({} AS NVARCHAR(40)) AS DetailID, \
         CAST({} AS NVARCHAR(40)) AS MasterID \
         FROM [{}] WHERE {} = @p1",
        detail_pk, fk, detail_table, fk
    );
    let q_params: Vec<&dyn tiberius::ToSql> = vec![&lookup_id];
    let rows = match conn.query(&sql, &q_params).await {
        Ok(stream) => match stream.into_first_result().await {
            Ok(rs) => rs,
            Err(_) => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    // 缓存 master StkID（detail.StkID 为 NULL 时 fallback）
    let master_stk: String = if rows.iter().any(|r| r.get::<&str, _>("StkID").map(|s| s.is_empty()).unwrap_or(true)) {
        if let Some((mt, mf)) = resolve_master_stk_field(table) {
            let msql = format!("SELECT ISNULL(CAST({} AS NVARCHAR(40)),'') AS S FROM [{}] WHERE {} = @p1", mf, mt, fk);
            let mp: Vec<&dyn tiberius::ToSql> = vec![&lookup_id];
            match conn.query(&msql, &mp).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("S").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            }
        } else { String::new() }
    } else { String::new() };
    rows.iter()
        .map(|r| {
            let qty_str = r.get::<&str, _>("Qty").unwrap_or("0");
            let qty: f64 = qty_str.parse().unwrap_or(0.0);
            let stk = r.get::<&str, _>("StkID").unwrap_or("").to_string();
            let stk = if stk.is_empty() { master_stk.clone() } else { stk };
            (
                r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                stk,
                qty,
                r.get::<&str, _>("DetailID").unwrap_or("").to_string(),
                r.get::<&str, _>("MasterID").unwrap_or("").to_string(),
            )
        })
        .collect()
}



/// UPSERT tStk_Stock（Qty + QQty 同步），返回更新后的 Qty
pub async fn apply_stock_delta_qq(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    delta: f64,
) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Stock SET Qty = ISNULL(Qty,0) + @p3, QQty = ISNULL(QQty,0) + @p3
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty)
                 VALUES (NEWID(), @p1, @p2, @p3, @p3)"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid, &delta];
    if conn.execute(sql, &params).await.is_err() {
        return -1.0;
    }
    query_stock_qty(conn, gdsid, stkid).await
}

/// 读单据的业务日期列（用于反审会计期间检查）
async fn get_doc_action_date(conn: &mut Conn, table: &str, primary_key: &str, id: &str) -> Option<chrono::NaiveDate> {
    // 不同表对应的日期列名
    let date_col: &str = match table.to_lowercase().as_str() {
        s if s.contains("sal_order") => "OrderDate",
        s if s.contains("sal_inv") => "InvDate",
        s if s.contains("sal_return") => "RetDate",
        s if s.contains("pur_order") => "PODate",
        s if s.contains("pur_inv") => "RecvDate",
        s if s.contains("pur_return") => "RetDate",
        s if s.contains("stk_io") && !s.contains("detail") => "IoDate",
        s if s.contains("stk_move") => "MoveDate",
        s if s.contains("stk_tran") && !s.contains("detail") => "TranDate",
        s if s.contains("replenish") => "ApplyDate",
        _ => "EDate",  // 兜底取创建日期
    };
    let sql = format!("SELECT CAST({} AS DATE) AS D FROM [{}] WHERE [{}] = @p1", date_col, table, primary_key);
    let row_opt = match conn.query(&sql, &[&id]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(_) => None,
    };
    row_opt.and_then(|r| {
        // tiberius 的 Row::get 返回 Option<T>，用 unwrap_or 给默认值
        let default_date = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let d = r.get::<chrono::NaiveDate, _>("D").unwrap_or(default_date);
        let year_2000 = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        if d < year_2000 { None } else { Some(d) }
    })
}

/// 反审前置检查：会计期间是否已结账
/// DB 规则：月初 month_end_settle 后该月视为"已结账"，禁止反审
/// 判定方式：tStk_StockYM 中下月存在 InitQty>0 的记录 → 本月已结
async fn check_period_closed(conn: &mut Conn, action_date: chrono::NaiveDate) -> Option<String> {
    let ym: i32 = action_date.format("%Y%m").to_string().parse().unwrap_or(202501);
    // 计算下个月 YYYYMM
    let next_ym: i32 = if ym % 100 == 12 {
        (ym / 100 + 1) * 100 + 1
    } else {
        ym + 1
    };
    let sql = "SELECT TOP 1 ISNULL(InitQty, 0) AS IQ FROM tStk_StockYM WHERE AccYM = @p1 AND InitQty > 0";
    let row = match conn.query(sql, &[&next_ym]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(_) => return None,  // 查询失败 = 不阻断
    };
    if let Some(r) = row {
        let init_qty = row_get_f64(&r, "IQ");
        if init_qty > 0.0 {
            return Some(format!("会计期间 {} 已月结账，无法反审核", ym));
        }
    }
    None
}

/// 反审前置检查：是否被下游单据引用
/// DB 规则：被下游引用的单据禁止反审（防孤儿数据）
/// 当前覆盖：销售订单 / 销售出库 / 采购订单 / 采购入库
async fn check_downstream_exists(conn: &mut Conn, doc_type: &str, id: &str) -> Option<String> {
    match doc_type {
        "sales_order" => {
            // 已被 SD(tSal_Inv) 或 SR(tStk_IO Kind='SR') 引用 → 禁止反审
            let sql = "SELECT TOP 1 CAST(SIID AS NVARCHAR(40)) AS ID FROM tSal_Inv WHERE SOID = @p1";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该销售订单已被销售出库引用，无法反审核".to_string());
                    }
                }
            }
            // SR 实际是 tStk_IO，过滤 Kind='SR'
            let sql2 = "SELECT TOP 1 CAST(IOID AS NVARCHAR(40)) AS ID FROM tStk_IO WHERE SOID = @p1 AND Kind='SR' AND State NOT IN ('D','C')";
            if let Ok(s) = conn.query(sql2, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该销售订单已被销售退货引用，无法反审核".to_string());
                    }
                }
            }
        }
        "sales_outbound" => {
            // 已被 SR 引用（tStk_IODetail.SouID 指向本 SI）→ 禁止反审
            // tStk_IO Kind='SR' 是销售退货单据，tStk_IODetail.SouID 是源单据明细ID
            let sql = "SELECT TOP 1 d.IODetailID AS ID \
                       FROM tStk_IODetail d \
                       INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                       WHERE io.Kind='SR' AND d.SouID = @p1 AND io.State NOT IN ('D','C')";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该销售出库单已有退货引用，无法反审核".to_string());
                    }
                }
            }
        }
        "purchase_order" => {
            // 已被 RI(tPur_Inv) 或 PR(tStk_IO Kind='PR') 引用
            let sql = "SELECT TOP 1 CAST(PIID AS NVARCHAR(40)) AS ID FROM tPur_Inv WHERE POID = @p1";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该采购订单已有入库单引用，无法反审核".to_string());
                    }
                }
            }
            // 采购退货 = tStk_IO Kind IN ('PR','TH')，同时覆盖两种可能
            let sql2 = "SELECT TOP 1 CAST(IOID AS NVARCHAR(40)) AS ID FROM tStk_IO WHERE POID = @p1 AND Kind IN ('PR','TH') AND State NOT IN ('D','C')";
            if let Ok(s) = conn.query(sql2, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该采购订单已有退货单引用，无法反审核".to_string());
                    }
                }
            }
        }
        "purchase_inbound" => {
            // 已被 TH/PR 引用（tStk_IODetail.SouID 指向本 PI 明细）
            let sql = "SELECT TOP 1 d.IODetailID AS ID \
                       FROM tStk_IODetail d \
                       INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                       WHERE io.Kind IN ('PR','TH') AND d.SouID = @p1 AND io.State NOT IN ('D','C')";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该采购入库单已有退货引用，无法反审核".to_string());
                    }
                }
            }
        }
        // ===== 别名：与 sales_outbound 同样规则（SouID 被 SR 引用则禁止反审）=====
        "sales_inv" => {
            let sql = "SELECT TOP 1 d.IODetailID AS ID \
                       FROM tStk_IODetail d \
                       INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                       WHERE io.Kind='SR' AND d.SouID = @p1 AND io.State NOT IN ('D','C')";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该门店销售单已有退货引用，无法反审核".to_string());
                    }
                }
            }
        }
        // ===== 别名：与 purchase_inbound 同样规则（FromRID 被 PR 引用）=====
        "purchase_receipt" => {
            let sql = "SELECT TOP 1 CAST(PRID AS NVARCHAR(40)) AS ID FROM tPur_Return WHERE FromRID = @p1";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                        return Some("该采购收货单已有退货引用，无法反审核".to_string());
                    }
                }
            }
        }
        // ===== 别名：与 purchase_return 同样规则（TH 退货单的下游）=====
        "store_return" => {
            // 退货单一般无下游，跳过
        }
        // ===== IO 通用：检查是否被 SR 引用（tStk_IODetail.SouID）=====
        "stock_io" | "requisition" | "zp_delivery" | "oti_inbound" | "oto_outbound" => {
            // SD/SI: 检查是否被 SR(tStk_IODetail.SouID) 引用
            let kind = get_io_kind(conn, id).await;
            if kind == "SD" {
                let sql = "SELECT TOP 1 d.IODetailID AS ID \
                           FROM tStk_IODetail d \
                           INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                           WHERE io.Kind='SR' AND d.SouID = @p1 AND io.State NOT IN ('D','C')";
                if let Ok(s) = conn.query(sql, &[&id]).await {
                    if let Ok(Some(r)) = s.into_row().await {
                        if !r.get::<&str, _>("ID").unwrap_or("").is_empty() {
                            return Some("该出库单已有销售退货引用，无法反审核".to_string());
                        }
                    }
                }
            }
            // TH/PR: 检查是否被下游引用（一般无）
            if kind == "TH" || kind == "PR" {
                // 暂无下游单据
            }
        }
        "stock_take" | "stock_check" => {
            // 盘点单无下游单据
        }
        "stock_move" => {
            // 调拨单无下游单据
        }
        "replenish" => {
            // 补货申请：检查是否已生成 PD 草稿
            let sql = "SELECT TOP 1 CAST(IONo AS NVARCHAR(20)) AS N FROM tStk_IO WHERE Kind = 'PD' AND Note LIKE '%' + @p1 + '%' AND State <> 'D'";
            if let Ok(s) = conn.query(sql, &[&id]).await {
                if let Ok(Some(r)) = s.into_row().await {
                    if !r.get::<&str, _>("N").unwrap_or("").is_empty() {
                        return Some("该补货申请已生成采购入库草稿，请先删除对应入库单".to_string());
                    }
                }
            }
        }
        _ => {}
    }
    None
}

/// 反审 SD 时回滚 Reserve：ReleasedQty -= qty, State 按剩余量回退
/// DB 规则：预占表生命周期 = A 有效 / X 已释放
pub async fn unrelease_reserve_by_doc(
    conn: &mut Conn,
    doc_type: &str,
    source_doc_id: &str,
    gdsid: &str,
    stkid: &str,
    qty: f64,
) {
    if source_doc_id.is_empty() || gdsid.is_empty() || stkid.is_empty() || qty <= 0.0 {
        return;
    }
    // 找该 SD 引用的 SO 对应的 Reserve 记录
    let sql = "SELECT TOP 1 ReserveID, ISNULL(ReleasedQty,0) AS RQ, ISNULL(Qty,0) AS Q \
               FROM tStk_Reserve \
               WHERE DocType = @p1 AND DocID = @p2 AND GDSID = @p3 AND StkID = @p4";
    let row_opt = match conn.query(sql, &[&doc_type, &source_doc_id, &gdsid, &stkid]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(_) => None,
    };
    if let Some(r) = row_opt {
        let reserve_id = r.get::<&str, _>("ReserveID").unwrap_or("").to_string();
        let cur_released = row_get_f64(&r, "RQ");
        let total = row_get_f64(&r, "Q");
        if reserve_id.is_empty() { return; }
        let new_released = (cur_released - qty).max(0.0);
        // 反审后剩余释放量必然 < total（因为是在减少），State 恢复为 A
        // 边界保护：万一 new_released == total（不应该发生）才标 X
        let new_state = if new_released >= total - 0.0001 { "X" } else { "A" };
        let upd = "UPDATE tStk_Reserve SET ReleasedQty = @p1, State = @p2 WHERE ReserveID = @p3";
        let p: Vec<&dyn tiberius::ToSql> = vec![&new_released, &new_state, &reserve_id];
        let _ = conn.execute(upd, &p).await;
    }
}

/// 读 tStk_Stock 当前 Qty
pub async fn query_stock_qty(conn: &mut Conn, gdsid: &str, stkid: &str) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    match conn.query(sql, &params).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                let q_str = row.get::<&str, _>("Q").unwrap_or("0");
                q_str.parse().unwrap_or(0.0)
            } else {
                0.0
            }
        }
        Err(_) => 0.0,
    }
}

/// 写 tStk_StockTranHis（覆盖式 UPSERT，存的是最近一次出入库流水）
pub async fn upsert_stock_tran_his(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    tran_id: &str,
    tran_detail_id: &str,
    in_qty: f64,
    out_qty: f64,
    end_qty: f64,
) {
    if gdsid.is_empty() || stkid.is_empty() {
        return;
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockTranHis WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_StockTranHis
                 SET LastTranDate = GETDATE(), Qty = @p7 - @p3 + @p4,
                     TranID = @p5, TranDetailID = @p6,
                     InQty = @p3, OutQty = @p4, EndQty = @p7
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_StockTranHis
                 (GDSID, StkID, LastTranDate, Qty, TranID, TranDetailID, InQty, OutQty, EndQty)
                 VALUES (@p1, @p2, GETDATE(), @p7 - @p3 + @p4, @p5, @p6, @p3, @p4, @p7)"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &gdsid, &stkid,
        &in_qty, &out_qty,
        &tran_id, &tran_detail_id,
        &end_qty,
    ];
    let _ = conn.execute(sql, &params).await.map_err(|e| {
        eprintln!("[approve] upsert_stock_tran_his SQL失败: {} (gdsid={}, stkid={})", e, gdsid, stkid);
        e
    });
}

/// 累加 tStk_StockYM（按当前月份 YYYYMM）
pub async fn upsert_stock_ym(conn: &mut Conn, gdsid: &str, stkid: &str, in_qty: f64, out_qty: f64) {
    if gdsid.is_empty() || stkid.is_empty() || (in_qty == 0.0 && out_qty == 0.0) {
        return;
    }
    // AccYM 格式：YYYYMM (int)
    let ym: i32 = chrono::Local::now().format("%Y%m").to_string().parse().unwrap_or(202501);
    let delta = in_qty - out_qty;
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_StockYM WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3)
                 UPDATE tStk_StockYM
                 SET InQty = ISNULL(InQty,0) + @p4,
                     OutQty = ISNULL(OutQty,0) + @p5,
                     EndQty = ISNULL(EndQty,0) + @p6
                 WHERE AccYM = @p1 AND StkID = @p2 AND GDSID = @p3
                 ELSE
                 INSERT INTO tStk_StockYM (AccYM, StkID, GDSID, InitQty, InQty, OutQty, EndQty)
                 VALUES (@p1, @p2, @p3, 0, @p4, @p5, @p6)"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &ym, &stkid, &gdsid,
        &in_qty, &out_qty, &delta,
    ];
    let _ = conn.execute(sql, &params).await;
}

/// 维护 tStk_Qty 物化快照表（与 tStk_Stock 同步）
/// DB 设计要求：每次过账后同步更新该表用于快速查询
pub async fn upsert_stock_qty_snapshot(conn: &mut Conn, gdsid: &str, stkid: &str) {
    if gdsid.is_empty() || stkid.is_empty() {
        return;
    }
    // 读 tStk_Stock 当前 Qty
    let qty: f64 = query_stock_qty(conn, gdsid, stkid).await;
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Qty WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Qty SET Qty = @p3, LUTime = GETDATE()
                 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Qty (GDSID, StkID, Qty, LUTime)
                 VALUES (@p1, @p2, @p3, GETDATE())"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid, &qty];
    let _ = conn.execute(sql, &params).await;
}

/// 反审时删除 tStk_StockTranHis 中关联此单据的流水记录
/// 因为该表只保留"最近一次交易"，删除后由下一次过账自动重建
pub async fn delete_stock_tran_his(
    conn: &mut Conn,
    tran_id: &str,
) {
    if tran_id.is_empty() {
        return;
    }
    let sql = "DELETE FROM tStk_StockTranHis WHERE TranID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&tran_id];
    let _ = conn.execute(sql, &params).await;
}

/// 月结：把指定月份所有 (StkID, GDSID) 的 EndQty 作为下月 InitQty
/// DB 规则：月初把上月 EndQty 复制为 InitQty
pub async fn month_end_settle(
    conn: &mut Conn,
    from_ym: i32,  // 来源月份 YYYYMM
    to_ym: i32,    // 目标月份 YYYYMM
) -> i32 {
    // 1) 检查目标月份是否已结存（如果 InitQty 已经从历史继承则跳过，避免重复累加）
    let sql = r#"
        INSERT INTO tStk_StockYM (AccYM, StkID, GDSID, InitQty, InQty, OutQty, EndQty)
        SELECT @p1, m.StkID, m.GDSID, m.EndQty, 0, 0, m.EndQty
        FROM tStk_StockYM m
        WHERE m.AccYM = @p2
          AND NOT EXISTS (
              SELECT 1 FROM tStk_StockYM t
              WHERE t.AccYM = @p1 AND t.StkID = m.StkID AND t.GDSID = m.GDSID
          );
    "#;
    let params: Vec<&dyn tiberius::ToSql> = vec![&to_ym, &from_ym];
    let result = conn.execute(sql, &params).await;
    match result {
        Ok(r) => r.rows_affected().iter().sum::<u64>() as i32,
        Err(_) => -1,
    }
}

/// 回填详情表的库存快照 StkQty/AQty（便于后续单据看到当前库存）
pub async fn fill_detail_stock_snapshot(
    conn: &mut Conn,
    detail_table: &str,
    detail_pk: &str,
    detail_id: &str,
) {
    if detail_id.is_empty() {
        return;
    }
    // 1. 取这个 detail 的 GDSID, StkID
    let sql = format!(
        "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID \
         FROM [{}] WHERE {} = @p1",
        detail_table, detail_pk
    );
    let mut gdsid = String::new();
    let mut stkid = String::new();
    if let Ok(stream) = conn.query(&sql, &[&detail_id]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            gdsid = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
            stkid = row.get::<&str, _>("StkID").unwrap_or("").to_string();
        }
    }
    if gdsid.is_empty() || stkid.is_empty() {
        return;
    }
    // 2. 读 Qty/QQty
    let stk_sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Qq FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let mut qty: f64 = 0.0;
    let mut qqty: f64 = 0.0;
    if let Ok(stream) = conn.query(stk_sql, &[&gdsid, &stkid]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let q_str = row.get::<&str, _>("Q").unwrap_or("0");
            let qq_str = row.get::<&str, _>("Qq").unwrap_or("0");
            qty = q_str.parse().unwrap_or(0.0);
            qqty = qq_str.parse().unwrap_or(0.0);
        }
    }
    // 3. 回填
    let upd = format!("UPDATE [{}] SET StkQty = @p1, AQty = @p2 WHERE {} = @p3", detail_table, detail_pk);
    let params: Vec<&dyn tiberius::ToSql> = vec![&qty, &qqty, &detail_id];
    let _ = conn.execute(&upd, &params).await;
}

/// 批量回填 tStk_IODetail 的 StkQty/AQty（按 IOID）
/// 用于 create_io / update_io 时让草稿也能看到当前库存
pub async fn fill_io_detail_stock_snapshot(conn: &mut Conn, ioid: &str) {
    if ioid.is_empty() {
        return;
    }
    // 用单条 SQL 批量 UPDATE（避免 N+1 查询）
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_IODetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.IOID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&ioid];
    let _ = conn.execute(sql, &params).await;
}

/// 批量回填 tStk_MoveDetail 的 StkQty/AQty（按 MoveID）
pub async fn fill_move_detail_stock_snapshot(conn: &mut Conn, move_id: &str) {
    if move_id.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_MoveDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.MoveID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&move_id];
    let _ = conn.execute(sql, &params).await;
}

/// 批量回填 tStk_TranDetail 的 StkQty/AQty（按 TranID）
pub async fn fill_tran_detail_stock_snapshot(conn: &mut Conn, tran_id: &str) {
    if tran_id.is_empty() {
        return;
    }
    let sql = "UPDATE d SET d.StkQty = ISNULL(s.Qty, 0), d.AQty = ISNULL(s.QQty, 0) \
               FROM tStk_TranDetail d \
               LEFT JOIN tStk_Stock s ON s.GDSID = d.GDSID AND s.StkID = d.StkID \
               WHERE d.TranID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&tran_id];
    let _ = conn.execute(sql, &params).await;
}

/// 统一的库存"过账"：更新三件套（tStk_Stock + tStk_StockTranHis + tStk_StockYM）
/// 并同步 tStk_Qty 物化快照
/// 返回 (new_qty, success)
pub async fn post_ledger(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,    // +1 入库 / -1 出库
    tran_id: &str,
    tran_detail_id: &str,
) -> (f64, bool) {
    if gdsid.is_empty() || stkid.is_empty() || qty == 0.0 {
        return (0.0, true);
    }
    let delta = direction * qty;
    // 出库时先检查库存
    if delta < 0.0 {
        let cur = query_stock_qty(conn, gdsid, stkid).await;
        if cur + delta < -0.0001 {
            return (cur, false);
        }
    }
    let new_qty = apply_stock_delta_qq(conn, gdsid, stkid, delta).await;
    if new_qty < -0.5 {
        return (0.0, false);
    }
    let in_qty = if delta > 0.0 { delta } else { 0.0 };
    let out_qty = if delta < 0.0 { -delta } else { 0.0 };
    upsert_stock_tran_his(conn, gdsid, stkid, tran_id, tran_detail_id, in_qty, out_qty, new_qty).await;
    upsert_stock_ym(conn, gdsid, stkid, in_qty, out_qty).await;
    // 维护 tStk_Qty 物化快照
    upsert_stock_qty_snapshot(conn, gdsid, stkid).await;
    (new_qty, true)
}

/// 反审时"反过账"：方向相反
async fn post_ledger_reverse(
    conn: &mut Conn,
    gdsid: &str,
    stkid: &str,
    qty: f64,
    direction: f64,
    tran_id: &str,
    tran_detail_id: &str,
) -> bool {
    post_ledger(conn, gdsid, stkid, qty, -direction, tran_id, tran_detail_id).await.1
}

/// 预占/释放（销售订单逻辑，QQty 减，Qty 不变）
pub async fn apply_qqty_delta(conn: &mut Conn, gdsid: &str, stkid: &str, delta: f64) -> bool {
    if gdsid.is_empty() || stkid.is_empty() {
        return true;
    }
    if delta < 0.0 {
        let cur_qq = query_qqty(conn, gdsid, stkid).await;
        if cur_qq + delta < -0.0001 {
            return false;
        }
    }
    let sql = r#"IF EXISTS (SELECT 1 FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2)
                 UPDATE tStk_Stock SET QQty = ISNULL(QQty,0) + @p3 WHERE GDSID = @p1 AND StkID = @p2
                 ELSE
                 INSERT INTO tStk_Stock (GDSStockID, GDSID, StkID, Qty, QQty) VALUES (NEWID(), @p1, @p2, 0, @p3)"#;
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid, &delta];
    conn.execute(sql, &params).await.is_ok()
}

async fn query_qqty(conn: &mut Conn, gdsid: &str, stkid: &str) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = "SELECT ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Q FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    match conn.query(sql, &params).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                let q_str = row.get::<&str, _>("Q").unwrap_or("0");
                q_str.parse().unwrap_or(0.0)
            } else {
                0.0
            }
        }
        Err(_) => 0.0,
    }
}

async fn query_reserved_qty(conn: &mut Conn, gdsid: &str, stkid: &str) -> f64 {
    if gdsid.is_empty() || stkid.is_empty() {
        return 0.0;
    }
    let sql = "SELECT ISNULL(CAST(SUM(ISNULL(Qty,0) - ISNULL(ReleasedQty,0)) AS NVARCHAR(50)), '0') AS R \
               FROM tStk_Reserve WHERE GDSID = @p1 AND StkID = @p2 AND State = 'A'";
    let params: Vec<&dyn tiberius::ToSql> = vec![&gdsid, &stkid];
    match conn.query(sql, &params).await {
        Ok(stream) => {
            if let Ok(Some(row)) = stream.into_row().await {
                row_get_f64(&row, "R")
            } else {
                0.0
            }
        }
        Err(_) => 0.0,
    }
}

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
    if qty <= 0.0 {
        return true;
    }
    let sql = "INSERT INTO tStk_Reserve (ReserveID, DocType, DocID, DocNo, DetailID, GDSID, StkID, Qty, ReleasedQty, State, EDate, EUser) \
               VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, 0, 'A', GETDATE(), @p9)";
    let params: Vec<&dyn tiberius::ToSql> = vec![
        &reserve_id, &doc_type, &doc_id, &doc_no, &detail_id,
        &gdsid, &stkid, &qty, &user,
    ];
    conn.execute(sql, &params).await.is_ok()
}

async fn release_reserve_by_doc(
    conn: &mut Conn,
    doc_type: &str,
    doc_id: &str,
    gdsid: &str,
    stkid: &str,
    ship_qty: f64,
) {
    let sql = "SELECT TOP 1 ReserveID, ISNULL(Qty,0) - ISNULL(ReleasedQty,0) AS Remain \
               FROM tStk_Reserve WHERE DocType = @p1 AND DocID = @p2 AND GDSID = @p3 AND StkID = @p4 AND State = 'A' \
               ORDER BY EDate ASC";
    let params: Vec<&dyn tiberius::ToSql> = vec![&doc_type, &doc_id, &gdsid, &stkid];

    let (reserve_id, remain) = match conn.query(sql, &params).await {
        Ok(stream) => match stream.into_row().await {
            Ok(Some(row)) => {
                let id = row.get::<&str, _>("ReserveID").unwrap_or("").to_string();
                let r = row_get_f64(&row, "Remain");
                (id, r)
            }
            _ => (String::new(), 0.0),
        },
        _ => (String::new(), 0.0),
    };

    let to_release = ship_qty.min(remain).max(0.0);
    if !reserve_id.is_empty() && to_release > 0.0 {
        let upd = "UPDATE tStk_Reserve SET ReleasedQty = ISNULL(ReleasedQty,0) + @p1, \
                   State = CASE WHEN ISNULL(ReleasedQty,0) + @p1 >= ISNULL(Qty,0) THEN 'X' ELSE 'A' END \
                   WHERE ReserveID = @p2";
        let p2: Vec<&dyn tiberius::ToSql> = vec![&to_release, &reserve_id];
        let _ = conn.execute(upd, &p2).await;
    }
}

async fn void_reserve_by_doc(conn: &mut Conn, doc_type: &str, doc_id: &str) {
    let sql = "UPDATE tStk_Reserve SET State = 'X' WHERE DocType = @p1 AND DocID = @p2 AND State = 'A'";
    let params: Vec<&dyn tiberius::ToSql> = vec![&doc_type, &doc_id];
    let _ = conn.execute(sql, &params).await;
}

async fn query_doc_no(conn: &mut Conn, table: &str, primary_key: &str, id: &str) -> String {
    let sql = format!("SELECT OrderNo FROM [{}] WHERE [{}] = @p1", table, primary_key);
    let params: Vec<&dyn tiberius::ToSql> = vec![&id];
    if let Ok(stream) = conn.query(&sql, &params).await {
        if let Ok(Some(row)) = stream.into_row().await {
            return row.get::<&str, _>("OrderNo").unwrap_or("").to_string();
        }
    }
    String::new()
}

/// 从主表查源 SOID（支持业务单号或主表 PK 两种入参）
/// - 业务单号场景：反审 SD/SR 时 params.id 是 SINo/IONo，tStk_Reserve.DocID 存的是 SOID，
///   必须先经 resolve_id_transform 把业务单号转 SIID/IOID，再用 PK 反查 SOID
/// - 主表 PK 场景：传的就是 SIID/IOID，可直接查
async fn query_source_soid(conn: &mut Conn, table: &str, primary_key: &str, id: &str) -> String {
    if id.is_empty() {
        return String::new();
    }
    // 业务单号 → 主表 PK 转换（当 primary_key 就是主表 PK 时才需要）
    let lookup_id: String = if let Some((mt, bk, mpk)) = resolve_id_transform(table) {
        if mpk == primary_key {
            let sql = format!("SELECT CAST({} AS NVARCHAR(40)) AS PK FROM [{}] WHERE [{}] = @p1", mpk, mt, bk);
            let p: Vec<&dyn tiberius::ToSql> = vec![&id];
            match conn.query(&sql, &p).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("PK").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                Err(e) => {
                    eprintln!("[query_source_soid] 业务单号→PK 转换失败: table={}, biz_id={}, err={}", table, id, e);
                    String::new()
                }
            }
        } else {
            id.to_string()
        }
    } else {
        id.to_string()
    };
    if lookup_id.is_empty() {
        return String::new();
    }
    for col in &["SOID", "SQID", "FromSOID"] {
        let sql = format!("SELECT ISNULL([{}], '') AS S FROM [{}] WHERE [{}] = @p1", col, table, primary_key);
        let params: Vec<&dyn tiberius::ToSql> = vec![&lookup_id];
        if let Ok(stream) = conn.query(&sql, &params).await {
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

fn gen_doc_no(prefix: &str) -> String {
    let now = chrono::Local::now();
    format!("{}{}", prefix, now.format("%Y%m%d%H%M%S%3f"))
}

/// 主审核入口
pub async fn approve_doc(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ApproveParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = format!(
        "UPDATE [{}] SET State = '{}', AUser = @p1, ADate = @p2 WHERE [{}] = @p3 AND (State = '{}' OR State = '{}')",
        params.table, doc_state::STATE_REVIEWED, params.primary_key,
        doc_state::STATE_DRAFT, doc_state::STATE_NEW
    );
    let now = chrono::Local::now().naive_local();
    let approver_uuid: String = format_uuid_or_zero(&claims.user_code);
    let approve_params: Vec<&dyn tiberius::ToSql> = vec![&approver_uuid, &now, &params.id];
    let result = conn.execute(&sql, &approve_params).await?;
    let rows_affected = result.rows_affected().get(0).copied().unwrap_or(0);

    if rows_affected == 0 {
        return Ok(Json(ApiResponse::err("审核失败：单据不存在或状态不是草稿")));
    }

    if let Some(dt) = &params.doc_type {
        match dt.as_str() {
            "sales_order" => {
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                if rows.is_empty() {
                    return Ok(Json(ApiResponse::err("销售订单无明细行")));
                }
                let mut shortage: Vec<String> = Vec::new();
                for (gdsid, stkid, qty, _did, _mid) in &rows {
                    let stock = query_stock_qty(&mut conn, gdsid, stkid).await;
                    let reserved = query_reserved_qty(&mut conn, gdsid, stkid).await;
                    let available = stock - reserved;
                    if available < *qty {
                        shortage.push(format!("商品{} 仓库{} 库存{} 预占{} 可用{} 需求{}", gdsid, stkid, stock, reserved, available, qty));
                    }
                }
                if !shortage.is_empty() {
                    return Ok(Json(ApiResponse::err(&format!("可用量不足: {}", shortage.join("; ")))));
                }
                // 预占：QQty 减（Qty 不变）
                for (gdsid, stkid, qty, _did, _mid) in &rows {
                    if !apply_qqty_delta(&mut conn, gdsid, stkid, -*qty).await {
                        return Ok(Json(ApiResponse::err("预占失败：可用量不足")));
                    }
                }
                let doc_no = query_doc_no(&mut conn, &params.table, &params.primary_key, &params.id).await;
                for (gdsid, stkid, qty, did, _mid) in &rows {
                    let rid = gen_doc_no("R");
                    if !insert_reserve(&mut conn, &rid, "sales_order", &params.id, &doc_no, did, gdsid, stkid, *qty, &format_uuid_or_zero(&claims.user_code)).await {
                        return Ok(Json(ApiResponse::err("写入预占失败")));
                    }
                }
            }

            "purchase_inbound" | "purchase_receipt" => {
                let (detail_table, detail_pk) = resolve_detail_meta(&params.table)
                    .map(|(t, p, _)| (t, p))
                    .unwrap_or(("tPur_OrderDetail", "PODetailID"));
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    let (new_qty, ok) = post_ledger(&mut conn, gdsid, stkid, *qty, 1.0, mid, did).await;
                    if !ok {
                        return Ok(Json(ApiResponse::err("更新库存失败")));
                    }
                    fill_detail_stock_snapshot(&mut conn, detail_table, detail_pk, did).await;
                    let _ = new_qty;
                }
            }

            "purchase_return" | "store_return" => {
                let (detail_table, detail_pk) = resolve_detail_meta(&params.table)
                    .map(|(t, p, _)| (t, p))
                    .unwrap_or(("tPur_OrderDetail", "PODetailID"));
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    let (new_qty, ok) = post_ledger(&mut conn, gdsid, stkid, *qty, -1.0, mid, did).await;
                    if !ok { return Ok(Json(ApiResponse::err("库存不足"))); }
                    fill_detail_stock_snapshot(&mut conn, detail_table, detail_pk, did).await;
                    let _ = new_qty;
                }
            }

            "sales_outbound" | "sales_inv" => {
                let (detail_table, detail_pk) = resolve_detail_meta(&params.table)
                    .map(|(t, p, _)| (t, p))
                    .unwrap_or(("tSal_InvDetail", "SIDetailID"));
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                if rows.is_empty() {
                    return Ok(Json(ApiResponse::err("出库单无明细行")));
                }
                let source_soid = query_source_soid(&mut conn, &params.table, &params.primary_key, &params.id).await;
                // tStk_Reserve.DocID VARCHAR(30) 实际存的是源单单号（SoNo）而非主键（SOID），
                // 所以要再用 SOID 反查 SoNo，传给 release_reserve_by_doc
                let source_doc_no = if !source_soid.is_empty() {
                    let biz_key_opt = resolve_id_transform(&params.table).map(|(_, bk, _)| bk);
                    if let Some(biz_key) = biz_key_opt {
                        let sql = format!("SELECT ISNULL([{}], '') AS N FROM [{}] WHERE [{}] = @p1", biz_key, params.table, params.primary_key);
                        let p: Vec<&dyn tiberius::ToSql> = vec![&source_soid];
                        match conn.query(&sql, &p).await {
                            Ok(s) => match s.into_row().await {
                                Ok(Some(r)) => r.get::<&str, _>("N").unwrap_or("").to_string(),
                                _ => String::new(),
                            },
                            Err(e) => {
                                eprintln!("[approve.sales_outbound] SOID→SoNo 反查失败: table={}, soid={}, err={}", params.table, source_soid, e);
                                String::new()
                            }
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                let release_key = if !source_doc_no.is_empty() { source_doc_no } else { source_soid };
                for (gdsid, stkid, qty, did, mid) in &rows {
                    if !release_key.is_empty() {
                        // 1) 释放预占（仅更新 tStk_Reserve，不动 tStk_Stock）
                        release_reserve_by_doc(&mut conn, "sales_order", &release_key, gdsid, stkid, *qty).await;
                    } else {
                        eprintln!("[approve.sales_outbound] 源单据缺失（既无 SOID 也无 SoNo），跳过预占释放: table={}, id={}", params.table, params.id);
                    }
                    // 2) 先恢复 QQty（释放预占后 +qty），让 post_ledger 再扣减
                    let _ = apply_qqty_delta(&mut conn, gdsid, stkid, *qty).await;
                    // 3) 出库：Qty 和 QQty 同步减
                    let (new_qty, ok) = post_ledger(&mut conn, gdsid, stkid, *qty, -1.0, mid, did).await;
                    if !ok {
                        return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, qty))));
                    }
                    fill_detail_stock_snapshot(&mut conn, detail_table, detail_pk, did).await;
                }
            }

            "sales_return" => {
                // SR 实际是 tStk_IO Kind='SR'，明细是 tStk_IODetail
                let (detail_table, detail_pk) = ("tStk_IODetail", "IODetailID");
                let rows = fetch_doc_detail_rows(&mut conn, "tStk_IO", &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    let (_, ok) = post_ledger(&mut conn, gdsid, stkid, *qty, 1.0, mid, did).await;
                    if !ok { return Ok(Json(ApiResponse::err("更新库存失败"))); }
                    fill_detail_stock_snapshot(&mut conn, detail_table, detail_pk, did).await;
                }
            }

            "stock_io" => {
                // 通用入出库单：按 Kind 决定库存方向
                let kind = get_io_kind(&mut conn, &params.id).await;
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                if rows.is_empty() {
                    return Ok(Json(ApiResponse::err("入出库单无明细行")));
                }
                match kind.as_str() {
                    "OT" | "ZP" => {
                        // OT/ZP: 按每行 Qty 符号决定方向（正=入库，负=出库）
                        for (gdsid, stkid, qty, did, mid) in &rows {
                            let dir = if *qty >= 0.0 { 1.0 } else { -1.0 };
                            let abs_qty = qty.abs();
                            let (new_qty, ok) = post_ledger(&mut conn, gdsid, stkid, abs_qty, dir, mid, did).await;
                            if !ok { return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, abs_qty)))); }
                            fill_detail_stock_snapshot(&mut conn, "tStk_IODetail", "IODetailID", did).await;
                        }
                    }
                    _ => {
                        let direction: f64 = match kind.as_str() {
                            "RI" | "PD" | "SR" => 1.0,
                            "SD" | "POS" | "SI" | "TH" | "PR" => -1.0,
                            _ => 0.0,
                        };
                        if direction == 0.0 {
                            return Ok(Json(ApiResponse::err(&format!("该单据类型({}) 需走专门审核流程", kind))));
                        }
                        for (gdsid, stkid, qty, did, mid) in &rows {
                            let (new_qty, ok) = post_ledger(&mut conn, gdsid, stkid, *qty, direction, mid, did).await;
                            if !ok { return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, qty)))); }
                            fill_detail_stock_snapshot(&mut conn, "tStk_IODetail", "IODetailID", did).await;
                        }
                    }
                }
            }

            "stock_move" => {
                // 调拨：扣减 FromStkID, 增加 ToStkID —— 必须在同一事务内完成
                let (from_id, to_id) = get_move_stk(&mut conn, &params.id).await;
                if from_id.is_empty() || to_id.is_empty() {
                    return Ok(Json(ApiResponse::err("调拨单仓库信息不完整")));
                }
                let detail_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, CAST(StkID AS NVARCHAR(40)) AS StkID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, CAST(MoveDetailID AS NVARCHAR(40)) AS DID \
                                  FROM tStk_MoveDetail WHERE MoveID = @p1";
                let detail_rows = match conn.query(detail_sql, &[&params.id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                if detail_rows.is_empty() {
                    return Ok(Json(ApiResponse::err("调拨单无明细行")));
                }
                // 调拨双向过账必须在显式事务内：FromStkID -qty 与 ToStkID +qty 同步提交
                // tiberius 0.11 无 Rust API 事务，使用 SQL 显式 BEGIN TRAN / COMMIT / ROLLBACK
                let empty_p: Vec<&dyn tiberius::ToSql> = vec![];
                if let Err(e) = conn.execute("BEGIN TRAN", &empty_p).await {
                    return Ok(Json(ApiResponse::err(&format!("开启调拨事务失败: {}", e))));
                }
                let mut tx_failed: Option<String> = None;
                for r in &detail_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let qty: f64 = r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || qty == 0.0 { continue; }
                    // 调出仓 -qty
                    let (cur, ok1) = post_ledger(&mut conn, &gdsid, &from_id, qty, -1.0, &params.id, &did).await;
                    if !ok1 {
                        tx_failed = Some(format!("调出仓库存不足: 现有{} 需求{}", cur, qty));
                        break;
                    }
                    // 调入仓 +qty
                    let (_, ok2) = post_ledger(&mut conn, &gdsid, &to_id, qty, 1.0, &params.id, &did).await;
                    if !ok2 {
                        tx_failed = Some("调入仓写入失败".to_string());
                        break;
                    }
                    fill_detail_stock_snapshot(&mut conn, "tStk_MoveDetail", "MoveDetailID", &did).await;
                }
                if let Some(err) = tx_failed {
                    let _ = conn.execute("ROLLBACK TRAN", &empty_p).await;
                    return Ok(Json(ApiResponse::err(&err)));
                }
                if let Err(e) = conn.execute("COMMIT TRAN", &empty_p).await {
                    let _ = conn.execute("ROLLBACK TRAN", &empty_p).await;
                    return Ok(Json(ApiResponse::err(&format!("提交调拨事务失败: {}", e))));
                }
            }

            "stocktake" | "stock_take" | "stock_check" => {
                // 盘点单：改用 tStk_Tran + tStk_TranDetail（按 DiffQty 调库存）
                // id 是业务单号（TranNo），先转 TranID
                let tran_id = match resolve_tran_id(&mut conn, &params.id).await {
                    Some(s) if !s.is_empty() => s,
                    _ => return Ok(Json(ApiResponse::err("盘点单 TranNo→TranID 转换失败"))),
                };
                let stk_id = get_tran_stk_by_id(&mut conn, &tran_id).await;
                if stk_id.is_empty() {
                    return Ok(Json(ApiResponse::err("盘点单仓库信息缺失")));
                }
                let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                               ISNULL(CAST(AccQty AS NVARCHAR(50)),'0') AS AQ, \
                               ISNULL(CAST(RealQty AS NVARCHAR(50)),'0') AS RQ, \
                               ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                               CAST(TranDetailID AS NVARCHAR(40)) AS DID \
                               FROM tStk_TranDetail WHERE TranID = @p1";
                let det_rows = match conn.query(det_sql, &[&tran_id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                for r in &det_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let dq: f64 = r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || dq == 0.0 { continue; }
                    // 差异 = 实存 - 账存。正数 = 入库 +dq，负数 = 出库 +|dq|
                    let (new_qty, ok) = post_ledger(&mut conn, &gdsid, &stk_id, dq.abs(), if dq > 0.0 { 1.0 } else { -1.0 }, &tran_id, &did).await;
                    if !ok { return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, dq.abs())))); }
                    fill_detail_stock_snapshot(&mut conn, "tStk_TranDetail", "TranDetailID", &did).await;
                }
            }

            "stock_cycle" => {
                // 周期盘点：tStk_StockCycle + tStk_StockCycleDetail（按 DiffQty 调库存）
                let stk_id = get_stock_cycle_stk(&mut conn, &params.id).await;
                if stk_id.is_empty() {
                    return Ok(Json(ApiResponse::err("周期盘点单仓库信息缺失")));
                }
                let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                               ISNULL(CAST(AccQty AS NVARCHAR(50)),'0') AS AQ, \
                               ISNULL(CAST(RealQty AS NVARCHAR(50)),'0') AS RQ, \
                               ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                               CAST(CycleDetailID AS NVARCHAR(40)) AS DID \
                               FROM tStk_StockCycleDetail WHERE CycleID = @p1";
                let det_rows = match conn.query(det_sql, &[&params.id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                for r in &det_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let dq: f64 = r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || dq == 0.0 { continue; }
                    let (new_qty, ok) = post_ledger(&mut conn, &gdsid, &stk_id, dq.abs(), if dq > 0.0 { 1.0 } else { -1.0 }, &params.id, &did).await;
                    if !ok { return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, dq.abs())))); }
                    fill_stock_cycle_snapshot(&mut conn, &gdsid, &stk_id, &did).await;
                }
            }

            "replenish" => {
                // 补货申请审核通过 → 自动生成 PD（采购入库）草稿
                // DB 业务规则：补货申请是"提示性单据"，转 PD 后走标准审核
                let apply_id = params.id.clone();
                // 1) 读 ReplenishApply 主表
                let head_sql = "SELECT CAST(ReplenishApplyID AS NVARCHAR(40)) AS AID, \
                                CAST(StkID AS NVARCHAR(40)) AS SID, \
                                ISNULL(CAST(EmpID AS NVARCHAR(36)),'') AS EMP \
                                FROM tStk_ReplenishApply WHERE ReplenishApplyNo = @p1";
                let head_opt = match conn.query(head_sql, &[&apply_id]).await {
                    Ok(s) => s.into_row().await.ok().flatten(),
                    Err(_) => None,
                };
                if let Some(h) = head_opt {
                    let stk_id = h.get::<&str, _>("SID").unwrap_or("").to_string();
                    let emp_id = h.get::<&str, _>("EMP").unwrap_or("").to_string();
                    if stk_id.is_empty() {
                        return Ok(Json(ApiResponse::err("补货申请仓库信息缺失")));
                    }
                    // 2) 读明细
                    let det_sql = "SELECT CAST(ApplyDetailID AS NVARCHAR(40)) AS ADID, \
                                   CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                                   ISNULL(CAST(UnitNO AS NVARCHAR(20)),'') AS UNIT, \
                                   ISNULL(CAST(ApplyQty AS NVARCHAR(50)),'0') AS Q \
                                   FROM tStk_ReplenishApplyDtl WHERE ReplenishApplyID = @p1";
                    let detail_rows = match conn.query(det_sql, &[&apply_id]).await {
                        Ok(s) => s.into_first_result().await.unwrap_or_default(),
                        Err(_) => Vec::new(),
                    };
                    if detail_rows.is_empty() {
                        return Ok(Json(ApiResponse::err("补货申请无明细行，无法生成入库单")));
                    }
                    // 3) 生成 PD 草稿
                    let io_no = format!("PD{}", chrono::Local::now().format("%Y%m%d%H%M%S%3f"));
                    let dt: chrono::NaiveDateTime = chrono::Local::now().naive_local();
                    let draft_state: &str = doc_state::STATE_DRAFT;
                    let io_sql = "INSERT INTO tStk_IO (IONo, IoDate, Kind, StkID, EmpID, Note, SumQty, ScanMode, State, EDate, EUser) \
                                  VALUES (@p1, @p2, 'PD', @p3, @p4, @p5, 0, 'N', @p6, @p7, @p8)";
                    let io_p_note = format!("自动从补货申请 {} 生成", apply_id);
                    let io_p: Vec<&dyn tiberius::ToSql> = vec![
                        &io_no, &dt, &stk_id, &emp_id,
                        &io_p_note,
                        &draft_state, &dt, &ZERO_UUID,
                    ];
                    if let Err(e) = conn.execute(io_sql, &io_p).await {
                        return Ok(Json(ApiResponse::err(&format!("生成 PD 草稿失败: {}", e))));
                    }
                    // 抓取 IOID
                    let new_ioid: String = {
                        let q = "SELECT CAST(IOID AS NVARCHAR(40)) AS ID FROM tStk_IO WHERE IONo = @p1";
                        match conn.query(q, &[&io_no]).await {
                            Ok(s) => match s.into_row().await {
                                Ok(Some(r)) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
                                _ => String::new(),
                            },
                            _ => String::new(),
                        }
                    };
                    // 4) 写 PD 明细
                    let mut total_qty: f64 = 0.0;
                    for (i, dr) in detail_rows.iter().enumerate() {
                        let row_no = (i + 1) as i32;
                        let gdsid = dr.get::<&str, _>("GDSID").unwrap_or("").to_string();
                        let unit = dr.get::<&str, _>("UNIT").unwrap_or("").to_string();
                        let qty_str = dr.get::<&str, _>("Q").unwrap_or("0");
                        let qty: f64 = qty_str.parse().unwrap_or(0.0);
                        total_qty += qty;
                        if gdsid.is_empty() { continue; }
                        let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, UnitNO, Qty, CNVQty, StdQty, AccCheckFlg, Price, Amt) \
                                  VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p6, @p6, 0, 0, 0)";
                        let dp: Vec<&dyn tiberius::ToSql> = vec![
                            &new_ioid, &row_no, &gdsid, &stk_id, &unit, &qty,
                        ];
                        let _ = conn.execute(ds, &dp).await;
                    }
                    // 5) 更新 PD 主表 SumQty
                    let upd_sum = "UPDATE tStk_IO SET SumQty = @p1 WHERE IONo = @p2";
                    let _ = conn.execute(upd_sum, &[&total_qty, &io_no]).await;
                    // 6) 回填 PD 明细的 StkQty/AQty
                    fill_io_detail_stock_snapshot(&mut conn, &new_ioid).await;
                } else {
                    return Ok(Json(ApiResponse::err("补货申请主表读取失败")));
                }
            }

            _ => {}
        }
    }

    // 记录单据审核操作日志（tSys_OperHis）
    let doc_type_label = params.doc_type.as_deref().unwrap_or("doc");
    let remark = format!("审核通过 {}", doc_type_label);
    write_oper_log(&mut conn, "APPROVE", &params.table, &params.id, &claims.user_code, Some(&remark)).await;

    Ok(Json(ApiResponse::msg("审核成功")))
}

/// 如果 user_code 是合法 UUID 则返回它，否则返回零值 UUID
fn format_uuid_or_zero(s: &str) -> String {
    if s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4 {
        s.to_string()
    } else {
        "00000000-0000-0000-0000-000000000000".to_string()
    }
}

async fn get_io_kind(conn: &mut Conn, id: &str) -> String {
    // id 是业务单号（IONo）
    let sql = "SELECT ISNULL(Kind,'RI') AS T FROM tStk_IO WHERE IONo = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&id];
    match conn.query(sql, &params).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(row)) => row.get::<&str, _>("T").unwrap_or("RI").to_string(),
            _ => "RI".to_string(),
        },
        _ => "RI".to_string(),
    }
}

async fn get_move_stk(conn: &mut Conn, id: &str) -> (String, String) {
    let sql = "SELECT ISNULL(CAST(FromStkID AS NVARCHAR(40)),'') AS F, ISNULL(CAST(ToStkID AS NVARCHAR(40)),'') AS T \
               FROM tStk_Move WHERE MoveID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&id];
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

async fn get_tran_stk(conn: &mut Conn, id: &str) -> String {
    let sql = "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_Tran WHERE TranID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&id];
    match conn.query(sql, &params).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(row)) => row.get::<&str, _>("S").unwrap_or("").to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

async fn get_stock_cycle_stk(conn: &mut Conn, id: &str) -> String {
    let sql = "SELECT ISNULL(CAST(StkID AS NVARCHAR(40)),'') AS S FROM tStk_StockCycle WHERE CycleID = @p1";
    let params: Vec<&dyn tiberius::ToSql> = vec![&id];
    match conn.query(sql, &params).await {
        Ok(s) => match s.into_row().await {
            Ok(Some(row)) => row.get::<&str, _>("S").unwrap_or("").to_string(),
            _ => String::new(),
        },
        _ => String::new(),
    }
}

async fn fill_stock_cycle_snapshot(conn: &mut Conn, gdsid: &str, stkid: &str, detail_id: &str) {
    if gdsid.is_empty() || stkid.is_empty() || detail_id.is_empty() {
        return;
    }
    let stk_sql = "SELECT ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, ISNULL(CAST(QQty AS NVARCHAR(50)),'0') AS Qq FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
    let mut qty: f64 = 0.0;
    let mut qqty: f64 = 0.0;
    if let Ok(stream) = conn.query(stk_sql, &[&gdsid, &stkid]).await {
        if let Ok(Some(row)) = stream.into_row().await {
            let q_str = row.get::<&str, _>("Q").unwrap_or("0");
            let qq_str = row.get::<&str, _>("Qq").unwrap_or("0");
            qty = q_str.parse().unwrap_or(0.0);
            qqty = qq_str.parse().unwrap_or(0.0);
        }
    }
    let upd = "UPDATE [tStk_StockCycleDetail] SET StkQty = @p1, AQty = @p2 WHERE CycleDetailID = @p3";
    let params: Vec<&dyn tiberius::ToSql> = vec![&qty, &qqty, &detail_id];
    let _ = conn.execute(upd, &params).await;
}

pub async fn unapprove_doc(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ApproveParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // ===== P0-1 前置检查（DB 规则：先校验，再改状态） =====
    if let Some(dt) = &params.doc_type {
        // 1) 会计期间检查
        if let Some(action_date) = get_doc_action_date(&mut conn, &params.table, &params.primary_key, &params.id).await {
            if let Some(err) = check_period_closed(&mut conn, action_date).await {
                return Ok(Json(ApiResponse::err(&err)));
            }
        }
        // 2) 下游引用检查
        if let Some(err) = check_downstream_exists(&mut conn, dt, &params.id).await {
            return Ok(Json(ApiResponse::err(&err)));
        }
    }

    let sql = format!(
        "UPDATE [{}] SET State = '{}', AUser = NULL, ADate = NULL WHERE [{}] = @p1 AND State = '{}'",
        params.table, doc_state::STATE_DRAFT, params.primary_key, doc_state::STATE_REVIEWED
    );
    let unapp_params: Vec<&dyn tiberius::ToSql> = vec![&params.id];
    let result = conn.execute(&sql, &unapp_params).await?;
    let rows_affected = result.rows_affected().get(0).copied().unwrap_or(0);

    if rows_affected == 0 {
        return Ok(Json(ApiResponse::err("反审核失败：单据不存在或状态不是已审核")));
    }

    if let Some(dt) = &params.doc_type {
        match dt.as_str() {
            "sales_order" => {
                // 反审订单：恢复 QQty（释放预占）
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, _did, _mid) in &rows {
                    let _ = apply_qqty_delta(&mut conn, gdsid, stkid, *qty).await;
                }
                void_reserve_by_doc(&mut conn, "sales_order", &params.id).await;
            }

            "purchase_inbound" => {
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    if !post_ledger_reverse(&mut conn, gdsid, stkid, *qty, 1.0, mid, did).await {
                        return Ok(Json(ApiResponse::err("回滚库存失败")));
                    }
                }
                // 删除关联的流水记录
                delete_stock_tran_his(&mut conn, &params.id).await;
            }

            "purchase_return" => {
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    if !post_ledger_reverse(&mut conn, gdsid, stkid, *qty, -1.0, mid, did).await {
                        return Ok(Json(ApiResponse::err("回滚库存失败")));
                    }
                }
                delete_stock_tran_his(&mut conn, &params.id).await;
            }

            "sales_outbound" => {
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                // SD 反审需要拿到源 SOID 才能复原 Reserve
                let source_soid = query_source_soid(&mut conn, &params.table, &params.primary_key, &params.id).await;
                // tStk_Reserve.DocID VARCHAR(30) 实际存的是源单单号（SoNo）而非主键（SOID），
                // 所以要再用 SOID 反查 SoNo，传给 unrelease_reserve_by_doc
                let source_doc_no = if !source_soid.is_empty() {
                    let biz_key_opt = resolve_id_transform(&params.table).map(|(_, bk, _)| bk);
                    if let Some(biz_key) = biz_key_opt {
                        let sql = format!("SELECT ISNULL([{}], '') AS N FROM [{}] WHERE [{}] = @p1", biz_key, params.table, params.primary_key);
                        let p: Vec<&dyn tiberius::ToSql> = vec![&source_soid];
                        match conn.query(&sql, &p).await {
                            Ok(s) => match s.into_row().await {
                                Ok(Some(r)) => r.get::<&str, _>("N").unwrap_or("").to_string(),
                                _ => String::new(),
                            },
                            Err(e) => {
                                eprintln!("[unapprove.sales_outbound] SOID→SoNo 反查失败: table={}, soid={}, err={}", params.table, source_soid, e);
                                String::new()
                            }
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                // 优先用业务单号（兼容表 DocID VARCHAR(30) 设计），fallback 用 SOID
                let release_key = if !source_doc_no.is_empty() { source_doc_no } else { source_soid };
                for (gdsid, stkid, qty, did, mid) in &rows {
                    if !post_ledger_reverse(&mut conn, gdsid, stkid, *qty, -1.0, mid, did).await {
                        return Ok(Json(ApiResponse::err("回滚库存失败")));
                    }
                    // 重新预占：QQty -qty（恢复出库前被释放的预占）
                    let _ = apply_qqty_delta(&mut conn, gdsid, stkid, -*qty).await;
                    // 复原预占表：ReleasedQty -= qty, State='A' 恢复有效
                    if !release_key.is_empty() {
                        unrelease_reserve_by_doc(&mut conn, "sales_order", &release_key, gdsid, stkid, *qty).await;
                    } else {
                        eprintln!("[unapprove.sales_outbound] 源单据缺失（既无 SOID 也无 SoNo），跳过预占复原: table={}, id={}", params.table, params.id);
                    }
                }
                delete_stock_tran_his(&mut conn, &params.id).await;
            }

            "sales_return" => {
                let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                for (gdsid, stkid, qty, did, mid) in &rows {
                    if !post_ledger_reverse(&mut conn, gdsid, stkid, *qty, 1.0, mid, did).await {
                        return Ok(Json(ApiResponse::err("回滚库存失败")));
                    }
                }
                delete_stock_tran_his(&mut conn, &params.id).await;
            }

            "stock_io" => {
                let kind = get_io_kind(&mut conn, &params.id).await;
                let direction: f64 = match kind.as_str() {
                    "RI" | "PD" | "SR" => 1.0,   // 反审 = 反向
                    "SD" | "POS" | "TH" | "PR" => -1.0,
                    _ => 0.0,
                };
                if direction == 0.0 {
                    eprintln!("[unapprove.stock_io] 跳过未支持 Kind: {}", kind);
                } else {
                    let rows = fetch_doc_detail_rows(&mut conn, &params.table, &params.id).await;
                    for (gdsid, stkid, qty, did, mid) in &rows {
                        if !post_ledger_reverse(&mut conn, gdsid, stkid, *qty, direction, mid, did).await {
                            return Ok(Json(ApiResponse::err("回滚库存失败")));
                        }
                    }
                    delete_stock_tran_his(&mut conn, &params.id).await;
                }
            }

            "stock_move" => {
                let (from_id, to_id) = get_move_stk(&mut conn, &params.id).await;
                if from_id.is_empty() || to_id.is_empty() {
                    return Ok(Json(ApiResponse::err("调拨单仓库信息不完整")));
                }
                let detail_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                                  ISNULL(CAST(Qty AS NVARCHAR(50)),'0') AS Q, CAST(MoveDetailID AS NVARCHAR(40)) AS DID \
                                  FROM tStk_MoveDetail WHERE MoveID = @p1";
                let detail_rows = match conn.query(detail_sql, &[&params.id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                // 调拨反审：调入仓 -qty, 调出仓 +qty，必须在同一事务内完成
                let empty_p: Vec<&dyn tiberius::ToSql> = vec![];
                if let Err(e) = conn.execute("BEGIN TRAN", &empty_p).await {
                    return Ok(Json(ApiResponse::err(&format!("开启调拨反审事务失败: {}", e))));
                }
                let mut tx_failed: Option<String> = None;
                for r in &detail_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let qty: f64 = r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || qty == 0.0 { continue; }
                    // 反审：调入仓减
                    if !post_ledger_reverse(&mut conn, &gdsid, &to_id, qty, 1.0, &params.id, &did).await {
                        tx_failed = Some("调入仓回滚失败".to_string());
                        break;
                    }
                    // 反审：调出仓加
                    if !post_ledger_reverse(&mut conn, &gdsid, &from_id, qty, -1.0, &params.id, &did).await {
                        tx_failed = Some("调出仓恢复失败".to_string());
                        break;
                    }
                }
                if let Some(err) = tx_failed {
                    let _ = conn.execute("ROLLBACK TRAN", &empty_p).await;
                    return Ok(Json(ApiResponse::err(&err)));
                }
                if let Err(e) = conn.execute("COMMIT TRAN", &empty_p).await {
                    let _ = conn.execute("ROLLBACK TRAN", &empty_p).await;
                    return Ok(Json(ApiResponse::err(&format!("提交调拨反审事务失败: {}", e))));
                }
            }

            "stocktake" | "stock_take" | "stock_check" => {
                // 盘点反审核：按 DiffQty 反向回滚库存
                let tran_id = match resolve_tran_id(&mut conn, &params.id).await {
                    Some(s) if !s.is_empty() => s,
                    _ => return Ok(Json(ApiResponse::err("盘点单 TranNo→TranID 转换失败"))),
                };
                let stk_id = get_tran_stk_by_id(&mut conn, &tran_id).await;
                if stk_id.is_empty() {
                    return Ok(Json(ApiResponse::err("盘点单仓库信息缺失")));
                }
                let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                               ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                               CAST(TranDetailID AS NVARCHAR(40)) AS DID \
                               FROM tStk_TranDetail WHERE TranID = @p1";
                let det_rows = match conn.query(det_sql, &[&tran_id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                for r in &det_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let dq: f64 = r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || dq == 0.0 { continue; }
                    // 审核时：DiffQty>0 → +1(入库)，DiffQty<0 → -1(出库)
                    // 反审核：反向，即 DiffQty>0 → 反方向=-1，DiffQty<0 → 反方向=+1
                    let original_direction = if dq > 0.0 { 1.0 } else { -1.0 };
                    if !post_ledger_reverse(&mut conn, &gdsid, &stk_id, dq.abs(), original_direction, &tran_id, &did).await {
                        return Ok(Json(ApiResponse::err("盘点反审核回滚库存失败")));
                    }
                }
                delete_stock_tran_his(&mut conn, &tran_id).await;
            }

            "stock_cycle" => {
                // 周期盘点反审：按 DiffQty 反向回滚库存（tStk_StockCycle + tStk_StockCycleDetail）
                let stk_id = get_stock_cycle_stk(&mut conn, &params.id).await;
                if stk_id.is_empty() {
                    return Ok(Json(ApiResponse::err("周期盘点单仓库信息缺失")));
                }
                let det_sql = "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, \
                               ISNULL(CAST(DiffQty AS NVARCHAR(50)),'0') AS DQ, \
                               CAST(CycleDetailID AS NVARCHAR(40)) AS DID \
                               FROM tStk_StockCycleDetail WHERE CycleID = @p1";
                let det_rows = match conn.query(det_sql, &[&params.id]).await {
                    Ok(s) => match s.into_first_result().await { Ok(rs) => rs, Err(_) => Vec::new() },
                    _ => Vec::new(),
                };
                for r in &det_rows {
                    let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let dq: f64 = r.get::<&str, _>("DQ").unwrap_or("0").parse().unwrap_or(0.0);
                    let did = r.get::<&str, _>("DID").unwrap_or("").to_string();
                    if gdsid.is_empty() || dq == 0.0 { continue; }
                    // 反审：审核时 DiffQty>0 入库，DiffQty<0 出库 → 反方向
                    let original_direction = if dq > 0.0 { 1.0 } else { -1.0 };
                    if !post_ledger_reverse(&mut conn, &gdsid, &stk_id, dq.abs(), original_direction, &params.id, &did).await {
                        return Ok(Json(ApiResponse::err("周期盘点反审核回滚库存失败")));
                    }
                }
                delete_stock_tran_his(&mut conn, &params.id).await;
            }

            "replenish" => {
                // 补货申请反审：删除已自动生成的 PD 草稿（按 Note 关联 + Kind='PD' + State<>'D'）
                let find_sql = "SELECT TOP 1 CAST(IOID AS NVARCHAR(40)) AS I, CAST(IONo AS NVARCHAR(20)) AS N \
                                FROM tStk_IO WHERE Kind = 'PD' AND Note LIKE '%' + @p1 + '%' AND State <> 'D'";
                let target_ioid: String = String::new();
                let target_iono: String = String::new();
                let (ioid, iono) = match conn.query(find_sql, &[&params.id]).await {
                    Ok(s) => match s.into_row().await {
                        Ok(Some(r)) => (
                            r.get::<&str, _>("I").unwrap_or("").to_string(),
                            r.get::<&str, _>("N").unwrap_or("").to_string(),
                        ),
                        _ => (String::new(), String::new()),
                    },
                    _ => (String::new(), String::new()),
                };
                let _ = (target_ioid, target_iono); // 抑制未用警告
                if !ioid.is_empty() {
                    // 1) 删除 PD 详情
                    let del_det = "DELETE FROM tStk_IODetail WHERE IOID = @p1";
                    let _ = conn.execute(del_det, &[&ioid]).await;
                    // 2) 删除 PD 主表
                    let del_main = "DELETE FROM tStk_IO WHERE IOID = @p1";
                    let _ = conn.execute(del_main, &[&ioid]).await;
                    eprintln!("[unapprove.replenish] 已删除自动生成的 PD 草稿: {}", iono);
                }
            }

            _ => {
                eprintln!("[unapprove_doc] 收到未识别的 doc_type: {} (单据不需过账)", dt);
            }
        }
    }

    // 记录单据反审核操作日志
    let doc_type_label = params.doc_type.as_deref().unwrap_or("doc");
    let remark = format!("反审核 {}", doc_type_label);
    write_oper_log(&mut conn, "UNAPPROVE", &params.table, &params.id, &claims.user_code, Some(&remark)).await;

    Ok(Json(ApiResponse::msg("反审核成功")))
}

pub async fn print_log(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<PrintLogParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let copies = params.copies.unwrap_or(1);
    let remark = format!("打印{}份", copies);
    write_oper_log(&mut conn, "PRINT", &params.table, &params.id, &claims.user_code, Some(&remark)).await;

    Ok(Json(ApiResponse::msg("打印记录已保存")))
}
