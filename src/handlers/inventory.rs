use super::base_data::row_to_json;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::{AppError, Result};
use crate::handlers::approval::Conn as StockConn;
use crate::middleware::auth::Claims;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use axum::{Extension, Json, extract::State};
use serde::Deserialize;
use tiberius::Row;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn empty_or_zero(s: &str) -> &str {
    if s.is_empty() { ZERO_UUID } else { s }
}

fn now() -> chrono::NaiveDateTime {
    chrono::Local::now().naive_local()
}

// ============== 入出库单 ==============
pub async fn get_io_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_IO WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1 OR Note LIKE @p2)");
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}

// ============== POID/SOID 上游单据校验 ==============

/// 校验采购订单（POID）的存在性、状态和累计入库数量
/// DB 规则：入库数量不能超过 PO 数量；PO 已作废禁止入库
pub async fn validate_upstream_po(
    conn: &mut StockConn,
    poid: &str,
    current_qty: f64,
) -> Result<()> {
    if poid.is_empty() || poid == "00000000-0000-0000-0000-000000000000" {
        return Ok(());
    }
    // 1) 存在性 + 状态
    let sql = "SELECT CAST(POID AS NVARCHAR(40)) AS ID, ISNULL(SumQty, 0) AS Q, State \
               FROM tPur_Order WHERE POID = @p1";
    let row_opt = match conn.query(sql, &[&poid]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(e) => return Err(AppError::Internal(format!("查 PO 失败: {}", e))),
    };
    let row = row_opt.ok_or_else(|| AppError::BadRequest("采购订单不存在".to_string()))?;
    let state = row.get::<&str, _>("State").unwrap_or("").to_string();
    if state == "D" || state == "C" {
        return Err(AppError::BadRequest(
            "该采购订单已作废，无法入库".to_string(),
        ));
    }
    let po_qty = row_get_f64(&row, "Q");
    // 2) 累计已入库数量（只算已审核/已确认的 PD 采购入库）
    // 注意：RI 是领用出库（不是入库），PR 是采购退货（出库方向），都不应统计为入库
    let sql2 = "SELECT ISNULL(SUM(d.Qty), 0) AS TotalIn \
                FROM tStk_IODetail d \
                INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                WHERE io.POID = @p1 AND io.Kind = 'PD' AND io.State IN ('S', 'Y')";
    let row2_opt = conn
        .query(sql2, &[&poid])
        .await
        .map_err(|e| AppError::Internal(format!("查 PO 累计入库失败: {}", e)))?
        .into_row()
        .await
        .ok()
        .flatten();
    let already_in: f64 = row2_opt.map(|r| row_get_f64(&r, "TotalIn")).unwrap_or(0.0);
    if already_in + current_qty > po_qty + 0.0001 {
        return Err(AppError::BadRequest(format!(
            "超量入库：PO数量={} 已入库={} 本次={}",
            po_qty, already_in, current_qty
        )));
    }
    Ok(())
}

/// 校验销售订单（SOID）的存在性、状态和累计出库数量
/// DB 规则：出库数量不能超过 SO 数量；SO 已作废禁止出库
/// 注意：SR（销售退货）会释放占用的出库额度，净出库 = SD/SI/POS - SR
pub async fn validate_upstream_so(
    conn: &mut StockConn,
    soid: &str,
    current_qty: f64,
) -> Result<()> {
    if soid.is_empty() || soid == "00000000-0000-0000-0000-000000000000" {
        return Ok(());
    }
    // 1) 存在性 + 状态
    let sql = "SELECT CAST(SOID AS NVARCHAR(40)) AS ID, ISNULL(SumQty, 0) AS Q, State \
               FROM tSal_Order WHERE SOID = @p1";
    let row_opt = match conn.query(sql, &[&soid]).await {
        Ok(s) => s.into_row().await.ok().flatten(),
        Err(e) => return Err(AppError::Internal(format!("查 SO 失败: {}", e))),
    };
    let row = row_opt.ok_or_else(|| AppError::BadRequest("销售订单不存在".to_string()))?;
    let state = row.get::<&str, _>("State").unwrap_or("").to_string();
    if state == "D" || state == "C" {
        return Err(AppError::BadRequest(
            "该销售订单已作废，无法出库".to_string(),
        ));
    }
    let so_qty = row_get_f64(&row, "Q");
    // 2) 累计净出库数量 = SD/SI/POS 出库 - SR 退货（只算已审核/已确认）
    let sql2 = "SELECT \
                  ISNULL(SUM(CASE WHEN io.Kind IN ('SD','SI','POS') THEN d.Qty ELSE 0 END), 0) - \
                  ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty ELSE 0 END), 0) AS TotalOut \
                FROM tStk_IODetail d \
                INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                WHERE io.SOID = @p1 AND io.Kind IN ('SD','SI','POS','SR') AND io.State IN ('S', 'Y')";
    let row2_opt = conn
        .query(sql2, &[&soid])
        .await
        .map_err(|e| AppError::Internal(format!("查 SO 累计出库失败: {}", e)))?
        .into_row()
        .await
        .ok()
        .flatten();
    let already_out: f64 = row2_opt.map(|r| row_get_f64(&r, "TotalOut")).unwrap_or(0.0);
    if already_out + current_qty > so_qty + 0.0001 {
        return Err(AppError::BadRequest(format!(
            "超量出库：SO数量={} 已出库={} 本次={}",
            so_qty, already_out, current_qty
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct CreateIORequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_io(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<CreateIORequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let io_no = json_str(d, "IONo");
    if io_no.is_empty() {
        return Ok(Json(ApiResponse::err("IONo 不能为空")));
    }
    let kind = json_str(d, "Kind");
    if kind.is_empty() {
        return Ok(Json(ApiResponse::err(
            "Kind 必填（PD/PR/SD/SR/SI/POS/RI/O/REQ/OTO/OTI/DBO/DBI/TH/DB/ZP/OT/ADJ）",
        )));
    }
    let stk_id = json_str(d, "StkID");
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }
    let dt = now();
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    // EmpID 优先取前端传入；为空时回退到当前登录用户（claims.emp_id）
    let emp_uuid = {
        let e = json_str(d, "EmpID");
        if e.is_empty() {
            claims.emp_id.clone()
        } else {
            e
        }
    };
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let dea_uuid = empty_or_zero(&json_str(d, "DeaTypeID")).to_string();
    let po_uuid = empty_or_zero(&json_str(d, "POID")).to_string();
    let so_uuid = empty_or_zero(&json_str(d, "SOID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let us_uuid = empty_or_zero(&json_str(d, "USID")).to_string();
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let total_camt: f64 = params.details.iter().map(|x| json_f64(x, "CostAmt")).sum();
    let rsum_amt = json_f64(d, "RSumAmt");
    let disrate = json_f64(d, "DisRate");
    let downpay = json_f64(d, "DownPay");
    // TermDay 字段已从 INSERT 中移除（见下方 P5 修复注释）
    let curr = if json_str(d, "CurrCode").is_empty() {
        "CNY".to_string()
    } else {
        json_str(d, "CurrCode")
    };
    let remark = json_str(d, "Remark");
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // ===== P1-3 POID/SOID 上游单据校验 =====
    // 采购类（PD 收货/PR 退货）必须关联 PO
    if matches!(kind.as_str(), "PD" | "PR") {
        if let Err(e) = validate_upstream_po(&mut conn, &po_uuid, total_qty.abs()).await {
            return Ok(Json(ApiResponse::err(&e.to_string())));
        }
    }
    // 销售出库类（SD/SR/POS/SI）必须关联 SO
    if matches!(kind.as_str(), "SD" | "SR" | "POS" | "SI") {
        if let Err(e) = validate_upstream_so(&mut conn, &so_uuid, total_qty.abs()).await {
            return Ok(Json(ApiResponse::err(&e.to_string())));
        }
    }

    // 事务包裹：INSERT 主表 + INSERT 明细 + 库存快照回填 原子化
    // 任一明细失败回滚，避免主表残留无明细的脏数据
    let mut ioid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tStk_IO.IOID (uniqueidentifier NOT NULL, 无 DEFAULT NEWID() 约束) 必须显式赋值
        // 使用 OUTPUT INSERTED.IOID 取代 SELECT-by-IONo，消除并发竟态条件
        // P5 修复：移除 TermDay 字段 — 数据库列是 datetime，但前端传整型（账期天数），
        //   tiberius 把 i32 隐式转换为 datetime 时报"字符串数据，右截断"错误。
        //   字段语义不清且无业务代码依赖，先让数据库默认填 NULL，后续若需要再设计正确类型。
        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, SuppID, CustID, EmpID, DeptID, DeaTypeID, POID, SOID, BTPID, USID, \
            CurrCode, DisRate, DownPay, SumAmt, SumQty, SumCAmt, RSumAmt, ScanMode, State, Note, EDate, EUser) \
            OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, 'N', @p21, @p22, @p23, @p24)";
        let euser = if claims.emp_id.is_empty() { ZERO_UUID.to_string() } else { claims.emp_id.clone() };
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &kind, &stk_id, &supp_id, &cust_id, &emp_uuid, &dept_uuid, &dea_uuid, &po_uuid, &so_uuid, &btp_uuid, &us_uuid,
            &curr, &disrate, &downpay, &total_amt, &total_qty, &total_camt, &rsum_amt,
            &draft_state, &remark, &dt, &euser,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let ioid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 IOID".to_string()),
        };
        if ioid.is_empty() {
            return Err("无法获取主表 IOID".to_string());
        }

        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let cnq = if det.get("CNVQty").is_some() { json_f64(det, "CNVQty") } else { qty };
            let stdq = if det.get("StdQty").is_some() { json_f64(det, "StdQty") } else { qty };
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, UnitNO, Qty, CNVQty, StdQty, AccCheckFlg, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, 0, @p9, @p10)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &ioid, &row_no, &gdsid, &stk_id, &unit, &qty, &cnq, &stdq, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        // ===== P1-4 创建时回填 StkQty/AQty 库存快照 =====
        // 让用户在草稿状态就能看到每个仓库的当前可用量
        crate::handlers::approval::fill_io_detail_stock_snapshot(&mut conn, &ioid).await;

        ioid_out = ioid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("入出库单保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "IONo": io_no, "IOID": ioid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateIORequest {
    pub ioid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_io(
    State(_config): State<Config>,
    Json(params): Json<UpdateIORequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let io_no = json_str(d, "IONo");
    if io_no.is_empty() {
        return Ok(Json(ApiResponse::err("IONo 不能为空")));
    }
    // 状态校验：仅 N/E 允许修改，避免已审核（S/Y/C）单据被破坏性更新
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_IO WHERE IOID = @p1";
    let state = match conn
        .query(state_sql, &[&params.ioid])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许修改，请先反审",
            state
        ))));
    }
    let stk_id = json_str(d, "StkID");
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");

    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let upd = "UPDATE tStk_IO SET StkID=@p1, SumAmt=@p2, SumQty=@p3, Note=@p4, LUTime=GETDATE() WHERE IOID=@p5";
        let p: Vec<&dyn tiberius::ToSql> = vec![&stk_id, &total_amt, &total_qty, &remark, &params.ioid];
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;

        conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.ioid]).await.map_err(|e| e.to_string())?;
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let cnq = if det.get("CNVQty").is_some() { json_f64(det, "CNVQty") } else { qty };
            let stdq = if det.get("StdQty").is_some() { json_f64(det, "StdQty") } else { qty };
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, UnitNO, Qty, CNVQty, StdQty, AccCheckFlg, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, 0, @p9, @p10)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.ioid, &row_no, &gdsid, &stk_id, &unit, &qty, &cnq, &stdq, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("入出库单更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("入出库单更新成功")))
}

// ============== 调拨单 ==============
pub async fn get_move_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_Move WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (MoveNO LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreateMoveRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_move(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<CreateMoveRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let move_no = json_str(d, "MoveNO");
    if move_no.is_empty() {
        return Ok(Json(ApiResponse::err("MoveNO 不能为空")));
    }
    let kind = json_str(d, "Kind");
    if kind.is_empty() {
        return Ok(Json(ApiResponse::err("Kind 必填（DB/TH/ZP）")));
    }
    let from_stk = json_str(d, "FromStkID");
    let to_stk = json_str(d, "ToStkID");
    if from_stk.is_empty() || to_stk.is_empty() {
        return Ok(Json(ApiResponse::err("FromStkID / ToStkID 必填")));
    }
    let dt = now();
    // EmpID 优先取前端传入；为空时回退到当前登录用户（claims.emp_id）
    let emp_uuid = {
        let e = json_str(d, "EmpID");
        if e.is_empty() {
            claims.emp_id.clone()
        } else {
            e
        }
    };
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 + 库存快照回填 原子化
    let mut moveid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let sql = "INSERT INTO tStk_Move (MoveID, MoveNO, MoveDate, Kind, FromStkID, ToStkID, EmpID, RSumAmt, ScanMode, State, EDate, EUser, LUTime) \
            OUTPUT CAST(INSERTED.MoveID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, 'N', @p8, @p9, @p10, @p11)";
        let euser = if claims.emp_id.is_empty() { ZERO_UUID.to_string() } else { claims.emp_id.clone() };
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &move_no, &dt, &kind, &from_stk, &to_stk, &emp_uuid, &total_amt,
            &draft_state, &dt, &euser, &dt,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let moveid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 MoveID".to_string()),
        };
        if moveid.is_empty() {
            return Err("无法获取主表 MoveID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let cnq = if det.get("CNVQty").is_some() { json_f64(det, "CNVQty") } else { qty };
            let stdq = if det.get("StdQty").is_some() { json_f64(det, "StdQty") } else { qty };
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tStk_MoveDetail (MoveID, MoveDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, '', '', @p4, @p5, @p6, @p7, @p8, @p9)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &moveid, &row_no, &gdsid, &unit, &qty, &cnq, &stdq, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }
        // P1-4 调拨创建时回填库存快照
        crate::handlers::approval::fill_move_detail_stock_snapshot(&mut conn, &moveid).await;

        moveid_out = moveid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("调拨单保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "MoveNO": move_no, "MoveID": moveid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateMoveRequest {
    pub moveid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_move(
    State(_config): State<Config>,
    Json(params): Json<UpdateMoveRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let move_no = json_str(d, "MoveNO");
    if move_no.is_empty() {
        return Ok(Json(ApiResponse::err("MoveNO 不能为空")));
    }
    // 状态校验：仅 N/E 允许修改
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Move WHERE MoveID = @p1";
    let state = match conn
        .query(state_sql, &[&params.moveid])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许修改，请先反审",
            state
        ))));
    }
    let from_stk = json_str(d, "FromStkID");
    let to_stk = json_str(d, "ToStkID");
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let remark = json_str(d, "Remark");
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let upd = "UPDATE tStk_Move SET FromStkID=@p1, ToStkID=@p2, RSumAmt=@p3, Note=@p4, LUTime=GETDATE() WHERE MoveID=@p5";
        let p: Vec<&dyn tiberius::ToSql> = vec![&from_stk, &to_stk, &total_amt, &remark, &params.moveid];
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;

        conn.execute("DELETE FROM tStk_MoveDetail WHERE MoveID = @p1", &[&params.moveid]).await.map_err(|e| e.to_string())?;
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let cnq = if det.get("CNVQty").is_some() { json_f64(det, "CNVQty") } else { qty };
            let stdq = if det.get("StdQty").is_some() { json_f64(det, "StdQty") } else { qty };
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tStk_MoveDetail (MoveID, MoveDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, '', '', @p4, @p5, @p6, @p7, @p8, @p9)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.moveid, &row_no, &gdsid, &unit, &qty, &cnq, &stdq, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }
        // P1-4 调拨创建时回填库存快照
        crate::handlers::approval::fill_move_detail_stock_snapshot(&mut conn, &params.moveid).await;

        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("调拨单更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("调拨单更新成功")))
}

// ============== 盘点单（tStk_Tran + tStk_TranDetail）==============
pub async fn get_check_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_Tran WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (TranNo LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreateCheckRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_check(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<CreateCheckRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let tran_no = json_str(d, "TranNo");
    if tran_no.is_empty() {
        return Ok(Json(ApiResponse::err("TranNo 不能为空")));
    }
    let btp = empty_or_zero(&json_str(d, "BTPID")).to_string();
    // EmpID 优先取前端传入；为空时回退到当前登录用户（claims.emp_id）
    let emp_uuid = {
        let e = json_str(d, "EmpID");
        if e.is_empty() {
            claims.emp_id.clone()
        } else {
            e
        }
    };
    let stk_id = json_str(d, "StkID");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 + 库存快照回填 原子化
    let mut tranid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tStk_Tran.TranID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
        // 使用 OUTPUT INSERTED.TranID 取代 SELECT-by-TranNo，消除并发竟态条件
        let sql = "INSERT INTO tStk_Tran (TranID, TranNo, TranDate, BTPID, StkID, EmpID, State, EDate, EUser, LUTime) \
            OUTPUT CAST(INSERTED.TranID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
        let euser = if claims.emp_id.is_empty() { ZERO_UUID.to_string() } else { claims.emp_id.clone() };
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &tran_no, &dt, &btp, &stk_id, &emp_uuid,
            &draft_state, &dt, &euser, &dt,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let tranid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 TranID".to_string()),
        };
        if tranid.is_empty() {
            return Err("无法获取主表 TranID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let acc = json_f64(det, "AccQty");
            let real = json_f64(det, "RealQty");
            let diff = real - acc;
            let unit = json_str(det, "UnitNO");
            let ds = "INSERT INTO tStk_TranDetail (TranID, TranDetailID, RowNO, GDSID, StkID, UnitNO, AccQty, RealQty, DiffQty) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &tranid, &row_no, &gdsid, &stk_id, &unit, &acc, &real, &diff,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }
        // P1-4 盘点创建时回填库存快照
        crate::handlers::approval::fill_tran_detail_stock_snapshot(&mut conn, &tranid).await;

        tranid_out = tranid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("盘点单保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "TranNo": tran_no, "TranID": tranid_out }),
    )))
}

// ============== 补货申请 ==============
pub async fn get_replenish_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_ReplenishApply WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (ReplenishApplyNo LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct CreateReplenishRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_replenish(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<CreateReplenishRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let apply_no = json_str(d, "ReplenishApplyNo");
    if apply_no.is_empty() {
        return Ok(Json(ApiResponse::err("ReplenishApplyNo 不能为空")));
    }
    let stk_id = json_str(d, "StkID");
    let kind = if json_str(d, "Kind").is_empty() {
        "RP".to_string()
    } else {
        json_str(d, "Kind")
    };
    // EmpID 优先取前端传入；为空时回退到当前登录用户（claims.emp_id）
    let emp_uuid = {
        let e = json_str(d, "EmpID");
        if e.is_empty() {
            claims.emp_id.clone()
        } else {
            e
        }
    };
    let end_date = json_str(d, "EndDate");
    let end_dt: chrono::NaiveDateTime = if end_date.is_empty() {
        now() + chrono::Duration::days(7)
    } else {
        chrono::NaiveDateTime::parse_from_str(&end_date, "%Y-%m-%d %H:%M:%S")
            .unwrap_or_else(|_| now() + chrono::Duration::days(7))
    };
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut apply_id_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let sql = "INSERT INTO tStk_ReplenishApply (ReplenishApplyID, ReplenishApplyNo, ReplenishApplyDate, StkID, EndDate, Kind, EmpID, State, EDate, EUser) \
            OUTPUT CAST(INSERTED.ReplenishApplyID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
        let euser = if claims.emp_id.is_empty() { ZERO_UUID.to_string() } else { claims.emp_id.clone() };
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &apply_no, &dt, &stk_id, &end_dt, &kind, &emp_uuid,
            &draft_state, &dt, &euser,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let apply_id = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 ReplenishApplyID".to_string()),
        };
        if apply_id.is_empty() {
            return Err("无法获取主表 ReplenishApplyID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "ApplyQty");
            let note = json_str(det, "ApplyNote");
            let apply_dt = now();
            let ds = "INSERT INTO tStk_ReplenishApplyDtl (ReplenishApplyID, ReplenishApplyDtlID, RowNO, GDSID, UnitNO, ApplyQty, ApplyNote, ApplyDate) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &apply_id, &row_no, &gdsid, &unit, &qty, &note, &apply_dt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        apply_id_out = apply_id;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("补货申请保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "ReplenishApplyNo": apply_no, "ReplenishApplyID": apply_id_out }),
    )))
}

// ============== 库存预警 ==============
// 扫描 tStk_Stock.QQty < tBas_Goods.BttomStkQty 的商品
// DB 业务规则：BttomStkQty 是安全库存下限，QQty 跌到该值以下触发预警

#[derive(serde::Serialize)]
pub struct LowStockItem {
    pub GDSID: String,
    pub GDSNO: String,
    pub GDSDesc: String,
    pub UnitNO: String,
    pub StkID: String,
    pub StkName: String,
    pub QQty: f64,          // 当前在库可用量
    pub Qty: f64,           // 账面总库存
    pub BttomStkQty: f64,   // 安全库存下限
    pub TopStkQty: f64,     // 安全库存上限
    pub SuggestQty: f64,    // 建议补货量
    pub AlertLevel: String, // 严重等级: 紧急(QQty=0) / 警告(QQty<50%下限) / 提醒(QQty<下限)
}

#[derive(serde::Serialize)]
pub struct LowStockAlertResult {
    pub total: i32,
    pub critical: i32, // QQty = 0
    pub warning: i32,  // QQty < 50% BttomStkQty
    pub items: Vec<LowStockItem>,
}

/// POST /api/inventory/low_stock_alert
/// body: { stk_id?: string, only_active?: bool }
/// 返回所有低于安全库存的商品列表
pub async fn low_stock_alert(
    State(_config): State<Config>,
    Json(params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<LowStockAlertResult>>> {
    let mut conn = get_pool().get().await?;
    let stk_id = params
        .get("stk_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let only_active = params
        .get("only_active")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // SQL：JOIN 库存 + 商品表 + 仓库表，过滤掉停用商品
    let sql = r#"
        SELECT
            CAST(g.GDSID AS NVARCHAR(40)) AS GDSID,
            ISNULL(g.GDSNO,'') AS GDSNO,
            ISNULL(g.GDSDesc,'') AS GDSDesc,
            ISNULL(g.UnitNO,'') AS UnitNO,
            CAST(s.StkID AS NVARCHAR(40)) AS StkID,
            ISNULL(st.StkName,'') AS StkName,
            ISNULL(CAST(s.QQty AS NVARCHAR(50)),'0') AS QQty,
            ISNULL(CAST(s.Qty  AS NVARCHAR(50)),'0') AS Q,
            ISNULL(CAST(g.BttomStkQty AS NVARCHAR(50)),'0') AS BSQ,
            ISNULL(CAST(g.TopStkQty   AS NVARCHAR(50)),'0') AS TSQ
        FROM tStk_Stock s
        INNER JOIN tBas_Goods g ON g.GDSID = s.GDSID
        LEFT JOIN tBas_Stock st ON st.StkID = s.StkID
        WHERE 1=1
          AND ISNULL(s.QQty, 0) < ISNULL(g.BttomStkQty, 0)
          AND ISNULL(g.BttomStkQty, 0) > 0
          AND (@p1 = '' OR s.StkID = @p1)
          AND (@p2 = 0 OR (g.GDSStateNO IN ('1','2','3')))
        ORDER BY (ISNULL(g.BttomStkQty, 0) - ISNULL(s.QQty, 0)) DESC
    "#;
    let stk_id_param: &str = &stk_id;
    let active_filter: i32 = if only_active { 1 } else { 0 };
    let rows = conn
        .query(sql, &[&stk_id_param, &active_filter])
        .await
        .map_err(|e| AppError::Internal(format!("查预警失败: {}", e)))?
        .into_first_result()
        .await
        .unwrap_or_default();

    let mut items: Vec<LowStockItem> = Vec::new();
    let mut critical_count = 0i32;
    let mut warning_count = 0i32;
    for r in &rows {
        let gdsid = r.get::<&str, _>("GDSID").unwrap_or("").to_string();
        let gdsno = r.get::<&str, _>("GDSNO").unwrap_or("").to_string();
        let gdsdesc = r.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
        let unitno = r.get::<&str, _>("UnitNO").unwrap_or("").to_string();
        let stkid = r.get::<&str, _>("StkID").unwrap_or("").to_string();
        let stkname = r.get::<&str, _>("StkName").unwrap_or("").to_string();
        let qqty: f64 = r
            .get::<&str, _>("QQty")
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        let qty: f64 = r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0);
        let bsq: f64 = r
            .get::<&str, _>("BSQ")
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        let tsq: f64 = r
            .get::<&str, _>("TSQ")
            .unwrap_or("0")
            .parse()
            .unwrap_or(0.0);
        // 建议补货量：补到 TopStkQty（如有），否则补到 BttomStkQty*2
        let suggest = if tsq > 0.0 {
            (tsq - qqty).max(0.0)
        } else if bsq > 0.0 {
            (bsq * 2.0 - qqty).max(0.0)
        } else {
            0.0
        };
        // 严重等级
        let level: &str;
        if qqty <= 0.0001 {
            level = "紧急";
            critical_count += 1;
        } else if qqty < bsq * 0.5 {
            level = "警告";
            warning_count += 1;
        } else {
            level = "提醒";
        }
        items.push(LowStockItem {
            GDSID: gdsid,
            GDSNO: gdsno,
            GDSDesc: gdsdesc,
            UnitNO: unitno,
            StkID: stkid,
            StkName: stkname,
            QQty: qqty,
            Qty: qty,
            BttomStkQty: bsq,
            TopStkQty: tsq,
            SuggestQty: suggest,
            AlertLevel: level.to_string(),
        });
    }
    Ok(Json(ApiResponse::ok(LowStockAlertResult {
        total: items.len() as i32,
        critical: critical_count,
        warning: warning_count,
        items,
    })))
}

// ============== 预警一键转采购订单 ==============
#[derive(Deserialize)]
pub struct ReplenishFromAlertRequest {
    /// 选中的预警项（不传则全部转）
    pub items: Option<Vec<AlertItem>>,
    /// 保留兼容（不再使用，固定按 SuppID+StkID 分组）
    pub group_by_stk: Option<bool>,
}

#[derive(Deserialize, Clone)]
pub struct AlertItem {
    pub GDSID: String,
    pub StkID: String,
    /// 前端传入的供应商ID（来自库存预警行 JOIN tBas_Goods 带出），后端优先使用
    pub SuppID: Option<String>,
    /// 用户调整后的补货量（不传则自动计算 TopStkQty - Qty 并按整件取整）
    pub ApplyQty: Option<f64>,
    pub UnitNO: Option<String>,
}

/// POST /api/inventory/alerts/replenish
/// 接收预警选中的项，自动生成 tPur_Order 采购订单草稿（State='N'）
///
/// 业务规则：
///   1. 建议补货量 = 最高库存(TopStkQty) - 当前库存(Qty)，取 >=0
///   2. 按整件补货：ceil(补货量 / PackCnvQty) * PackCnvQty（PackCnvQty<=1 时不取整）
///   3. 按 (供应商 SuppID + 仓库 StkID) 分组：每个组合生成一张采购订单
///   4. 采购单价用 tBas_Goods.AInPrice（默认进价），金额 = 数量 × 单价
///   5. 生成的 PO 为草稿状态（State='N'），用户需在采购订单列表中审核确认
pub async fn replenish_from_alert(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<ReplenishFromAlertRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use crate::services::inventory_ledger::{begin_tran, commit_tran, rollback_tran};
    use crate::utils::doc_no::generate_via_docnoseq;
    use std::collections::BTreeMap;

    let mut conn = get_pool().get().await?;

    // 1) 没传 items → 自动拉所有预警并计算整件补货量
    let items: Vec<AlertItem> = if let Some(items) = params.items.clone() {
        items
    } else {
        // ★ 自动拉取预警：与前端库存预警页一致
        //   - 只拉进销(1)、新品(2) 品态的商品（排停用/只销/止销）
        //   - 只拉需要补货的：Qty < TopStkQty（当前库存低于上限）
        //   - 建议补货量 = TopStkQty - Qty，按 PackCnvQty 整件取整
        let sql = r#"
            SELECT
                CAST(s.GDSID AS NVARCHAR(40)) AS GDSID,
                CAST(s.StkID AS NVARCHAR(40)) AS StkID,
                ISNULL(g.UnitNO,'') AS UnitNO,
                ISNULL(g.TopStkQty, 0) AS TopStkQty,
                ISNULL(g.PackCnvQty, 0) AS PackCnvQty,
                ISNULL(s.Qty, 0) AS Qty,
                CAST(ISNULL(g.SuppID, '00000000-0000-0000-0000-000000000000') AS NVARCHAR(40)) AS SuppID
            FROM tStk_Stock s
            INNER JOIN tBas_Goods g ON g.GDSID = s.GDSID
            WHERE ISNULL(g.TopStkQty, 0) > 0
              AND ISNULL(s.Qty, 0) < ISNULL(g.TopStkQty, 0)
              AND g.GDSStateNO IN (1, 2)
              AND (g.GDSID IS NOT NULL AND g.GDSID <> '00000000-0000-0000-0000-000000000000')
        "#;
        let rows = conn
            .query(sql, &[])
            .await
            .map_err(|e| AppError::Internal(format!("查预警失败: {}", e)))?
            .into_first_result()
            .await
            .unwrap_or_default();
        rows.iter()
            .filter_map(|r| {
                let top = r.get::<f64, _>("TopStkQty").unwrap_or(0.0);
                let qty = r.get::<f64, _>("Qty").unwrap_or(0.0);
                let pack = r.get::<f64, _>("PackCnvQty").unwrap_or(0.0);
                let raw = (top - qty).max(0.0);
                if raw <= 0.0 {
                    return None;
                }
                // 整件取整：ceil(raw / pack) * pack
                let apply_qty = if pack > 1.0 {
                    (raw / pack).ceil() * pack
                } else {
                    raw
                };
                Some(AlertItem {
                    GDSID: r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                    StkID: r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                    SuppID: Some(r.get::<&str, _>("SuppID").unwrap_or(ZERO_UUID).to_string()),
                    UnitNO: Some(r.get::<&str, _>("UnitNO").unwrap_or("").to_string()),
                    ApplyQty: Some(apply_qty),
                })
            })
            .collect()
    };
    if items.is_empty() {
        return Ok(Json(ApiResponse::err("无预警项需要转采购订单")));
    }
    // 过滤掉 GDSID/StkID 为空或 zero-uuid 的脏数据
    let valid_items: Vec<AlertItem> = items
        .into_iter()
        .filter(|i| {
            !i.GDSID.is_empty()
                && !i.StkID.is_empty()
                && i.GDSID != ZERO_UUID
                && i.StkID != ZERO_UUID
        })
        .collect();
    if valid_items.is_empty() {
        return Ok(Json(ApiResponse::err("所有项都缺少 GDSID/StkID，无法生成")));
    }

    // 2) 查询商品补充信息（SuppID, AInPrice, GDSNO, GDSDesc, BarCode, PackCnvQty）
    //    一次查所有商品，避免逐行查询
    let gds_ids: Vec<String> = valid_items.iter().map(|i| i.GDSID.clone()).collect();
    let mut goods_info: std::collections::HashMap<
        String,
        (String, String, String, String, f64, f64, String),
    > = Default::default();
    {
        let placeholders = (0..gds_ids.len())
            .map(|i| format!("@P{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT CAST(GDSID AS NVARCHAR(40)) AS GDSID, ISNULL(GDSNO,'') AS GDSNO, \
             ISNULL(GDSDesc,'') AS GDSDesc, ISNULL(BarCode,'') AS BarCode, \
             ISNULL(AInPrice, 0) AS AInPrice, ISNULL(PackCnvQty, 0) AS PackCnvQty, \
             CAST(ISNULL(SuppID, '00000000-0000-0000-0000-000000000000') AS NVARCHAR(40)) AS SuppID \
             FROM tBas_Goods WHERE GDSID IN ({})",
            placeholders
        );
        // ★ 修复：用 &String（gds_ids.iter() 产生 &String）而非 &&str
        //   原代码 gds_id_refs.iter().map(|s| ...) 中 s 是 &&str，作为 &dyn ToSql 传给
        //   tiberius 时被当作指针地址而非字符串值，导致 WHERE GDSID IN(...) 匹配不到任何行
        //   ★ 注意：&String 通过 deref coercion 转换为 &str，tiberius 能正确识别
        let params: Vec<&dyn tiberius::ToSql> =
            gds_ids.iter().map(|s| s as &dyn tiberius::ToSql).collect();
        match conn.query(&sql, &params).await {
            Ok(rows) => {
                if let Ok(result) = rows.into_first_result().await {
                    for r in result {
                        // ★ GDSID 统一转小写：SQL Server CAST(uniqueidentifier AS NVARCHAR) 返回大写，
                        //   而 uuid::Uuid::to_string() 返回小写，HashMap key 区分大小写会导致查不到
                        let gid = r.get::<&str, _>("GDSID").unwrap_or("").to_lowercase();
                        let supp_id = r
                            .get::<&str, _>("SuppID")
                            .unwrap_or(ZERO_UUID)
                            .to_lowercase();
                        let gds_no = r.get::<&str, _>("GDSNO").unwrap_or("").to_string();
                        let gds_desc = r.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
                        let barcode = r.get::<&str, _>("BarCode").unwrap_or("").to_string();
                        let ain_price = row_get_f64(&r, "AInPrice");
                        let pack_cnv = row_get_f64(&r, "PackCnvQty");
                        goods_info.insert(
                            gid,
                            (
                                supp_id,
                                gds_no,
                                gds_desc,
                                barcode,
                                ain_price,
                                pack_cnv,
                                String::new(),
                            ),
                        );
                    }
                }
            }
            Err(e) => {
                tracing::error!("查询商品补充信息失败: {}", e);
            }
        }
        if goods_info.is_empty() {
            return Ok(Json(ApiResponse::err("查询商品信息失败，无法生成采购订单")));
        }
    }

    // 3) 查询当前库存快照（StkQty）用于 PO 明细的 StkQty 字段
    let mut stock_map: std::collections::HashMap<(String, String), f64> = Default::default();
    for item in &valid_items {
        let q = "SELECT ISNULL(Qty, 0) AS Qty FROM tStk_Stock WHERE GDSID = @p1 AND StkID = @p2";
        let p: Vec<&dyn tiberius::ToSql> = vec![&item.GDSID, &item.StkID];
        if let Ok(s) = conn.query(q, &p).await {
            if let Ok(Some(r)) = s.into_row().await {
                stock_map.insert(
                    (item.GDSID.clone(), item.StkID.clone()),
                    row_get_f64(&r, "Qty"),
                );
            }
        }
    }

    // 4) 按 (SuppID, StkID) 分组
    //    ★ SuppID 优先级：前端传入（库存预警行 JOIN tBas_Goods 带出）> tBas_Goods 查询 > zero UUID
    //      前端传值更可靠（避免 goods_info 查询失败时丢失供应商）
    //    ★ GDSID 大小写统一：goods_info 的 key 已转小写，这里查找时也要转小写
    let mut grouped: BTreeMap<(String, String), Vec<AlertItem>> = BTreeMap::new();
    for item in valid_items {
        let gid_lower = item.GDSID.to_lowercase();
        let info = goods_info.get(&gid_lower);
        let goods_supp_id = info
            .map(|(s, _, _, _, _, _, _)| s.clone())
            .unwrap_or_else(|| ZERO_UUID.to_string());
        // ★ 优先用前端传的 SuppID（非空且非 zero UUID 才用，统一转小写匹配 goods_info 中的 key）
        let supp_id = item
            .SuppID
            .as_ref()
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty() && *s != ZERO_UUID)
            .map(|s| s.to_lowercase())
            .unwrap_or(goods_supp_id);
        let key = (supp_id, item.StkID.clone());
        grouped.entry(key).or_insert_with(Vec::new).push(item);
    }

    // 5) 每组生成一张采购订单
    let dt: chrono::NaiveDateTime = chrono::Local::now().naive_local();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;
    let euser = if claims.emp_id.is_empty() {
        ZERO_UUID.to_string()
    } else {
        claims.emp_id.clone()
    };
    let mut created_docs: Vec<serde_json::Value> = Vec::new();

    for ((supp_id, stk_id), group_items) in grouped {
        // 预计算明细数据 + 合计金额
        let mut total_amt: f64 = 0.0;
        // (GDSID, GDSNO, GDSDesc, BarCode, Qty, Price, Amt, PackCnvQty, UnitNO, StkQty)
        let mut detail_data: Vec<(
            String,
            String,
            String,
            String,
            f64,
            f64,
            f64,
            f64,
            String,
            f64,
        )> = Vec::new();
        for item in &group_items {
            let qty = item.ApplyQty.unwrap_or(0.0);
            if qty <= 0.0 {
                continue;
            }
            // ★ GDSID 转小写查找 goods_info（与插入时的大小写统一）
            let gid_lower = item.GDSID.to_lowercase();
            let info = goods_info.get(&gid_lower);
            let default_info = (
                ZERO_UUID.to_string(),
                String::new(),
                String::new(),
                String::new(),
                0.0,
                0.0,
                String::new(),
            );
            let (_, gds_no, gds_desc, barcode, ain_price, pack_cnv, _) =
                info.unwrap_or(&default_info);
            let price = *ain_price;
            let amt = qty * price;
            total_amt += amt;
            let unit = item.UnitNO.clone().unwrap_or_default();
            let stk_qty = stock_map
                .get(&(item.GDSID.clone(), item.StkID.clone()))
                .copied()
                .unwrap_or(0.0);
            detail_data.push((
                item.GDSID.clone(),
                gds_no.clone(),
                gds_desc.clone(),
                barcode.clone(),
                qty,
                price,
                amt,
                *pack_cnv,
                unit,
                stk_qty,
            ));
        }
        if detail_data.is_empty() {
            continue;
        }
        let detail_count = detail_data.len();

        // 事务包裹：主表 + 明细 原子化，返回 (POID, PoNo) 或错误
        let tx_result: std::result::Result<(String, String), String> = async {
            begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

            // 生成单据号 PO{YYMM}{####}
            let po_no = generate_via_docnoseq(&mut conn, "PO").await
                .map_err(|e| format!("生成PO单号失败: {}", e))?;

            // 插入 tPur_Order 主表
            let note_text = format!("从库存预警自动生成（{} 项）", detail_count);
            let sql = "INSERT INTO tPur_Order (POID, PoNo, PoDate, StkID, SuppID, EmpID, DeptID, BTPID, DisRate, CurrCode, SumAmt, State, EDate, EUser, Note) \
                OUTPUT CAST(INSERTED.POID AS NVARCHAR(40)) AS ID \
                VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, 0, 'CNY', @p8, @p9, @p10, @p11, @p12)";
            let p: Vec<&dyn tiberius::ToSql> = vec![
                &po_no, &dt, &stk_id, &supp_id, &euser, &ZERO_UUID, &ZERO_UUID,
                &total_amt, &draft_state, &dt, &euser, &note_text,
            ];
            let poid = match conn.query(sql, &p).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
                    _ => return Err(format!("未取到 POID [{}]", po_no)),
                },
                Err(e) => return Err(format!("保存采购订单[{}]失败: {}", po_no, e)),
            };
            if poid.is_empty() {
                return Err(format!("未取到 POID [{}]", po_no));
            }

            // 插入明细（into_iter 消费 detail_data，避免 &&String/&&f64 双重引用问题）
            for (i, (gdsid, gds_no, gds_desc, barcode, qty, price, amt, pack_cnv, unit, stk_qty)) in detail_data.into_iter().enumerate() {
                let row_no = (i + 1) as i32;
                let pack_qty = if pack_cnv > 0.0 { qty / pack_cnv } else { 0.0 };
                let ds = "INSERT INTO tPur_OrderDetail (POID, PODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, BarCode, AInPrice, Price, \
                    UnitNO, CNVQty, Qty, StdQty, DisRate, Amt, TaxRate, TaxAmt, Note, PDQty, PRQty, StkQty, PackCnvQty, PackQty) \
                    VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, 0, @p14, 0, 0, @p15, 0, 0, @p16, @p17, @p18)";
                let note = "自动生成自库存预警".to_string();
                let dp: Vec<&dyn tiberius::ToSql> = vec![
                    &poid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &barcode, &price, &price,
                    &unit, &qty, &qty, &qty, &amt, &note, &stk_qty, &pack_cnv, &pack_qty,
                ];
                conn.execute(ds, &dp).await
                    .map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
            }

            commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
            Ok((poid, po_no))
        }.await;

        match tx_result {
            Ok((poid, po_no)) => {
                created_docs.push(serde_json::json!({
                    "PoNo": po_no,
                    "POID": poid,
                    "SuppID": supp_id,
                    "StkID": stk_id,
                    "DetailCount": detail_count,
                    "SumAmt": total_amt,
                    "State": "N",
                }));
            }
            Err(e) => {
                rollback_tran(&mut conn).await;
                return Ok(Json(ApiResponse::err(&format!("采购订单生成失败: {}", e))));
            }
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "CreatedCount": created_docs.len(),
        "Documents": created_docs,
    }))))
}

// ============== 补货建议（缺货记录 + 库存预警 合并视图）==============
// 将 tStk_Shortage（实际缺货需求）与 tStk_Stock（库存预警下限）按 (GDSID+StkID) 合并，
// 采购员可在单一视图中看到所有需要补货的商品及其来源（缺货/预警/两者）。
// 建议补货量 = MAX(缺货数量, 补到最高库存的量)，避免重复采购或补货不足。

#[derive(Deserialize)]
pub struct ReplenishSuggestionParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub stk_id: Option<String>,
    pub supp_id: Option<String>,
    pub source_type: Option<String>, // all / shortage / alert
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

/// POST /api/inventory/replenish-suggestions
/// 返回合并后的补货建议列表（含分页）
pub async fn get_replenish_suggestions(
    State(_config): State<Config>,
    Json(params): Json<ReplenishSuggestionParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1).max(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(200), 1000);
    let keyword = params.keyword.unwrap_or_default().trim().to_string();
    let stk_id = params.stk_id.unwrap_or_default().trim().to_string();
    let supp_id = params.supp_id.unwrap_or_default().trim().to_string();
    let source_type = params.source_type.unwrap_or_else(|| "all".to_string());

    // 构建基础 CTE：
    //   shortage_cte：未处理缺货记录按 (GDSID, StkID) 汇总
    //   alert_cte：库存低于下限的记录
    //   merged：LEFT JOIN 两者，只保留至少有一个来源的行
    let where_clause = build_suggestion_where(&keyword, &stk_id, &supp_id, &source_type);

    // 构建排序子句
    // 默认（sort_prop 为空或 SuggestQty）：按来源优先级（both > shortage > alert）+ SuggestQty DESC
    //   ★ 来源优先级排序确保最紧急的"缺货+预警"行优先展示
    // 其他字段：按用户指定字段 + sort_order 排序
    let order_clause =
        build_suggestion_order(params.sort_prop.as_deref(), params.sort_order.as_deref());

    // 计算总数
    let count_sql = format!(
        r#"
        WITH shortage_cte AS (
            SELECT sh.GDSID AS GDSID, sh.StkID AS StkID,
                   SUM(sh.ShortQty) AS ShortQty,
                   SUM(sh.ShortQty * ISNULL(g.AInPrice, 0)) AS ShortAmt,
                   MAX(sh.EDate) AS LatestShortDate,
                   MIN(sh.SourceDocNo) AS SampleDocNo,
                   COUNT(*) AS ShortCount
            FROM tStk_Shortage sh
            INNER JOIN tBas_Goods g ON g.GDSID = sh.GDSID
            WHERE sh.State = 'N' AND sh.GDSID <> '00000000-0000-0000-0000-000000000000'
            GROUP BY sh.GDSID, sh.StkID
        ),
        alert_cte AS (
            SELECT s.GDSID AS GDSID, s.StkID AS StkID,
                   ISNULL(s.Qty, 0) AS CurStockQty,
                   ISNULL(s.QQty, 0) AS CurAvailableQty,
                   ISNULL(g.BttomStkQty, 0) AS BttomStkQty,
                   ISNULL(g.TopStkQty, 0) AS TopStkQty
            FROM tStk_Stock s
            INNER JOIN tBas_Goods g ON g.GDSID = s.GDSID
            WHERE g.GDSStateNO IN (1, 2)
              AND g.State <> 'D'
              AND g.GDSID <> '00000000-0000-0000-0000-000000000000'
              AND ISNULL(g.BttomStkQty, 0) > 0
              AND ISNULL(s.Qty, 0) < ISNULL(g.BttomStkQty, 0)
        ),
        merged AS (
            SELECT
                CAST(g.GDSID AS NVARCHAR(40)) AS GDSID,
                g.GDSNO AS GDSNO, g.GDSDesc AS GDSDesc, g.GDSSpec AS GDSSpec,
                g.BarCode AS BarCode, g.GDSStateNO AS GDSStateNO,
                CAST(g.GDSTypeID AS NVARCHAR(40)) AS GDSTypeID, gt.GDSTypeName AS GDSTypeName,
                CAST(g.BrandID AS NVARCHAR(40)) AS BrandID, b.BrandName AS BrandName,
                CAST(g.SuppID AS NVARCHAR(40)) AS SuppID, s.SuppName AS SuppName,
                g.UnitNO AS UnitNO, u.UnitName AS UnitName, g.PackCnvQty AS PackCnvQty, g.AInPrice AS AInPrice,
                g.TopStkQty AS TopStkQty, g.BttomStkQty AS BttomStkQty,
                CAST(ISNULL(sk.StkID, NEWID()) AS NVARCHAR(40)) AS StkID,
                ISNULL(sk.StkName, '') AS StkName,
                ISNULL(sh.ShortQty, 0) AS ShortQty,
                ISNULL(sh.ShortAmt, 0) AS ShortAmt,
                sh.LatestShortDate AS LatestShortDate,
                sh.SampleDocNo AS SampleDocNo,
                ISNULL(sh.ShortCount, 0) AS ShortCount,
                ISNULL(al.CurStockQty, 0) AS CurStockQty,
                ISNULL(al.CurAvailableQty, 0) AS CurAvailableQty,
                -- 来源类型：shortage(仅缺货) / alert(仅预警) / both(两者都有)
                CASE
                    WHEN sh.ShortQty > 0 AND al.BttomStkQty > 0 THEN 'both'
                    WHEN sh.ShortQty > 0 THEN 'shortage'
                    ELSE 'alert'
                END AS SourceType,
                -- 建议补货量：取缺货量和(最高库存-当前库存)的最大值
                CASE
                    WHEN ISNULL(g.TopStkQty, 0) > 0 THEN
                        CASE
                            WHEN ISNULL(sh.ShortQty, 0) > (g.TopStkQty - ISNULL(al.CurStockQty, 0))
                            THEN ISNULL(sh.ShortQty, 0)
                            ELSE (g.TopStkQty - ISNULL(al.CurStockQty, 0))
                        END
                    ELSE ISNULL(sh.ShortQty, 0)
                END AS SuggestQty
            FROM tBas_Goods g
            LEFT JOIN shortage_cte sh ON sh.GDSID = g.GDSID
            LEFT JOIN alert_cte al ON al.GDSID = g.GDSID AND al.StkID = ISNULL(sh.StkID, al.StkID)
            LEFT JOIN tBas_Stock sk ON sk.StkID = ISNULL(sh.StkID, al.StkID)
            LEFT JOIN tBas_GDSType gt ON gt.GDSTypeID = g.GDSTypeID
            LEFT JOIN tBas_Brand b ON b.BrandID = g.BrandID
            LEFT JOIN tBas_Supp s ON s.SuppID = g.SuppID
            LEFT JOIN tBas_Unit u ON u.UnitNO = g.UnitNO
            WHERE g.GDSStateNO IN (1, 2)
              AND g.State <> 'D'
              AND (sh.ShortQty > 0 OR al.BttomStkQty > 0)
        )
        SELECT COUNT(*) AS cnt FROM merged WHERE 1=1 {}
        "#,
        where_clause
    );

    // 参数绑定
    let mut count_params: Vec<&dyn tiberius::ToSql> = Vec::new();
    let kw_pattern = if !keyword.is_empty() {
        format!("%{}%", keyword)
    } else {
        String::new()
    };
    let stk_param: &str = &stk_id;
    let supp_param: &str = &supp_id;
    let kw_param: &str = &kw_pattern;

    if !keyword.is_empty() {
        count_params.push(&kw_param);
    }
    if !stk_id.is_empty() {
        count_params.push(&stk_param);
    }
    if !supp_id.is_empty() {
        count_params.push(&supp_param);
    }

    let total: i64 = {
        let row = conn
            .query(&count_sql, &count_params)
            .await
            .map_err(|e| AppError::Internal(format!("查补货建议总数失败: {}", e)))?
            .into_row()
            .await
            .map_err(|e| AppError::Internal(format!("读取补货建议总数失败: {}", e)))?;
        row.and_then(|r| r.get::<i32, _>("cnt")).unwrap_or(0) as i64
    };

    // 分页查询数据
    let offset = ((page - 1) * page_size) as i64;
    let data_sql = format!(
        r#"
        WITH shortage_cte AS (
            SELECT sh.GDSID AS GDSID, sh.StkID AS StkID,
                   SUM(sh.ShortQty) AS ShortQty,
                   SUM(sh.ShortQty * ISNULL(g.AInPrice, 0)) AS ShortAmt,
                   MAX(sh.EDate) AS LatestShortDate,
                   MIN(sh.SourceDocNo) AS SampleDocNo,
                   COUNT(*) AS ShortCount
            FROM tStk_Shortage sh
            INNER JOIN tBas_Goods g ON g.GDSID = sh.GDSID
            WHERE sh.State = 'N' AND sh.GDSID <> '00000000-0000-0000-0000-000000000000'
            GROUP BY sh.GDSID, sh.StkID
        ),
        alert_cte AS (
            SELECT s.GDSID AS GDSID, s.StkID AS StkID,
                   ISNULL(s.Qty, 0) AS CurStockQty,
                   ISNULL(s.QQty, 0) AS CurAvailableQty,
                   ISNULL(g.BttomStkQty, 0) AS BttomStkQty,
                   ISNULL(g.TopStkQty, 0) AS TopStkQty
            FROM tStk_Stock s
            INNER JOIN tBas_Goods g ON g.GDSID = s.GDSID
            WHERE g.GDSStateNO IN (1, 2)
              AND g.State <> 'D'
              AND g.GDSID <> '00000000-0000-0000-0000-000000000000'
              AND ISNULL(g.BttomStkQty, 0) > 0
              AND ISNULL(s.Qty, 0) < ISNULL(g.BttomStkQty, 0)
        ),
        merged AS (
            SELECT
                CAST(g.GDSID AS NVARCHAR(40)) AS GDSID,
                g.GDSNO AS GDSNO, g.GDSDesc AS GDSDesc, g.GDSSpec AS GDSSpec,
                g.BarCode AS BarCode, g.GDSStateNO AS GDSStateNO,
                CAST(g.GDSTypeID AS NVARCHAR(40)) AS GDSTypeID, gt.GDSTypeName AS GDSTypeName,
                CAST(g.BrandID AS NVARCHAR(40)) AS BrandID, b.BrandName AS BrandName,
                CAST(g.SuppID AS NVARCHAR(40)) AS SuppID, s.SuppName AS SuppName,
                g.UnitNO AS UnitNO, u.UnitName AS UnitName, g.PackCnvQty AS PackCnvQty, g.AInPrice AS AInPrice,
                g.TopStkQty AS TopStkQty, g.BttomStkQty AS BttomStkQty,
                CAST(ISNULL(sk.StkID, NEWID()) AS NVARCHAR(40)) AS StkID,
                ISNULL(sk.StkName, '') AS StkName,
                ISNULL(sh.ShortQty, 0) AS ShortQty,
                ISNULL(sh.ShortAmt, 0) AS ShortAmt,
                sh.LatestShortDate AS LatestShortDate,
                sh.SampleDocNo AS SampleDocNo,
                ISNULL(sh.ShortCount, 0) AS ShortCount,
                ISNULL(al.CurStockQty, 0) AS CurStockQty,
                ISNULL(al.CurAvailableQty, 0) AS CurAvailableQty,
                CASE
                    WHEN sh.ShortQty > 0 AND al.BttomStkQty > 0 THEN 'both'
                    WHEN sh.ShortQty > 0 THEN 'shortage'
                    ELSE 'alert'
                END AS SourceType,
                CASE
                    WHEN ISNULL(g.TopStkQty, 0) > 0 THEN
                        CASE
                            WHEN ISNULL(sh.ShortQty, 0) > (g.TopStkQty - ISNULL(al.CurStockQty, 0))
                            THEN ISNULL(sh.ShortQty, 0)
                            ELSE (g.TopStkQty - ISNULL(al.CurStockQty, 0))
                        END
                    ELSE ISNULL(sh.ShortQty, 0)
                END AS SuggestQty
            FROM tBas_Goods g
            LEFT JOIN shortage_cte sh ON sh.GDSID = g.GDSID
            LEFT JOIN alert_cte al ON al.GDSID = g.GDSID AND al.StkID = ISNULL(sh.StkID, al.StkID)
            LEFT JOIN tBas_Stock sk ON sk.StkID = ISNULL(sh.StkID, al.StkID)
            LEFT JOIN tBas_GDSType gt ON gt.GDSTypeID = g.GDSTypeID
            LEFT JOIN tBas_Brand b ON b.BrandID = g.BrandID
            LEFT JOIN tBas_Supp s ON s.SuppID = g.SuppID
            LEFT JOIN tBas_Unit u ON u.UnitNO = g.UnitNO
            WHERE g.GDSStateNO IN (1, 2)
              AND g.State <> 'D'
              AND (sh.ShortQty > 0 OR al.BttomStkQty > 0)
        )
        SELECT * FROM merged WHERE 1=1 {}
        ORDER BY {}
        OFFSET {} ROWS FETCH NEXT {} ROWS ONLY
        "#,
        where_clause, order_clause, offset, page_size
    );

    // 参数绑定（与 count 相同）
    let mut data_params: Vec<&dyn tiberius::ToSql> = Vec::new();
    if !keyword.is_empty() {
        data_params.push(&kw_param);
    }
    if !stk_id.is_empty() {
        data_params.push(&stk_param);
    }
    if !supp_id.is_empty() {
        data_params.push(&supp_param);
    }

    let rows = conn
        .query(&data_sql, &data_params)
        .await
        .map_err(|e| AppError::Internal(format!("查补货建议失败: {}", e)))?
        .into_first_result()
        .await
        .map_err(|e| AppError::Internal(format!("读取补货建议失败: {}", e)))?;

    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let source_type = r
                .try_get::<&str, _>("SourceType")
                .ok()
                .flatten()
                .unwrap_or("alert");
            serde_json::json!({
                "GDSID": r.try_get::<&str, _>("GDSID").ok().flatten().unwrap_or(""),
                "GDSNO": r.try_get::<&str, _>("GDSNO").ok().flatten().unwrap_or(""),
                "GDSDesc": r.try_get::<&str, _>("GDSDesc").ok().flatten().unwrap_or(""),
                "GDSSpec": r.try_get::<&str, _>("GDSSpec").ok().flatten().unwrap_or(""),
                "BarCode": r.try_get::<&str, _>("BarCode").ok().flatten().unwrap_or(""),
                "GDSStateNO": r.try_get::<i32, _>("GDSStateNO").ok().flatten().unwrap_or(0),
                "GDSTypeName": r.try_get::<&str, _>("GDSTypeName").ok().flatten().unwrap_or(""),
                "BrandName": r.try_get::<&str, _>("BrandName").ok().flatten().unwrap_or(""),
                "SuppID": r.try_get::<&str, _>("SuppID").ok().flatten().unwrap_or(""),
                "SuppName": r.try_get::<&str, _>("SuppName").ok().flatten().unwrap_or(""),
                "UnitNO": r.try_get::<&str, _>("UnitNO").ok().flatten().unwrap_or(""),
                "UnitName": r.try_get::<&str, _>("UnitName").ok().flatten().unwrap_or(""),
                "PackCnvQty": row_get_f64(r, "PackCnvQty"),
                "AInPrice": row_get_f64(r, "AInPrice"),
                "TopStkQty": row_get_f64(r, "TopStkQty"),
                "BttomStkQty": row_get_f64(r, "BttomStkQty"),
                "StkID": r.try_get::<&str, _>("StkID").ok().flatten().unwrap_or(""),
                "StkName": r.try_get::<&str, _>("StkName").ok().flatten().unwrap_or(""),
                "ShortQty": row_get_f64(r, "ShortQty"),
                "ShortAmt": row_get_f64(r, "ShortAmt"),
                "LatestShortDate": r.try_get::<chrono::NaiveDateTime, _>("LatestShortDate")
                    .ok().flatten()
                    .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default(),
                "SampleDocNo": r.try_get::<&str, _>("SampleDocNo").ok().flatten().unwrap_or(""),
                "ShortCount": r.try_get::<i32, _>("ShortCount").ok().flatten().unwrap_or(0),
                "CurStockQty": row_get_f64(r, "CurStockQty"),
                "CurAvailableQty": row_get_f64(r, "CurAvailableQty"),
                "SuggestQty": row_get_f64(r, "SuggestQty"),
                "SourceType": source_type,
                "SuggestAmt": row_get_f64(r, "SuggestQty") * row_get_f64(r, "AInPrice"),
            })
        })
        .collect();

    // 统计各来源数量
    let shortage_count = items
        .iter()
        .filter(|v| {
            let st = v.get("SourceType").and_then(|s| s.as_str()).unwrap_or("");
            st == "shortage" || st == "both"
        })
        .count();
    let alert_count = items
        .iter()
        .filter(|v| {
            let st = v.get("SourceType").and_then(|s| s.as_str()).unwrap_or("");
            st == "alert" || st == "both"
        })
        .count();
    let both_count = items
        .iter()
        .filter(|v| v.get("SourceType").and_then(|s| s.as_str()).unwrap_or("") == "both")
        .count();

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
        "shortage_count": shortage_count,
        "alert_count": alert_count,
        "both_count": both_count,
    }))))
}

/// 构建补货建议的 WHERE 条件（keyword + stk_id + supp_id + source_type）
fn build_suggestion_where(keyword: &str, stk_id: &str, supp_id: &str, source_type: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut idx = 1;

    if !keyword.is_empty() {
        parts.push(format!(
            "AND (GDSNO LIKE @p{} OR GDSDesc LIKE @p{} OR BarCode LIKE @p{})",
            idx, idx, idx
        ));
        idx += 1;
    }
    if !stk_id.is_empty() {
        parts.push(format!("AND StkID = @p{}", idx));
        idx += 1;
    }
    if !supp_id.is_empty() {
        parts.push(format!("AND SuppID = @p{}", idx));
    }
    // source_type 过滤
    match source_type {
        "shortage" => parts.push("AND SourceType = 'shortage'".to_string()),
        "alert" => parts.push("AND SourceType = 'alert'".to_string()),
        "both" => parts.push("AND SourceType = 'both'".to_string()),
        _ => {} // all = 不过滤
    }

    parts.join(" ")
}

/// 构建补货建议的 ORDER BY 子句
/// - 默认（sort_prop 为空或 SuggestQty）：按来源优先级（both > shortage > alert）+ SuggestQty DESC
/// - 其他字段：按白名单字段 + sort_order 排序
///   ★ 白名单防 SQL 注入；未知字段回退到默认排序
fn build_suggestion_order(sort_prop: Option<&str>, sort_order: Option<&str>) -> String {
    const DEFAULT_ORDER: &str =
        "CASE SourceType WHEN 'both' THEN 0 WHEN 'shortage' THEN 1 ELSE 2 END, SuggestQty DESC";

    // 字段名 → SQL 排序表达式（SuggestAmt 在 CTE 里无对应列，用表达式替代）
    let allowed: &[(&str, &str)] = &[
        ("SourceType", "SourceType"),
        ("StkName", "StkName"),
        ("GDSNO", "GDSNO"),
        ("GDSDesc", "GDSDesc"),
        ("GDSTypeName", "GDSTypeName"),
        ("BrandName", "BrandName"),
        ("SuppName", "SuppName"),
        ("UnitName", "UnitName"),
        ("ShortQty", "ShortQty"),
        ("ShortCount", "ShortCount"),
        ("LatestShortDate", "LatestShortDate"),
        ("CurStockQty", "CurStockQty"),
        ("BttomStkQty", "BttomStkQty"),
        ("TopStkQty", "TopStkQty"),
        ("PackCnvQty", "PackCnvQty"),
        ("SuggestQty", "SuggestQty"),
        ("SuggestAmt", "(SuggestQty * AInPrice)"),
        ("AInPrice", "AInPrice"),
        ("SampleDocNo", "SampleDocNo"),
    ];

    let prop = sort_prop.unwrap_or("").trim();
    if prop.is_empty() || prop.eq_ignore_ascii_case("SuggestQty") {
        return DEFAULT_ORDER.to_string();
    }

    if let Some((_, expr)) = allowed.iter().find(|(k, _)| *k == prop) {
        let direction = match sort_order.unwrap_or("").to_lowercase().as_str() {
            "asc" => "ASC",
            _ => "DESC",
        };
        return format!("{} {}", expr, direction);
    }

    DEFAULT_ORDER.to_string()
}

// ============== 库存手工调整 ==============
#[derive(Deserialize)]
pub struct InventoryAdjustRequest {
    pub GDSID: String,
    pub StkID: String,
    pub Qty: f64,
    pub Reason: Option<String>,
    pub Kind: Option<String>,
}

/// 库存手工调整：走标准 IO 单 + 过账 流程
/// 1) 创建 tStk_IO (State='N' 草稿)
/// 2) 创建 tStk_IODetail (一行)
/// 3) 调用 post_ledger 过账（tStk_Stock + tStk_StockTranHis + tStk_StockYM + tStk_Qty 一次性更新）
/// 4) 将 IO 单 State 置为 'Y' (已确认) —— 因为手工调整不需走审核
///
/// 事务包裹：主表 + 明细 + 库存过账 + 状态更新 必须原子完成，
/// 任一失败回滚，避免"主表已写入但库存未扣减"或"库存已扣减但状态仍为 N"的数据不一致。
pub async fn inventory_adjust(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<InventoryAdjustRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use crate::handlers::approval::{fill_detail_stock_snapshot, post_ledger};
    use crate::services::inventory_ledger::{begin_tran, commit_tran, rollback_tran};

    if params.GDSID.is_empty() || params.StkID.is_empty() {
        return Ok(Json(ApiResponse::err("GDSID 和 StkID 必填")));
    }
    if params.Qty == 0.0 {
        return Ok(Json(ApiResponse::err("Qty 不能为 0")));
    }

    let mut conn = get_pool().get().await?;
    let kind = params.Kind.unwrap_or_else(|| "OT".to_string());
    let io_no = format!("OT{}", chrono::Local::now().format("%Y%m%d%H%M%S%3f"));
    let dt = now();
    let remark = params.Reason.unwrap_or_else(|| "手工调整".to_string());
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;
    let confirmed_state: &str = crate::handlers::doc_state::STATE_CONFIRMED;
    let direction: f64 = if params.Qty >= 0.0 { 1.0 } else { -1.0 };
    let abs_qty = params.Qty.abs();
    // EUser/AUser 用当前登录用户（claims.emp_id），原硬编码 ZERO_UUID 违反审计规则
    let euser = if claims.emp_id.is_empty() {
        ZERO_UUID.to_string()
    } else {
        claims.emp_id.clone()
    };
    let auser = euser.clone();

    // 主事务包裹：主表+明细+库存过账+状态更新，任一失败回滚
    let tx_result: std::result::Result<(String, f64), String> = async {
        begin_tran(&mut conn).await?;

        // 1) 写 tStk_IO 主表（State='N'）+ 用 OUTPUT 子句直接获取 IOID
        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, SumAmt, SumQty, SumCAmt, ScanMode, State, Note, EDate, EUser) \
                   OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
                   VALUES (NEWID(), @p1, @p2, @p3, @p4, 0, @p5, 0, 'N', @p6, @p7, @p8, @p9)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &kind, &params.StkID, &params.Qty,
            &draft_state, &remark, &dt, &euser,
        ];
        let ioid: String = match conn.query(sql, &p).await {
            Ok(s) => match s.into_row().await {
                Ok(Some(r)) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
                _ => String::new(),
            },
            Err(e) => return Err(format!("写入调整单失败: {}", e)),
        };
        if ioid.is_empty() {
            return Err("未取到 IOID".to_string());
        }

        // 2) 写 tStk_IODetail (一行)
        let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, Qty, CNVQty, StdQty, AccCheckFlg, Price, Amt) \
                  VALUES (@p1, NEWID(), 1, @p2, @p3, @p4, @p4, @p4, 0, 0, 0)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![&ioid, &params.GDSID, &params.StkID, &params.Qty];
        conn.execute(ds, &dp).await
            .map_err(|e| format!("写入明细失败: {}", e))?;

        // 抓取 IODetailID
        let detail_id: String = {
            let q = "SELECT CAST(IODetailID AS NVARCHAR(40)) AS ID FROM tStk_IODetail WHERE IOID = @p1";
            match conn.query(q, &[&ioid]).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                Err(e) => return Err(format!("查询 IODetailID 失败: {}", e)),
            }
        };
        if detail_id.is_empty() {
            return Err("未取到 IODetailID".to_string());
        }

        // 3) 调 post_ledger：方向由 Qty 符号决定（正=入库 +1，负=出库 -1）
        let (new_qty, ok) = post_ledger(
            &mut conn,
            &params.GDSID,
            &params.StkID,
            abs_qty,
            direction,
            &ioid,
            &detail_id,
            0, // 用当前月份
        ).await;
        if !ok {
            return Err(format!("库存不足: 现有{} 需求{}", new_qty, abs_qty));
        }

        // 4) 回填详情表 StkQty/AQty
        fill_detail_stock_snapshot(&mut conn, "tStk_IODetail", "IODetailID", &detail_id).await;

        // 5) 将 IO 单 State 置为 'Y' (已确认) —— 手工调整不需走审核
        let upd_state = "UPDATE tStk_IO SET State = @p1, AUser = @p2, ADate = @p3 WHERE IOID = @p4";
        let up: Vec<&dyn tiberius::ToSql> = vec![&confirmed_state, &auser, &dt, &ioid];
        conn.execute(upd_state, &up).await
            .map_err(|e| format!("更新状态失败: {}", e))?;

        commit_tran(&mut conn).await?;
        Ok((ioid, new_qty))
    }.await;

    match tx_result {
        Ok((ioid, new_qty)) => Ok(Json(ApiResponse::ok(serde_json::json!({
            "IONo": io_no,
            "IOID": ioid,
            "Delta": params.Qty,
            "NewQty": new_qty,
        })))),
        Err(e) => {
            rollback_tran(&mut conn).await;
            // P1-12 修复：库存不足时返回结构化 shortage_list，与 /api/doc/save 行为一致
            // 前端 useStockShortage 期望 res.data.shortage_list 数组，用于表格展示 + 一键删除
            if e.starts_with("库存不足") {
                // 从错误消息 "库存不足: 现有{X} 需求{Y}" 中解析当前库存 X
                // new_qty 在 Ok 分支绑定，Err 分支不可见，需从消息解析
                let current_qty = parse_current_qty_from_err(&e);
                // 查询商品和仓库名称（事务已回滚，只读查询不影响数据）
                let (gds_no, gds_name, stk_no, stk_name) =
                    query_gds_stk_names(&mut conn, &params.GDSID, &params.StkID).await;
                let shortage = (abs_qty - current_qty).max(0.0);
                let shortage_list = serde_json::json!([{
                    "row_no": 1,
                    "gds_id": params.GDSID,
                    "gds_no": gds_no,
                    "gds_name": gds_name,
                    "stk_id": params.StkID,
                    "stk_no": stk_no,
                    "stk_name": stk_name,
                    "stock": current_qty,
                    "reserved": 0.0,
                    "available": current_qty,
                    "qty": abs_qty,
                    "shortage": shortage,
                }]);
                return Ok(Json(ApiResponse::err_with_data(
                    &e,
                    "STOCK_INSUFFICIENT",
                    serde_json::json!({ "shortage_list": shortage_list }),
                )));
            }
            Ok(Json(ApiResponse::err(&e)))
        }
    }
}

/// 从库存不足错误消息中解析当前库存量
/// 错误格式："库存不足: 现有{X} 需求{Y}"
fn parse_current_qty_from_err(e: &str) -> f64 {
    if let Some(start) = e.find("现有") {
        let rest = &e[start + "现有".len()..];
        if let Some(end) = rest.find(" 需求") {
            if let Ok(q) = rest[..end].trim().parse::<f64>() {
                return q;
            }
        }
    }
    0.0
}

/// 查询商品编码/名称 + 仓库编码/名称（库存不足错误展示用）
/// 失败时返回空字符串，不阻断错误响应
async fn query_gds_stk_names(
    conn: &mut StockConn,
    gdsid: &str,
    stkid: &str,
) -> (String, String, String, String) {
    let sql = "SELECT g.GDSNO, g.GDSDesc, s.StkNO, s.StkName \
               FROM tBas_Goods g LEFT JOIN tBas_Stock s ON '1'='1' \
               WHERE g.GDSID = @p1 AND s.StkID = @p2";
    match conn.query(sql, &[&gdsid, &stkid]).await {
        Ok(stream) => match stream.into_row().await {
            Ok(Some(row)) => {
                let gds_no = row.get::<&str, _>("GDSNO").unwrap_or("").to_string();
                let gds_name = row.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
                let stk_no = row.get::<&str, _>("StkNO").unwrap_or("").to_string();
                let stk_name = row.get::<&str, _>("StkName").unwrap_or("").to_string();
                (gds_no, gds_name, stk_no, stk_name)
            }
            _ => (String::new(), String::new(), String::new(), String::new()),
        },
        Err(_) => (String::new(), String::new(), String::new(), String::new()),
    }
}

// ============== 月结（月末把上月 EndQty → 本月 InitQty）==============
#[derive(Deserialize)]
pub struct MonthSettleRequest {
    pub from_ym: i32, // 来源月份 YYYYMM，如 202605
    pub to_ym: i32,   // 目标月份 YYYYMM，如 202606
}

/// POST /api/inventory/month_settle
/// 触发月结：把 from_ym 月份的 EndQty 复制为 to_ym 月份的 InitQty
/// DB 规则：月初把上月 EndQty 复制为 InitQty
///
/// P0-12 权限校验：月结是高危操作，仅限 admin 用户执行
/// P0-12 审计日志：记录操作人、操作时间、月份范围、影响行数
/// P2-21 修复：应用级互斥锁，避免两个 admin 同时触发月结造成 tStk_StockYM 数据错乱
pub async fn month_settle(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MonthSettleRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use crate::handlers::approval::month_end_settle;
    use std::sync::atomic::{AtomicBool, Ordering};

    // P2-21：应用级互斥锁
    //   原仅 admin 权限校验，两个 admin 可同时触发月结
    //   改为：通过 AtomicBool 标志位保证单实例串行执行
    //   多实例部署需要数据库 sp_getapplock 或 Redis 分布式锁（后续扩展）
    static MONTH_SETTLE_RUNNING: AtomicBool = AtomicBool::new(false);
    if MONTH_SETTLE_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(Json(ApiResponse::err_with_code(
            "月结操作正在执行中，请稍后重试（避免并发导致库存数据错乱）",
            "OPERATION_IN_PROGRESS",
        )));
    }

    // 用 guard 模式保证异常时也能释放锁
    struct SettleGuard;
    impl Drop for SettleGuard {
        fn drop(&mut self) {
            MONTH_SETTLE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = SettleGuard;

    // 权限校验：仅 admin 可执行月结（破坏性操作，影响全公司库存核算）
    if !claims.user_code.eq_ignore_ascii_case("admin") {
        return Ok(Json(ApiResponse::err_with_code(
            "无权限执行月结操作，仅管理员（admin）可执行",
            "PERMISSION_DENIED",
        )));
    }

    let mut conn = get_pool().get().await?;

    // 简单校验
    if params.from_ym < 200001 || params.from_ym > 209912 {
        return Ok(Json(ApiResponse::err(
            "from_ym 格式应为 YYYYMM（如 202605）",
        )));
    }
    if params.to_ym < 200001 || params.to_ym > 209912 {
        return Ok(Json(ApiResponse::err("to_ym 格式应为 YYYYMM（如 202606）")));
    }
    if params.to_ym <= params.from_ym {
        return Ok(Json(ApiResponse::err("to_ym 必须大于 from_ym")));
    }

    let rows = month_end_settle(&mut conn, params.from_ym, params.to_ym).await;
    if rows < 0 {
        // 失败也记录审计日志（便于排查）
        let remark = format!(
            "月结失败：from_ym={}, to_ym={}",
            params.from_ym, params.to_ym
        );
        crate::services::inventory_ledger::record_oper(
            &mut conn,
            "POST",
            "tStk_StockYM",
            "",
            &claims.user_code,
            None,
            Some(&remark),
        )
        .await;
        return Ok(Json(ApiResponse::err("月结执行失败，请检查数据库连接")));
    }

    // 审计日志：记录月结成功
    let remark = format!(
        "月结成功：from_ym={}, to_ym={}, 影响 {} 条库存月结记录",
        params.from_ym, params.to_ym, rows
    );
    crate::services::inventory_ledger::record_oper(
        &mut conn,
        "POST",
        "tStk_StockYM",
        "",
        &claims.user_code,
        None,
        Some(&remark),
    )
    .await;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "from_ym": params.from_ym,
        "to_ym": params.to_ym,
        "settled_count": rows,
    }))))
}

#[derive(Deserialize)]
pub struct MonthSettleRollbackRequest {
    pub to_ym: i32,         // 要回滚的目标月份 YYYYMM
    pub force: Option<i32>, // 0=安全模式（默认），1=强制回滚
}

/// POST /api/inventory/month_settle_rollback
/// 月结回滚：删除指定月份的 StockYM 记录，使其恢复"未结存"状态
/// 安全策略：如果该月 inQty/OutQty 非0（已有业务活动），默认拒绝回滚
///
/// P0-12 权限校验：月结回滚是高危操作，仅限 admin 用户执行
/// P0-12 审计日志：记录操作人、操作时间、回滚月份、是否强制、影响行数
/// P2-21 修复：应用级互斥锁，避免与 month_settle 或自身并发执行
pub async fn month_settle_rollback(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<MonthSettleRollbackRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use std::sync::atomic::{AtomicBool, Ordering};

    // P2-21：应用级互斥锁
    static ROLLBACK_RUNNING: AtomicBool = AtomicBool::new(false);
    if ROLLBACK_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(Json(ApiResponse::err_with_code(
            "月结回滚操作正在执行中，请稍后重试",
            "OPERATION_IN_PROGRESS",
        )));
    }

    struct RollbackGuard;
    impl Drop for RollbackGuard {
        fn drop(&mut self) {
            ROLLBACK_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    let _guard = RollbackGuard;

    // 权限校验：仅 admin 可执行月结回滚（破坏性操作，影响库存核算）
    if !claims.user_code.eq_ignore_ascii_case("admin") {
        return Ok(Json(ApiResponse::err_with_code(
            "无权限执行月结回滚操作，仅管理员（admin）可执行",
            "PERMISSION_DENIED",
        )));
    }

    let mut conn = get_pool().get().await?;

    if params.to_ym < 200001 || params.to_ym > 209912 {
        return Ok(Json(ApiResponse::err("to_ym 格式应为 YYYYMM（如 202606）")));
    }

    let force = params.force.unwrap_or(0);

    // P5 修复：原调用不存在的存储过程 sp_MonthSettleRollback 改为 Rust 内联实现
    // 见 approval::month_rollback
    let ret = crate::handlers::approval::month_rollback(&mut conn, params.to_ym, force).await;

    // 审计日志：记录月结回滚结果（成功/失败/拒绝）
    let (audit_remark, audit_success) = match ret {
        -1 => (
            format!("月结回滚失败：参数错误 to_ym={}", params.to_ym),
            false,
        ),
        -2 => (
            format!(
                "月结回滚被拒：to_ym={} 已有业务活动（force={}）",
                params.to_ym, force
            ),
            false,
        ),
        -3 => (
            format!("月结回滚跳过：to_ym={} 无 StockYM 记录", params.to_ym),
            true,
        ),
        n if n >= 0 => (
            format!(
                "月结回滚成功：to_ym={}, force={}, 删除 {} 条记录",
                params.to_ym, force, n
            ),
            true,
        ),
        _ => (
            format!(
                "月结回滚失败：to_ym={}, force={}, 未知错误 ret={}",
                params.to_ym, force, ret
            ),
            false,
        ),
    };
    let oper_type = if audit_success { "POST" } else { "DELETE" };
    crate::services::inventory_ledger::record_oper(
        &mut conn,
        oper_type,
        "tStk_StockYM",
        "",
        &claims.user_code,
        None,
        Some(&audit_remark),
    )
    .await;

    match ret {
        -1 => Ok(Json(ApiResponse::err("参数错误：to_ym 格式应为 YYYYMM"))),
        -2 => Ok(Json(ApiResponse::err(
            "该月已有业务活动（inQty/OutQty 非0），拒绝回滚。如需强制回滚请传 force=1",
        ))),
        -3 => Ok(Json(ApiResponse::ok(serde_json::json!({
            "to_ym": params.to_ym,
            "deleted_count": 0,
            "message": "该月无 StockYM 记录，无需回滚",
        })))),
        n if n >= 0 => Ok(Json(ApiResponse::ok(serde_json::json!({
            "to_ym": params.to_ym,
            "deleted_count": n,
            "message": "月结回滚完成",
        })))),
        _ => Ok(Json(ApiResponse::err("月结回滚执行失败（未知错误）"))),
    }
}

// ============== 详情查询 ==============
#[derive(Deserialize)]
pub struct DetailParams {
    pub id: String,
}

pub async fn get_io_detail(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 查主表
    let master_sql = "SELECT * FROM tStk_IO WHERE IOID = @p1";
    let master = match conn
        .query(master_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    // 查明细
    let det_sql = "SELECT * FROM tStk_IODetail WHERE IOID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn
        .query(det_sql, &[&params.id])
        .await?
        .into_first_result()
        .await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "master": master, "details": details }),
    )))
}

pub async fn get_move_detail(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let master_sql = "SELECT * FROM tStk_Move WHERE MoveID = @p1";
    let master = match conn
        .query(master_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    let det_sql = "SELECT * FROM tStk_MoveDetail WHERE MoveID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn
        .query(det_sql, &[&params.id])
        .await?
        .into_first_result()
        .await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "master": master, "details": details }),
    )))
}

pub async fn get_check_detail(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let master_sql = "SELECT * FROM tStk_Tran WHERE TranID = @p1";
    let master = match conn
        .query(master_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    let det_sql = "SELECT * FROM tStk_TranDetail WHERE TranID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn
        .query(det_sql, &[&params.id])
        .await?
        .into_first_result()
        .await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "master": master, "details": details }),
    )))
}

// ============== 盘点单更新 ==============
#[derive(Deserialize)]
pub struct UpdateCheckRequest {
    pub tranid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_check(
    State(_config): State<Config>,
    Json(params): Json<UpdateCheckRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let stk_id = json_str(d, "StkID");
    let btp = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let note = json_str(d, "Note");
    let tran_date = json_str(d, "TranDate");
    let dt = if tran_date.is_empty() {
        now()
    } else {
        chrono::NaiveDateTime::parse_from_str(&tran_date, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&tran_date, "%Y-%m-%d"))
            .unwrap_or_else(|_| now())
    };
    // 状态校验：仅 N/E 允许修改
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Tran WHERE TranID = @p1";
    let state = match conn
        .query(state_sql, &[&params.tranid])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许修改，请先反审",
            state
        ))));
    }
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let upd = "UPDATE tStk_Tran SET StkID=@p1, BTPID=@p2, EmpID=@p3, Note=@p4, TranDate=@p5, LUTime=GETDATE() WHERE TranID=@p6";
        let p: Vec<&dyn tiberius::ToSql> = vec![&stk_id, &btp, &emp_uuid, &note, &dt, &params.tranid];
        conn.execute(upd, &p).await.map_err(|e| format!("更新主表失败: {}", e))?;

        conn.execute("DELETE FROM tStk_TranDetail WHERE TranID = @p1", &[&params.tranid]).await.map_err(|e| e.to_string())?;
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let acc = json_f64(det, "AccQty");
            let real = json_f64(det, "RealQty");
            let diff = real - acc;
            let unit = json_str(det, "UnitNO");
            let ds = "INSERT INTO tStk_TranDetail (TranID, TranDetailID, RowNO, GDSID, StkID, UnitNO, AccQty, RealQty, DiffQty) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.tranid, &row_no, &gdsid, &stk_id, &unit, &acc, &real, &diff,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("盘点单更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("盘点单更新成功")))
}

// ============== 删除单据（仅草稿/编辑中状态）==============
// 状态判断包含 E（编辑中），避免用户卡在编辑流程出不来；主表+明细删除用事务包裹避免脏数据
pub async fn delete_io(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 检查状态：D（已软删） / N（新建） / E（编辑中） 允许删除；S/Y/C 需先反审
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_IO WHERE IOID = @p1";
    let state = match conn
        .query(state_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "D" | "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许删除，请先反审",
            state
        ))));
    }
    // 事务包裹：明细删除成功但主表删除失败时避免明细丢失
    let result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.id])
            .await
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_IO WHERE IOID = @p1", &[&params.id])
            .await
            .map_err(|e| e.to_string())?;
        crate::services::inventory_ledger::commit_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("删除失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("入出库单已删除")))
}

pub async fn delete_move(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Move WHERE MoveID = @p1";
    let state = match conn
        .query(state_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "D" | "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许删除，请先反审",
            state
        ))));
    }
    let result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM tStk_MoveDetail WHERE MoveID = @p1",
            &[&params.id],
        )
        .await
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_Move WHERE MoveID = @p1", &[&params.id])
            .await
            .map_err(|e| e.to_string())?;
        crate::services::inventory_ledger::commit_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("删除失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("调拨单已删除")))
}

pub async fn delete_check(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Tran WHERE TranID = @p1";
    let state = match conn
        .query(state_sql, &[&params.id])
        .await?
        .into_row()
        .await?
    {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if !matches!(state.as_str(), "D" | "N" | "E") {
        return Ok(Json(ApiResponse::err(&format!(
            "当前状态({})不允许删除，请先反审",
            state
        ))));
    }
    let result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM tStk_TranDetail WHERE TranID = @p1",
            &[&params.id],
        )
        .await
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_Tran WHERE TranID = @p1", &[&params.id])
            .await
            .map_err(|e| e.to_string())?;
        crate::services::inventory_ledger::commit_tran(&mut conn)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("删除失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("盘点单已删除")))
}

// ============== 库存流水（tStk_StockTranHis）==============
pub async fn get_stock_flow(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT h.*, g.[GDSNO], g.[GDSDesc], g.[GDSSpec], sk.[StkName] \
                          FROM [tStk_StockTranHis] h \
                          LEFT JOIN [tBas_Goods] g ON h.[GDSID] = g.[GDSID] \
                          LEFT JOIN [tBas_Stock] sk ON h.[StkID] = sk.[StkID] \
                          WHERE 1=1"
        .to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR CAST(h.[TranID] AS NVARCHAR(40)) LIKE @p{})", pidx, pidx+1, pidx+2));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}

// ============== 单据流水（三表 UNION：tStk_IODetail + tStk_MoveDetail + tStk_TranDetail） ==============
#[derive(Deserialize)]
pub struct DocFlowParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
    pub Kind: Option<String>,
    pub StkID: Option<String>,
    pub DeptID: Option<String>,
    pub SuppID: Option<String>,
    pub CustID: Option<String>,
    /// DataPage 日期范围选择器传入的数组 [start, end]（兼容空字符串/空数组/null）
    pub DocDate: Option<serde_json::Value>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub State: Option<String>,
    /// 商品 ID 过滤（硬删除引用跳转用：查该商品的所有流水）
    pub GDSID: Option<String>,
    /// 通用 wheres 条件（DataPage focus 跳转精确查询单号用）
    /// 仅识别 field=DocNo op=eq 的条件，下推到三个子查询做精确匹配
    pub wheres: Option<Vec<serde_json::Value>>,
}

/// 解析 DocDate 参数：兼容数组 ["start","end"]、空字符串 ""、null、缺失
/// 返回 (start, end)，均为空字符串表示无日期过滤
fn parse_doc_date_range(
    doc_date: &Option<serde_json::Value>,
    start_date: &Option<String>,
    end_date: &Option<String>,
) -> (String, String) {
    if let Some(v) = doc_date {
        match v {
            serde_json::Value::Array(arr) => {
                let s = arr
                    .get(0)
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let e = arr
                    .get(1)
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                return (s, e);
            }
            serde_json::Value::String(s) => {
                if s.is_empty() {
                    return (String::new(), String::new());
                }
                return (s.clone(), String::new());
            }
            serde_json::Value::Null => {}
            _ => {}
        }
    }
    (
        start_date.clone().unwrap_or_default(),
        end_date.clone().unwrap_or_default(),
    )
}

pub async fn get_doc_flows(
    State(_config): State<Config>,
    Json(params): Json<DocFlowParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1).max(1);
    // 导出场景需较大 page_size 一次拉完；与前端 MAX_EXPORT_ROWS 对齐
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 50000);

    // 解析日期范围（提前计算，用于内联到每个 UNION 子查询）
    // 完全尊重前端传入：用户清空日期 = 查全部（无日期过滤）
    // 若只传一端，则仅按该端过滤，另一端不强制补齐
    let (start_d, end_d) =
        parse_doc_date_range(&params.DocDate, &params.start_date, &params.end_date);

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    // 构建内联 WHERE 片段（日期/状态/Kind/仓库等下推到每个子查询）
    let mut io_where = String::from("io.State NOT IN ('D','C')");
    let mut mv_where = String::from("mv.State NOT IN ('D','C')");
    let mut tr_where = String::from("tr.State NOT IN ('D','C')");
    // IO 子查询去重：
    // 1) 双边调拨类（DB/ZP/TH/OT）在 tStk_Move 也有定义，若同一单号在 IO 和 Move 都存在，
    //    IO 侧跳过避免重复；但 IO 独有的历史调拨数据（Move 没有）仍保留，用 NOT EXISTS 动态判断
    //    DBI/DBO 是单边调拨，不属于此列，仍由 IO 处理
    //    ★ OT（零散出入库）在 IO_KIND_TRANSFER 调拨类中，应与 DB/ZP/TH 一样从 IO 排除
    // 2) 盘点单（TR 前缀）在 tStk_Tran 也有定义，老系统可能在 tStk_IO 也留有同号记录（Kind 为空），
    //    若 IONo 在 tStk_Tran 已存在，IO 侧跳过，统一由 TRAN 子查询呈现，避免单号重复显示
    io_where.push_str(" AND NOT (io.Kind IN ('DB','ZP','TH','OT') AND EXISTS (SELECT 1 FROM tStk_Move mv2 WHERE mv2.MoveNO = io.IONo AND mv2.State NOT IN ('D','C')))");
    io_where.push_str(" AND NOT EXISTS (SELECT 1 FROM tStk_Tran tr2 WHERE tr2.TranNo = io.IONo AND tr2.State NOT IN ('D','C'))");

    // 状态过滤
    if let Some(st) = &params.State {
        if !st.is_empty() {
            io_where.push_str(&format!(" AND io.State = @p{}", pidx));
            mv_where.push_str(&format!(" AND mv.State = @p{}", pidx));
            tr_where.push_str(&format!(" AND tr.State = @p{}", pidx));
            query_params.push(Some(st.clone()));
            pidx += 1;
        }
    }
    // 盘点单：显示全部明细（含 DiffQty=0 的无差异行），方便查询是否盘点过
    // （用户需求：不过滤无差异明细，便于核对盘点覆盖率）
    // 日期过滤（下推到每个子查询，利用 IoDate/MoveDate/TranDate 索引）
    if !start_d.is_empty() {
        io_where.push_str(&format!(" AND io.IoDate >= @p{}", pidx));
        mv_where.push_str(&format!(" AND mv.MoveDate >= @p{}", pidx));
        tr_where.push_str(&format!(" AND tr.TranDate >= @p{}", pidx));
        query_params.push(Some(start_d.clone()));
        pidx += 1;
    }
    if !end_d.is_empty() {
        io_where.push_str(&format!(" AND io.IoDate <= @p{}", pidx));
        mv_where.push_str(&format!(" AND mv.MoveDate <= @p{}", pidx));
        tr_where.push_str(&format!(" AND tr.TranDate <= @p{}", pidx));
        query_params.push(Some(end_d.clone()));
        pidx += 1;
    }
    // Kind 过滤（下推）
    // ★ 调拨合并：Kind=DB 时 IO 子查询同时匹配 DB/DBI/DBO（DBI/DBO 是老系统单边调拨历史数据，
    //    现系统双边调拨走 tStk_Move 表 Kind=DB）。这样搜索"调拨单"能同时查到新/旧调拨数据。
    if let Some(k) = &params.Kind {
        if !k.is_empty() {
            if k == "DB" {
                // 调拨：IO 表匹配 DB/DBI/DBO，Move 表匹配 DB
                io_where.push_str(&format!(" AND io.Kind IN ('DB','DBI','DBO')"));
                mv_where.push_str(&format!(" AND mv.Kind = @p{}", pidx));
                tr_where.push_str(" AND 1=0");
            } else {
                io_where.push_str(&format!(" AND io.Kind = @p{}", pidx));
                mv_where.push_str(&format!(" AND mv.Kind = @p{}", pidx));
                // TRAN 的 Kind 固定为 'TR'，只有 Kind=TR 时才需要查
                if k == "TR" {
                    // 保留 tr_where 不加 Kind 条件（TRAN 全部是 TR）
                } else {
                    // 非 TR 时，TRAN 子查询返回空
                    tr_where.push_str(" AND 1=0");
                }
            }
            query_params.push(Some(k.clone()));
            pidx += 1;
        }
    }
    // 仓库过滤（下推）：每条流水只属于一个仓库，按发生仓库精确匹配
    //   - IO / TRAN：StkID 直接匹配
    //   - Move 调拨：每条明细拆成2行（出库 FromStkID + 入库 ToStkID），
    //     按仓库筛选时只显示该仓库对应的那行流水（A 调出给 B，筛 A 只看出库行）
    if let Some(s) = &params.StkID {
        if !s.is_empty() {
            io_where.push_str(&format!(" AND io.StkID = @p{}", pidx));
            mv_where.push_str(&format!(" AND ((dir.Direction = -1 AND mv.FromStkID = @p{}) OR (dir.Direction = 1 AND mv.ToStkID = @p{}))", pidx, pidx));
            tr_where.push_str(&format!(" AND tr.StkID = @p{}", pidx));
            query_params.push(Some(s.clone()));
            pidx += 1;
        }
    }
    // 调拨仓库过滤（下推）：仅对调拨类（MOVE）按对方仓库过滤
    //   - IO：采购/销售单据无调拨仓库概念，不应用此过滤
    //   - Move：按对方仓库匹配（出库行看去向 ToStkID，入库行看来源 FromStkID）
    //   - TRAN：盘点单无调拨仓库概念，跳过
    if let Some(d) = &params.DeptID {
        if !d.is_empty() {
            mv_where.push_str(&format!(" AND ((dir.Direction = -1 AND mv.ToStkID = @p{}) OR (dir.Direction = 1 AND mv.FromStkID = @p{}))", pidx, pidx));
            // IO/TRAN 无调拨仓库概念，跳过
            io_where.push_str(" AND 1=0");
            tr_where.push_str(" AND 1=0");
            query_params.push(Some(d.clone()));
            pidx += 1;
        }
    }
    // 供应商过滤（仅 IO 有）
    if let Some(s) = &params.SuppID {
        if !s.is_empty() {
            io_where.push_str(&format!(" AND io.SuppID = @p{}", pidx));
            query_params.push(Some(s.clone()));
            pidx += 1;
        }
    }
    // 客户过滤（仅 IO 有）
    if let Some(c) = &params.CustID {
        if !c.is_empty() {
            io_where.push_str(&format!(" AND io.CustID = @p{}", pidx));
            query_params.push(Some(c.clone()));
            pidx += 1;
        }
    }
    // 商品 GDSID 过滤（下推到三个子查询的明细表 d.GDSID）
    // 记录 gdsid_pidx 供期初子查询复用同一参数（避免重复 push）
    let mut gdsid_pidx: usize = 0;
    if let Some(g) = &params.GDSID {
        if !g.is_empty() {
            gdsid_pidx = pidx;
            io_where.push_str(&format!(" AND d.GDSID = @p{}", pidx));
            mv_where.push_str(&format!(" AND d.GDSID = @p{}", pidx));
            tr_where.push_str(&format!(" AND d.GDSID = @p{}", pidx));
            query_params.push(Some(g.clone()));
            pidx += 1;
        }
    }
    // 通用 wheres：仅识别 field=DocNo op=eq（DataPage focus 跳转精确查询单号）
    // 下推到三个子查询做等值匹配，利用 IONo/MoveNO/TranNo 索引秒查
    if let Some(ws) = &params.wheres {
        for w in ws {
            let field = w.get("field").and_then(|v| v.as_str()).unwrap_or("");
            let op = w.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let value = w.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if field == "DocNo" && op == "eq" && !value.is_empty() {
                io_where.push_str(&format!(" AND io.IONo = @p{}", pidx));
                mv_where.push_str(&format!(" AND mv.MoveNO = @p{}", pidx));
                tr_where.push_str(&format!(" AND tr.TranNo = @p{}", pidx));
                query_params.push(Some(value.to_string()));
                pidx += 1;
            }
        }
    }

    // 关键词搜索：下推到子查询（DocNo + 商品编码/名称）
    // ★ 必须下推：TOP N 下推时子查询取前 N 条（按日期排序），若 keyword 在外层过滤，
    //   子查询的 TOP N 可能不含目标记录 → 分页查询返回空但 count 正确
    //   （bug 现象：搜单号查不到，加 Kind 过滤后能查到）
    let outer_where = String::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            let kw_pat = format!("%{}%", kw);
            // 下推到三个子查询：DocNo + 商品编码/名称（与外层 GoodsGDSNO/GoodsGDSDesc 逻辑一致）
            io_where.push_str(&format!(
                " AND (io.IONo LIKE @p{} OR ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) LIKE @p{} OR ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) LIKE @p{})",
                pidx, pidx, pidx
            ));
            mv_where.push_str(&format!(
                " AND (mv.MoveNO LIKE @p{} OR ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) LIKE @p{} OR ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) LIKE @p{})",
                pidx, pidx, pidx
            ));
            tr_where.push_str(&format!(
                " AND (tr.TranNo LIKE @p{} OR ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) LIKE @p{} OR ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) LIKE @p{})",
                pidx, pidx, pidx
            ));
            query_params.push(Some(kw_pat));
            pidx += 1;
        }
    }

    // 性能优化：当排序字段为 DocDate 时，把 TOP N + ORDER BY 下推到每个子查询
    // 让每个子查询用自己的日期索引（IoDate/MoveDate/TranDate）快速取前 N 条，
    // 然后外层合并再排序分页。否则三表 UNION 1130 万行全局排序会超时。
    // 实测：无日期范围 page=1 从 30s 超时 → 380ms（提速 80倍+）
    //
    // ★ 结存计算与 TOP N 下推的冲突：
    //   窗口函数 SUM() OVER(PARTITION BY ... ORDER BY ...) 只能看到 TOP N 子集，
    //   若某商品流水超过 N 条，运行总和只覆盖子集，导致结存错误。
    //   解决：单商品查询（有 GDSID 过滤）时禁用 TOP N 下推，让窗口函数看到全量数据。
    //   单商品数据量通常几百~几千行，全量排序性能可接受。
    //   多商品查询时保持 TOP N 下推（结存在子集上计算，跨页可能不连续，但多商品结存意义不大）。
    let sort_prop = params.sort_prop.as_deref().unwrap_or("");
    let sort_order = params.sort_order.as_deref().unwrap_or("desc");
    let is_docdate_sort = sort_prop == "DocDate";
    let has_gdsid_filter = params
        .GDSID
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    // TOP N 下推条件：按 DocDate 排序 且 无 GDSID 过滤（单商品查询需全量计算结存）
    let use_top_pushdown = is_docdate_sort && !has_gdsid_filter;
    let top_n = (page * page_size) as u32; // 子查询取前 N 条用于外层分页
    let direction = if sort_order.eq_ignore_ascii_case("asc") {
        "ASC"
    } else {
        "DESC"
    };
    // 子查询内部的 ORDER BY 片段（仅启用下推时生效）
    let io_order = if use_top_pushdown {
        format!(" ORDER BY io.IoDate {}", direction)
    } else {
        String::new()
    };
    let mv_order = if use_top_pushdown {
        format!(" ORDER BY mv.MoveDate {}", direction)
    } else {
        String::new()
    };
    let tr_order = if use_top_pushdown {
        format!(" ORDER BY tr.TranDate {}", direction)
    } else {
        String::new()
    };
    // 子查询前缀：TOP N（启用下推时）；否则空（全表扫由外层分页）
    let io_top = if use_top_pushdown {
        format!("TOP ({}) ", top_n)
    } else {
        String::new()
    };
    let mv_top = if use_top_pushdown {
        format!("TOP ({}) ", top_n)
    } else {
        String::new()
    };
    let tr_top = if use_top_pushdown {
        format!("TOP ({}) ", top_n)
    } else {
        String::new()
    };

    // 三表 UNION 基础 SQL（过滤条件已下推到每个子查询）
    // ★ 基本设计：每条流水 = 一次仓库库存变动，只有一列「仓库」+「方向(入库+1/出库-1)」
    //   - IO  / TRAN：每条明细1行流水，仓库=业务仓库，方向由 Kind/DiffQty 决定
    //   - Move 调拨 ：每条明细拆成2行（调出仓出库-1 + 调入仓入库+1），用 CROSS JOIN (VALUES) 展开
    //                 这样自然体现两仓库对调，避免「来源/去向」两列的语义混乱
    // Direction: +1=入库, -1=出库
    //
    // ★ 结存计算（Balance 列）：
    //   结存 = 期初库存 + 查询范围内截至当前行的累计净变动
    //   期初库存 = 该商品该仓库在查询起始日期之前的所有已审核流水净变动之和
    //   实现方式：
    //     1) 用 CTE 先算出每个 (GDSID, StkID) 的期初库存 opening_qty
    //     2) 外层用 SUM(Qty*Direction) OVER (PARTITION BY GDSID, StkID ORDER BY DocDate, DocNo) 算运行总和
    //     3) Balance = opening_qty + 当前行的运行总和（含当前行）
    //   注意：期初库存子查询不应用 Kind/供应商/客户等过滤（只受日期+仓库+商品+状态限制），
    //         否则期初会偏小，导致结存不准
    // 期初库存 JOIN 子查询（不含 CTE，避免 build_pagination_sql_with_sort 外层包装时 CTE 嵌套非法）
    // ★ 只在单商品查询时构建：多商品查询不计算 Balance，无需期初，避免三表全历史扫描拖慢
    // ★ 关键优化：期初子查询也下推 GDSID 过滤（复用 @p{gdsid_pidx} 参数）
    //   未下推时扫描三表全部历史数据计算所有商品期初，但单商品查询只需一个商品，
    //   浪费极大。下推后只扫描该商品的历史数据，性能提升数十倍。
    let opening_join = if has_gdsid_filter && !start_d.is_empty() {
        // 有起始日期时才算期初；无起始日期时期初=0（从最早开始累加）
        // ★ 注册 @p{opening_pidx} 参数 = start_d（期初截止日期）
        //   同一个参数名在 SQL 中可重复引用，只需 push 一次
        let opening_pidx = pidx;
        query_params.push(Some(start_d.clone()));
        format!(
            "\
LEFT JOIN ( \
  SELECT GDSID, StkID, SUM(Qty * Direction) AS opening_qty FROM ( \
    SELECT d.GDSID AS GDSID, io.StkID AS StkID, \
           ISNULL(CAST(d.Qty AS FLOAT),0) AS Qty, \
           CASE io.Kind WHEN 'PD' THEN 1 WHEN 'SR' THEN 1 WHEN 'OTI' THEN 1 WHEN 'DBI' THEN 1 \
                        WHEN 'SD' THEN -1 WHEN 'SI' THEN -1 WHEN 'POS' THEN -1 \
                        WHEN 'RI' THEN -1 WHEN 'PR' THEN -1 WHEN 'OTO' THEN -1 \
                        WHEN 'ADJ' THEN -1 WHEN 'O' THEN -1 WHEN 'REQ' THEN -1 \
                        WHEN 'DBO' THEN -1 ELSE 0 END AS Direction \
    FROM tStk_IODetail d \
    LEFT JOIN tStk_IO io ON d.IOID = io.IOID \
    WHERE io.State NOT IN ('D','C') AND io.IoDate < @p{opening_pidx} AND d.GDSID = @p{gdsid_pidx} \
      AND NOT (io.Kind IN ('DB','ZP','TH','OT') AND EXISTS (SELECT 1 FROM tStk_Move mv2 WHERE mv2.MoveNO = io.IONo AND mv2.State NOT IN ('D','C'))) \
      AND NOT EXISTS (SELECT 1 FROM tStk_Tran tr2 WHERE tr2.TranNo = io.IONo AND tr2.State NOT IN ('D','C')) \
    UNION ALL \
    SELECT d.GDSID AS GDSID, \
           CASE WHEN dir.Direction = -1 THEN mv.FromStkID ELSE mv.ToStkID END AS StkID, \
           ISNULL(CAST(d.Qty AS FLOAT),0) AS Qty, dir.Direction AS Direction \
    FROM tStk_MoveDetail d \
    LEFT JOIN tStk_Move mv ON d.MoveID = mv.MoveID \
    CROSS JOIN (VALUES (-1), (1)) AS dir(Direction) \
    WHERE mv.State NOT IN ('D','C') AND mv.MoveDate < @p{opening_pidx} AND d.GDSID = @p{gdsid_pidx} \
    UNION ALL \
    SELECT d.GDSID AS GDSID, tr.StkID AS StkID, \
           ISNULL(CAST(d.DiffQty AS FLOAT),0) AS Qty, \
           CASE WHEN d.DiffQty > 0 THEN 1 WHEN d.DiffQty < 0 THEN -1 ELSE 0 END AS Direction \
    FROM tStk_TranDetail d \
    LEFT JOIN tStk_Tran tr ON d.TranID = tr.TranID \
    WHERE tr.State NOT IN ('D','C') AND tr.TranDate < @p{opening_pidx} AND d.GDSID = @p{gdsid_pidx} \
  ) h GROUP BY GDSID, StkID \
) o ON flow.GDSID = o.GDSID AND flow.StkID = o.StkID",
            opening_pidx = opening_pidx,
            gdsid_pidx = gdsid_pidx
        )
    } else {
        String::new()
    };
    let base_query = format!(
        "\
SELECT * FROM ( \
  SELECT {io_top}d.IODetailID AS DetailID, io.IONo AS DocNo, io.IoDate AS DocDate, io.Kind AS Kind, \
         CAST(io.BTPID AS NVARCHAR(40)) AS BTPID, \
         CASE WHEN io.Kind='PD' THEN '采购入库' WHEN io.Kind='OTI' THEN '零散入库' \
              WHEN io.Kind='OTO' THEN '零散出库' WHEN io.Kind='DBI' THEN '调拨转入' \
              WHEN io.Kind='DBO' THEN '调拨转出' WHEN io.Kind='PR' THEN '采购退货' \
              WHEN io.Kind='RI' THEN '领用' WHEN io.Kind='O' THEN '领用出库' \
              WHEN io.Kind='REQ' THEN '领用申请' WHEN io.Kind='SI' THEN '门店销售' \
              WHEN io.Kind='POS' THEN 'POS收银' WHEN io.Kind='ADJ' THEN '库存调整' \
              WHEN io.Kind='OT' THEN '零散出入库' \
              WHEN io.Kind='SR' THEN '销售退货' \
              WHEN io.Kind='SD' THEN '销售出库' \
              ELSE ISNULL(io.Kind,'') END AS KindText, \
         CAST(d.GDSID AS NVARCHAR(40)) AS GDSID, \
         ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, \
         ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc, \
         u.UnitName, sk.StkName, \
         ISNULL(CAST(d.Qty AS FLOAT),0) AS Qty, ISNULL(CAST(d.Price AS FLOAT),0) AS Price, \
         ISNULL(CAST(d.Amt AS FLOAT),0) AS Amt, CAST(NULL AS FLOAT) AS CostAmt, \
         CASE io.Kind WHEN 'PD' THEN 1 WHEN 'SR' THEN 1 WHEN 'OTI' THEN 1 WHEN 'DBI' THEN 1 \
                      WHEN 'SD' THEN -1 WHEN 'SI' THEN -1 WHEN 'POS' THEN -1 \
                      WHEN 'RI' THEN -1 WHEN 'PR' THEN -1 WHEN 'OTO' THEN -1 \
                      WHEN 'ADJ' THEN -1 WHEN 'O' THEN -1 WHEN 'REQ' THEN -1 \
                      WHEN 'DBO' THEN -1 \
                      ELSE 0 END AS Direction, \
         io.State AS State, ISNULL(io.Note,'') AS Note, \
         CAST(io.SuppID AS NVARCHAR(40)) AS SuppID, CAST(io.CustID AS NVARCHAR(40)) AS CustID, \
         CAST(io.EmpID AS NVARCHAR(40)) AS EmpID, CAST(io.DeptID AS NVARCHAR(40)) AS DeptID, \
         CAST(io.StkID AS NVARCHAR(40)) AS StkID, \
         ISNULL(s.SuppName,'') AS SuppName, ISNULL(c.CustName,'') AS CustName, ISNULL(e.EmpName,'') AS EmpName, \
         CAST(NULL AS NVARCHAR(60)) AS DeptName, \
         'IO' AS SourceType \
  FROM tStk_IODetail d \
  LEFT JOIN tStk_IO io ON d.IOID = io.IOID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  LEFT JOIN tBas_Unit u ON g.UnitNO = u.UnitNO \
  LEFT JOIN tBas_Stock sk ON io.StkID = sk.StkID \
  LEFT JOIN tBas_Supp s ON io.SuppID = s.SuppID \
  LEFT JOIN tBas_Cust c ON io.CustID = c.CustID \
  LEFT JOIN tBas_Emp e ON io.EmpID = e.EmpID \
  WHERE {io_where}{io_order} \
  UNION ALL \
  SELECT {mv_top}d.MoveDetailID AS DetailID, mv.MoveNO AS DocNo, mv.MoveDate AS DocDate, mv.Kind AS Kind, \
         CAST(NULL AS NVARCHAR(40)) AS BTPID, \
         CASE WHEN mv.Kind='DB' AND dir.Direction=-1 THEN '调拨转出' \
              WHEN mv.Kind='DB' AND dir.Direction=1 THEN '调拨转入' \
              WHEN mv.Kind='ZP' AND dir.Direction=-1 THEN '门店直配发货' \
              WHEN mv.Kind='ZP' AND dir.Direction=1 THEN '门店直配收货' \
              WHEN mv.Kind='TH' AND dir.Direction=-1 THEN '门店退仓' \
              WHEN mv.Kind='TH' AND dir.Direction=1 THEN '门店退入' \
              WHEN mv.Kind='OT' THEN '零散出入库' \
              ELSE ISNULL(mv.Kind,'') END AS KindText, \
         CAST(d.GDSID AS NVARCHAR(40)) AS GDSID, \
         ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, \
         ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc, \
         u.UnitName, \
         CASE WHEN dir.Direction = -1 THEN fs.StkName ELSE ts.StkName END AS StkName, \
         ISNULL(CAST(d.Qty AS FLOAT),0) AS Qty, ISNULL(CAST(d.Price AS FLOAT),0) AS Price, \
         ISNULL(CAST(d.Amt AS FLOAT),0) AS Amt, CAST(NULL AS FLOAT) AS CostAmt, \
         dir.Direction AS Direction, \
         mv.State AS State, ISNULL(mv.Note,'') AS Note, \
         CAST(NULL AS NVARCHAR(40)) AS SuppID, CAST(NULL AS NVARCHAR(40)) AS CustID, \
         CAST(mv.EmpID AS NVARCHAR(40)) AS EmpID, CAST(NULL AS NVARCHAR(40)) AS DeptID, \
         CASE WHEN dir.Direction = -1 THEN CAST(mv.FromStkID AS NVARCHAR(40)) ELSE CAST(mv.ToStkID AS NVARCHAR(40)) END AS StkID, \
         CAST(NULL AS NVARCHAR(100)) AS SuppName, CAST(NULL AS NVARCHAR(100)) AS CustName, \
         ISNULL(e.EmpName,'') AS EmpName, \
         CASE WHEN dir.Direction = -1 THEN ISNULL(ts.StkName,'') ELSE ISNULL(fs.StkName,'') END AS DeptName, \
         'MOVE' AS SourceType \
  FROM tStk_MoveDetail d \
  LEFT JOIN tStk_Move mv ON d.MoveID = mv.MoveID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  LEFT JOIN tBas_Unit u ON g.UnitNO = u.UnitNO \
  LEFT JOIN tBas_Stock fs ON mv.FromStkID = fs.StkID \
  LEFT JOIN tBas_Stock ts ON mv.ToStkID = ts.StkID \
  LEFT JOIN tBas_Emp e ON mv.EmpID = e.EmpID \
  CROSS JOIN (VALUES (-1), (1)) AS dir(Direction) \
  WHERE {mv_where}{mv_order} \
  UNION ALL \
  SELECT {tr_top}d.TranDetailID AS DetailID, tr.TranNo AS DocNo, tr.TranDate AS DocDate, 'TR' AS Kind, \
         CAST(NULL AS NVARCHAR(40)) AS BTPID, \
         CASE WHEN d.DiffQty > 0 THEN '盘盈' WHEN d.DiffQty < 0 THEN '盘损' ELSE '盘点' END AS KindText, \
         CAST(d.GDSID AS NVARCHAR(40)) AS GDSID, \
         ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, \
         ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc, \
         u.UnitName, sk.StkName, \
         ISNULL(CAST(d.DiffQty AS FLOAT),0) AS Qty, CAST(NULL AS FLOAT) AS Price, \
         CAST(NULL AS FLOAT) AS Amt, CAST(NULL AS FLOAT) AS CostAmt, \
         CASE WHEN d.DiffQty > 0 THEN 1 WHEN d.DiffQty < 0 THEN -1 ELSE 0 END AS Direction, \
         tr.State AS State, ISNULL(tr.Note,'') AS Note, \
         CAST(NULL AS NVARCHAR(40)) AS SuppID, CAST(NULL AS NVARCHAR(40)) AS CustID, \
         CAST(tr.EmpID AS NVARCHAR(40)) AS EmpID, CAST(NULL AS NVARCHAR(40)) AS DeptID, \
         CAST(tr.StkID AS NVARCHAR(40)) AS StkID, \
         CAST(NULL AS NVARCHAR(100)) AS SuppName, CAST(NULL AS NVARCHAR(100)) AS CustName, \
         ISNULL(e.EmpName,'') AS EmpName, CAST(NULL AS NVARCHAR(100)) AS DeptName, \
         'TRAN' AS SourceType \
  FROM tStk_TranDetail d \
  LEFT JOIN tStk_Tran tr ON d.TranID = tr.TranID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  LEFT JOIN tBas_Unit u ON g.UnitNO = u.UnitNO \
  LEFT JOIN tBas_Stock sk ON tr.StkID = sk.StkID \
  LEFT JOIN tBas_Emp e ON tr.EmpID = e.EmpID \
  WHERE {tr_where}{tr_order} \
) AS flow WHERE 1=1{outer_where}",
        io_where = io_where,
        mv_where = mv_where,
        tr_where = tr_where,
        outer_where = outer_where,
        io_top = io_top,
        mv_top = mv_top,
        tr_top = tr_top,
        io_order = io_order,
        mv_order = mv_order,
        tr_order = tr_order
    );

    // ★ 结存计算：只在单商品查询（有 GDSID 过滤）时计算 Balance
    //   多商品查询时不同商品的结存混在一起无意义，且窗口函数需全局排序、期初子查询
    //   需扫描三表全部历史数据，导致多商品查询从 380ms 暴涨到数十秒。
    //   单商品数据量小（通常几百~几千行），窗口函数+期初子查询性能可接受。
    //   多商品查询时 Balance 返回 NULL，前端显示为空。
    //
    //   Balance = ISNULL(期初库存, 0) + SUM(Qty*Direction) OVER (PARTITION BY GDSID, StkID ORDER BY DocDate, DocNo, DetailID)
    //   期初库存来自 opening_join 子查询（仅在有起始日期时存在）
    //   运行总和含当前行，确保结存 = 该行发生后的最新余额
    //   ★ 使用 LEFT JOIN 子查询而非 CTE（build_pagination_sql_with_sort 外层包装时 CTE 嵌套非法）
    //   ★ ORDER BY 加 DetailID 第三排序字段，确保同单据同商品多行时排序稳定
    let balance_query = if has_gdsid_filter {
        if !start_d.is_empty() {
            // 单商品 + 有起始日期：期初 + 运行总和
            format!(
                "\
SELECT flow.*, \
       ISNULL(o.opening_qty, 0) \
       + ISNULL(SUM(flow.Qty * flow.Direction) OVER (PARTITION BY flow.GDSID, flow.StkID ORDER BY flow.DocDate, flow.DocNo, flow.DetailID), 0) AS Balance \
FROM ({base_query}) flow \
{opening_join}",
                base_query = base_query,
                opening_join = opening_join
            )
        } else {
            // 单商品 + 无起始日期：期初=0，直接运行总和
            format!(
                "\
SELECT flow.*, \
       ISNULL(SUM(flow.Qty * flow.Direction) OVER (PARTITION BY flow.GDSID, flow.StkID ORDER BY flow.DocDate, flow.DocNo, flow.DetailID), 0) AS Balance \
FROM ({base_query}) flow",
                base_query = base_query
            )
        }
    } else {
        // 多商品查询：不计算 Balance，返回 NULL（保持列结构一致，避免前端报错）
        format!(
            "\
SELECT flow.*, CAST(NULL AS FLOAT) AS Balance \
FROM ({base_query}) flow",
            base_query = base_query
        )
    };

    // COUNT 查询不能用下推后的 base_query（含 TOP N），需要构造一个无 TOP 的版本
    // 否则 total 会变成 3*top_n，不是真实总数
    // 同时需 SELECT 出关键词搜索相关字段（DocNo/GoodsGDSNO/GoodsGDSDesc）以支持 outer_where
    let count_sql = if is_docdate_sort {
        // 简化版 count：只 SELECT 关键词字段，但 Move 子查询必须保留 CROSS JOIN
        // 因为 mv_where 中的仓库过滤引用了 dir.Direction
        let count_query = format!(
            "\
SELECT COUNT(*) as cnt FROM ( \
  SELECT io.IONo AS DocNo, ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc \
  FROM tStk_IODetail d \
  LEFT JOIN tStk_IO io ON d.IOID = io.IOID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  WHERE {io_where} \
  UNION ALL \
  SELECT mv.MoveNO AS DocNo, ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc \
  FROM tStk_MoveDetail d \
  LEFT JOIN tStk_Move mv ON d.MoveID = mv.MoveID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  CROSS JOIN (VALUES (-1), (1)) AS dir(Direction) \
  WHERE {mv_where} \
  UNION ALL \
  SELECT tr.TranNo AS DocNo, ISNULL(NULLIF(d.GDSNO,''), g.GDSNO) AS GoodsGDSNO, ISNULL(NULLIF(d.GDSDesc,''), g.GDSDesc) AS GoodsGDSDesc \
  FROM tStk_TranDetail d \
  LEFT JOIN tStk_Tran tr ON d.TranID = tr.TranID \
  LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID \
  WHERE {tr_where} \
) t WHERE 1=1{outer_where}",
            io_where = io_where,
            mv_where = mv_where,
            tr_where = tr_where,
            outer_where = outer_where
        );
        count_query
    } else {
        format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query)
    };
    // ★ 关键：分页基于 balance_query（含 Balance 列），而非 base_query
    //   之前用 base_query 导致 Balance 列从未被查询，前端拿不到后端结存，只能本地从 0 累加
    let paginated_sql = build_pagination_sql_with_sort(
        &balance_query,
        page,
        page_size,
        params.sort_prop.as_deref(),
        params.sort_order.as_deref(),
    );
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();
    let mut total: i32 = 0;
    if let Some(row) = conn
        .query(&count_sql, &param_refs)
        .await?
        .into_row()
        .await?
    {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn
        .query(&paginated_sql, &param_refs)
        .await?
        .into_first_result()
        .await?;
    Ok(Json(ApiResponse::ok_paginated(
        rows.iter().map(row_to_json).collect(),
        total as u64,
        page,
        page_size,
    )))
}
