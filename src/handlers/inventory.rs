use axum::{extract::State, Json};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::{AppError, Result};
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use super::base_data::try_get_value;
use crate::handlers::approval::Conn as StockConn;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" { continue; }
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn json_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn json_i32(v: &serde_json::Value, key: &str) -> i32 {
    v.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
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
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page, page_size)))
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
        return Err(AppError::BadRequest("该采购订单已作废，无法入库".to_string()));
    }
    let po_qty = row_get_f64(&row, "Q");
    // 2) 累计已入库数量（只算已审核/已确认的 RI/PD）
    let sql2 = "SELECT ISNULL(SUM(d.Qty), 0) AS TotalIn \
                FROM tStk_IODetail d \
                INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                WHERE io.POID = @p1 AND io.Kind IN ('RI', 'PD') AND io.State IN ('S', 'Y')";
    let row2_opt = conn.query(sql2, &[&poid]).await
        .map_err(|e| AppError::Internal(format!("查 PO 累计入库失败: {}", e)))?
        .into_row().await
        .ok()
        .flatten();
    let already_in: f64 = row2_opt
        .map(|r| r.get::<f64, _>("TotalIn").unwrap_or(0.0))
        .unwrap_or(0.0);
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
        return Err(AppError::BadRequest("该销售订单已作废，无法出库".to_string()));
    }
    let so_qty = row_get_f64(&row, "Q");
    // 2) 累计已出库数量（只算已审核/已确认的 SD/SR 净出库）
    let sql2 = "SELECT ISNULL(SUM(d.Qty), 0) AS TotalOut \
                FROM tStk_IODetail d \
                INNER JOIN tStk_IO io ON io.IOID = d.IOID \
                WHERE io.SOID = @p1 AND io.Kind IN ('SD', 'SI', 'POS') AND io.State IN ('S', 'Y')";
    let row2_opt = conn.query(sql2, &[&soid]).await
        .map_err(|e| AppError::Internal(format!("查 SO 累计出库失败: {}", e)))?
        .into_row().await
        .ok()
        .flatten();
    let already_out: f64 = row2_opt
        .map(|r| r.get::<f64, _>("TotalOut").unwrap_or(0.0))
        .unwrap_or(0.0);
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
        return Ok(Json(ApiResponse::err("Kind 必填（RI/TH/SD/SR/DB/OT/PD/POS/SI/PR/ZP）")));
    }
    let stk_id = json_str(d, "StkID");
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }
    let dt = now();
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let dea_uuid = empty_or_zero(&json_str(d, "DeaTypeID")).to_string();
    let po_uuid = empty_or_zero(&json_str(d, "POID")).to_string();
    let so_uuid = empty_or_zero(&json_str(d, "SOID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let us_uuid = empty_or_zero(&json_str(d, "USID")).to_string();
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let total_camt: f64 = params.details.iter().map(|x| json_f64(x, "CostAmt")).sum();
    let rsum_amt = json_f64(d, "RSumAmt");
    let disrate = json_f64(d, "DisRate");
    let downpay = json_f64(d, "DownPay");
    let term_day = json_i32(d, "TermDay");
    let curr = if json_str(d, "CurrCode").is_empty() { "CNY".to_string() } else { json_str(d, "CurrCode") };
    let remark = json_str(d, "Remark");
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // ===== P1-3 POID/SOID 上游单据校验 =====
    // 采购入库类（RI/PD）必须关联 PO
    if matches!(kind.as_str(), "RI" | "PD" | "TH" | "PR") {
        if let Err(e) = validate_upstream_po(&mut conn, &po_uuid, total_qty.abs()).await {
            return Ok(Json(ApiResponse::err(&e.to_string())));
        }
    }
    // 销售出库类（SD/SR/POS）必须关联 SO
    if matches!(kind.as_str(), "SD" | "SR" | "POS" | "SI") {
        if let Err(e) = validate_upstream_so(&mut conn, &so_uuid, total_qty.abs()).await {
            return Ok(Json(ApiResponse::err(&e.to_string())));
        }
    }

    let sql = "INSERT INTO tStk_IO (IONo, IoDate, Kind, StkID, SuppID, CustID, EmpID, DeptID, DeaTypeID, POID, SOID, BTPID, USID, \
        TermDay, CurrCode, DisRate, DownPay, SumAmt, SumQty, SumCAmt, RSumAmt, ScanMode, State, Note, EDate, EUser) \
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, 'N', @p22, @p23, @p24, @p25)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &io_no, &dt, &kind, &stk_id, &supp_id, &cust_id, &emp_uuid, &dept_uuid, &dea_uuid, &po_uuid, &so_uuid, &btp_uuid, &us_uuid,
        &term_day, &curr, &disrate, &downpay, &total_amt, &total_qty, &total_camt, &rsum_amt,
        &draft_state, &remark, &dt, &ZERO_UUID,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let ioid: String = {
        let q = "SELECT CAST(IOID AS NVARCHAR(40)) AS ID FROM tStk_IO WHERE IONo = @p1";
        match conn.query(q, &[&io_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
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
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    // ===== P1-4 创建时回填 StkQty/AQty 库存快照 =====
    // 让用户在草稿状态就能看到每个仓库的当前可用量
    crate::handlers::approval::fill_io_detail_stock_snapshot(&mut conn, &ioid).await;
    Ok(Json(ApiResponse::ok(serde_json::json!({ "IONo": io_no, "IOID": ioid }))))
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
    let stk_id = json_str(d, "StkID");
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");

    let upd = "UPDATE tStk_IO SET StkID=@p1, SumAmt=@p2, SumQty=@p3, Note=@p4, LUTime=GETDATE() WHERE IONo=@p5";
    let p: Vec<&dyn tiberius::ToSql> = vec![&stk_id, &total_amt, &total_qty, &remark, &io_no];
    conn.execute(upd, &p).await?;
    conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.ioid]).await?;
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
        conn.execute(ds, &dp).await?;
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tStk_Move WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (MoveNo LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page, page_size)))
}

#[derive(Deserialize)]
pub struct CreateMoveRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_move(
    State(_config): State<Config>,
    Json(params): Json<CreateMoveRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let move_no = json_str(d, "MoveNo");
    if move_no.is_empty() {
        return Ok(Json(ApiResponse::err("MoveNo 不能为空")));
    }
    let kind = json_str(d, "Kind");
    if kind.is_empty() { return Ok(Json(ApiResponse::err("Kind 必填（DB/TH/ZP）"))); }
    let from_stk = json_str(d, "FromStkID");
    let to_stk = json_str(d, "ToStkID");
    if from_stk.is_empty() || to_stk.is_empty() {
        return Ok(Json(ApiResponse::err("FromStkID / ToStkID 必填")));
    }
    let dt = now();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let remark = json_str(d, "Remark");
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    let sql = "INSERT INTO tStk_Move (MoveNo, MoveDate, Kind, FromStkID, ToStkID, EmpID, RSumAmt, ScanMode, State, EDate, EUser, LUTime) \
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, 'N', @p8, @p9, @p10, @p11)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &move_no, &dt, &kind, &from_stk, &to_stk, &emp_uuid, &total_amt,
        &draft_state, &dt, &ZERO_UUID, &dt,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let moveid: String = {
        let q = "SELECT CAST(MoveID AS NVARCHAR(40)) AS ID FROM tStk_Move WHERE MoveNo = @p1";
        match conn.query(q, &[&move_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
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
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    // P1-4 调拨创建时回填库存快照
    crate::handlers::approval::fill_move_detail_stock_snapshot(&mut conn, &moveid).await;
    Ok(Json(ApiResponse::ok(serde_json::json!({ "MoveNo": move_no, "MoveID": moveid }))))
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
    let move_no = json_str(d, "MoveNo");
    if move_no.is_empty() {
        return Ok(Json(ApiResponse::err("MoveNo 不能为空")));
    }
    let from_stk = json_str(d, "FromStkID");
    let to_stk = json_str(d, "ToStkID");
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tStk_Move SET FromStkID=@p1, ToStkID=@p2, RSumAmt=@p3, Note=@p4, LUTime=GETDATE() WHERE MoveNo=@p5";
    let p: Vec<&dyn tiberius::ToSql> = vec![&from_stk, &to_stk, &total_amt, &remark, &move_no];
    conn.execute(upd, &p).await?;
    conn.execute("DELETE FROM tStk_MoveDetail WHERE MoveID = @p1", &[&params.moveid]).await?;
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
        conn.execute(ds, &dp).await?;
    }
    // P1-4 调拨创建时回填库存快照
    crate::handlers::approval::fill_move_detail_stock_snapshot(&mut conn, &params.moveid).await;
    Ok(Json(ApiResponse::msg("调拨单更新成功")))
}

// ============== 盘点单（tStk_Tran + tStk_TranDetail）==============
pub async fn get_check_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tStk_Tran WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (TranNo LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page, page_size)))
}

#[derive(Deserialize)]
pub struct CreateCheckRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_check(
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
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let stk_id = json_str(d, "StkID");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // tStk_Tran.TranID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
    let sql = "INSERT INTO tStk_Tran (TranID, TranNo, TranDate, BTPID, StkID, EmpID, State, EDate, EUser, LUTime) \
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &tran_no, &dt, &btp, &stk_id, &emp_uuid,
        &draft_state, &dt, &ZERO_UUID, &dt,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let tranid: String = {
        let q = "SELECT CAST(TranID AS NVARCHAR(40)) AS ID FROM tStk_Tran WHERE TranNo = @p1";
        match conn.query(q, &[&tran_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
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
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    // P1-4 盘点创建时回填库存快照
    crate::handlers::approval::fill_tran_detail_stock_snapshot(&mut conn, &tranid).await;
    Ok(Json(ApiResponse::ok(serde_json::json!({ "TranNo": tran_no, "TranID": tranid }))))
}

// ============== 补货申请 ==============
pub async fn get_replenish_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tStk_ReplenishApply WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (ReplenishApplyNo LIKE @p1)");
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page, page_size)))
}

#[derive(Deserialize)]
pub struct CreateReplenishRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_replenish(
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
    let kind = if json_str(d, "Kind").is_empty() { "RP".to_string() } else { json_str(d, "Kind") };
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let end_date = json_str(d, "EndDate");
    let end_dt: chrono::NaiveDateTime = if end_date.is_empty() {
        now() + chrono::Duration::days(7)
    } else {
        chrono::NaiveDateTime::parse_from_str(&end_date, "%Y-%m-%d %H:%M:%S").unwrap_or_else(|_| now() + chrono::Duration::days(7))
    };
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    let sql = "INSERT INTO tStk_ReplenishApply (ReplenishApplyNo, ReplenishApplyDate, StkID, EndDate, Kind, EmpID, State, EDate, EUser) \
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &apply_no, &dt, &stk_id, &end_dt, &kind, &emp_uuid,
        &draft_state, &dt, &ZERO_UUID,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let apply_id: String = {
        let q = "SELECT CAST(ReplenishApplyID AS NVARCHAR(40)) AS ID FROM tStk_ReplenishApply WHERE ReplenishApplyNo = @p1";
        match conn.query(q, &[&apply_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "ApplyQty");
        let note = json_str(det, "ApplyNote");
        let apply_dt = now();
        let ds = "INSERT INTO tStk_ReplenishApplyDtl (ReplenishApplyID, ApplyDetailID, RowNO, GDSID, UnitNO, ApplyQty, ApplyNote, ApplyDate) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &apply_id, &row_no, &gdsid, &unit, &qty, &note, &apply_dt,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "ReplenishApplyNo": apply_no, "ReplenishApplyID": apply_id }))))
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
    pub QQty: f64,         // 当前在库可用量
    pub Qty: f64,          // 账面总库存
    pub BttomStkQty: f64,  // 安全库存下限
    pub TopStkQty: f64,    // 安全库存上限
    pub SuggestQty: f64,   // 建议补货量
    pub AlertLevel: String,// 严重等级: 紧急(QQty=0) / 警告(QQty<50%下限) / 提醒(QQty<下限)
}

#[derive(serde::Serialize)]
pub struct LowStockAlertResult {
    pub total: i32,
    pub critical: i32,    // QQty = 0
    pub warning: i32,     // QQty < 50% BttomStkQty
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
    let stk_id = params.get("stk_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let only_active = params.get("only_active").and_then(|v| v.as_bool()).unwrap_or(true);

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
    let rows = conn.query(sql, &[&stk_id_param, &active_filter]).await
        .map_err(|e| AppError::Internal(format!("查预警失败: {}", e)))?
        .into_first_result().await
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
        let qqty: f64 = r.get::<&str, _>("QQty").unwrap_or("0").parse().unwrap_or(0.0);
        let qty: f64 = r.get::<&str, _>("Q").unwrap_or("0").parse().unwrap_or(0.0);
        let bsq: f64 = r.get::<&str, _>("BSQ").unwrap_or("0").parse().unwrap_or(0.0);
        let tsq: f64 = r.get::<&str, _>("TSQ").unwrap_or("0").parse().unwrap_or(0.0);
        // 建议补货量：补到 TopStkQty（如有），否则补到 BttomStkQty*2
        let suggest = if tsq > 0.0 { (tsq - qqty).max(0.0) }
                      else if bsq > 0.0 { (bsq * 2.0 - qqty).max(0.0) }
                      else { 0.0 };
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
            GDSID: gdsid, GDSNO: gdsno, GDSDesc: gdsdesc,
            UnitNO: unitno, StkID: stkid, StkName: stkname,
            QQty: qqty, Qty: qty, BttomStkQty: bsq, TopStkQty: tsq,
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

// ============== 预警一键转补货申请 ==============
#[derive(Deserialize)]
pub struct ReplenishFromAlertRequest {
    /// 选中的预警项（不传则全部转）
    pub items: Option<Vec<AlertItem>>,
    /// 强制按仓库分组（每仓一张单）
    pub group_by_stk: Option<bool>,
}

#[derive(Deserialize, Clone)]
pub struct AlertItem {
    pub GDSID: String,
    pub StkID: String,
    /// 用户调整后的补货量（不传用 SuggestQty）
    pub ApplyQty: Option<f64>,
    pub UnitNO: Option<String>,
}

/// POST /api/inventory/replenish_from_alert
/// 接收预警选中的项，自动生成 tStk_ReplenishApply 草稿（State='N'）
/// 按 StkID 分组：每个仓库生成一张补货申请单
pub async fn replenish_from_alert(
    State(_config): State<Config>,
    Json(params): Json<ReplenishFromAlertRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 1) 没传 items → 自动拉所有预警
    let items: Vec<AlertItem> = if let Some(items) = params.items.clone() {
        items
    } else {
        let sql = r#"
            SELECT
                CAST(s.GDSID AS NVARCHAR(40)) AS GDSID,
                CAST(s.StkID AS NVARCHAR(40)) AS StkID,
                ISNULL(g.UnitNO,'') AS UnitNO,
                CASE WHEN ISNULL(g.TopStkQty, 0) > 0 THEN g.TopStkQty - s.QQty
                     WHEN ISNULL(g.BttomStkQty, 0) > 0 THEN g.BttomStkQty * 2 - s.QQty
                     ELSE 0 END AS SuggestQty
            FROM tStk_Stock s
            INNER JOIN tBas_Goods g ON g.GDSID = s.GDSID
            WHERE ISNULL(s.QQty, 0) < ISNULL(g.BttomStkQty, 0)
              AND ISNULL(g.BttomStkQty, 0) > 0
              AND g.GDSStateNO IN ('1','2','3')
        "#;
        let rows = conn.query(sql, &[]).await
            .map_err(|e| AppError::Internal(format!("查预警失败: {}", e)))?
            .into_first_result().await
            .unwrap_or_default();
        rows.iter().filter_map(|r| {
            let sq = r.get::<f64, _>("SuggestQty").unwrap_or(0.0);
            if sq <= 0.0 { return None; }
            Some(AlertItem {
                GDSID: r.get::<&str, _>("GDSID").unwrap_or("").to_string(),
                StkID: r.get::<&str, _>("StkID").unwrap_or("").to_string(),
                UnitNO: Some(r.get::<&str, _>("UnitNO").unwrap_or("").to_string()),
                ApplyQty: Some(sq),
            })
        }).collect()
    };
    if items.is_empty() {
        return Ok(Json(ApiResponse::err("无预警项需要转补货申请")));
    }
    // 过滤掉 GDSID/StkID 为空的脏数据
    let valid_items: Vec<AlertItem> = items.into_iter()
        .filter(|i| !i.GDSID.is_empty() && !i.StkID.is_empty())
        .collect();
    if valid_items.is_empty() {
        return Ok(Json(ApiResponse::err("所有项都缺少 GDSID/StkID，无法生成")));
    }

    // 2) 按 StkID 分组
    use std::collections::BTreeMap;
    let mut grouped: BTreeMap<String, Vec<AlertItem>> = BTreeMap::new();
    for item in valid_items {
        grouped.entry(item.StkID.clone()).or_insert_with(Vec::new).push(item);
    }

    // 3) 每个仓库生成一张 ReplenishApply 草稿
    let dt: chrono::NaiveDateTime = chrono::Local::now().naive_local();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;
    let mut created_docs: Vec<serde_json::Value> = Vec::new();

    for (stk_id, group_items) in grouped {
        let apply_no = format!("RPL{}", dt.format("%Y%m%d%H%M%S%3f"));
        let end_dt = dt + chrono::Duration::days(7);
        let sql = "INSERT INTO tStk_ReplenishApply (ReplenishApplyNo, ReplenishApplyDate, StkID, EndDate, Kind, State, EDate, EUser, Note) \
                   VALUES (@p1, @p2, @p3, @p4, 'RP', @p5, @p6, @p7, @p8)";
        let note_text = format!("从库存预警自动生成（{} 项）", group_items.len());
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &apply_no, &dt, &stk_id, &end_dt, &draft_state, &dt, &ZERO_UUID, &note_text,
        ];
        if let Err(e) = conn.execute(sql, &p).await {
            return Ok(Json(ApiResponse::err(&format!("保存补货单[{}]失败: {}", apply_no, e))));
        }
        // 抓取 ID
        let apply_id: String = {
            let q = "SELECT CAST(ReplenishApplyID AS NVARCHAR(40)) AS ID FROM tStk_ReplenishApply WHERE ReplenishApplyNo = @p1";
            match conn.query(q, &[&apply_no]).await {
                Ok(s) => match s.into_row().await {
                    Ok(Some(r)) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
                    _ => String::new(),
                },
                _ => String::new(),
            }
        };
        // 写明细
        let mut detail_count = 0;
        for (i, item) in group_items.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let unit = item.UnitNO.clone().unwrap_or_default();
            let qty = item.ApplyQty.unwrap_or(0.0);
            if qty <= 0.0 { continue; }
            let ds = "INSERT INTO tStk_ReplenishApplyDtl (ReplenishApplyID, ApplyDetailID, RowNO, GDSID, UnitNO, ApplyQty, ApplyNote, ApplyDate) \
                      VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7)";
            let note = "自动生成自库存预警".to_string();
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &apply_id, &row_no, &item.GDSID, &unit, &qty, &note, &dt,
            ];
            if conn.execute(ds, &dp).await.is_ok() {
                detail_count += 1;
            }
        }
        created_docs.push(serde_json::json!({
            "ReplenishApplyNo": apply_no,
            "ReplenishApplyID": apply_id,
            "StkID": stk_id,
            "DetailCount": detail_count,
            "State": "N",
        }));
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "CreatedCount": created_docs.len(),
        "Documents": created_docs,
    }))))
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
pub async fn inventory_adjust(
    State(_config): State<Config>,
    Json(params): Json<InventoryAdjustRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use crate::handlers::approval::{post_ledger, fill_detail_stock_snapshot};

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
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;
    let confirmed_state: &str = crate::handlers::doc_state::STATE_CONFIRMED;

    // 1) 写 tStk_IO 主表（State='N'）
    let sql = "INSERT INTO tStk_IO (IONo, IoDate, Kind, StkID, SumAmt, SumQty, SumCAmt, ScanMode, State, Note, EDate, EUser) \
               VALUES (@p1, @p2, @p3, @p4, 0, @p5, 0, 'N', @p6, @p7, @p8, @p9)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &io_no, &dt, &kind, &params.StkID, &params.Qty,
        &draft_state, &remark, &dt, &ZERO_UUID,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("写入调整单失败: {}", e))));
    }

    // 抓取 IOID
    let ioid: String = {
        let q = "SELECT CAST(IOID AS NVARCHAR(40)) AS ID FROM tStk_IO WHERE IONo = @p1";
        match conn.query(q, &[&io_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };

    // 2) 写 tStk_IODetail (一行)
    let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, Qty, CNVQty, StdQty, AccCheckFlg, Price, Amt) \
              VALUES (@p1, NEWID(), 1, @p2, @p3, @p4, @p4, @p4, 0, 0, 0)";
    let dp: Vec<&dyn tiberius::ToSql> = vec![&ioid, &params.GDSID, &params.StkID, &params.Qty];
    if let Err(e) = conn.execute(ds, &dp).await {
        return Ok(Json(ApiResponse::err(&format!("写入明细失败: {}", e))));
    }

    // 抓取 IODetailID
    let detail_id: String = {
        let q = "SELECT CAST(IODetailID AS NVARCHAR(40)) AS ID FROM tStk_IODetail WHERE IOID = @p1";
        match conn.query(q, &[&ioid]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };

    // 3) 调 post_ledger：方向由 Qty 符号决定（正=入库 +1，负=出库 -1）
    let direction: f64 = if params.Qty >= 0.0 { 1.0 } else { -1.0 };
    let abs_qty = params.Qty.abs();
    let (new_qty, ok) = post_ledger(
        &mut conn,
        &params.GDSID,
        &params.StkID,
        abs_qty,
        direction,
        &ioid,
        &detail_id,
    ).await;
    if !ok {
        return Ok(Json(ApiResponse::err(&format!("库存不足: 现有{} 需求{}", new_qty, abs_qty))));
    }

    // 4) 回填详情表 StkQty/AQty
    fill_detail_stock_snapshot(&mut conn, "tStk_IODetail", "IODetailID", &detail_id).await;

    // 5) 将 IO 单 State 置为 'Y' (已确认) —— 手工调整不需走审核
    let upd_state = "UPDATE tStk_IO SET State = @p1, AUser = @p2, ADate = @p3 WHERE IOID = @p4";
    let up: Vec<&dyn tiberius::ToSql> = vec![&confirmed_state, &ZERO_UUID, &dt, &ioid];
    let _ = conn.execute(upd_state, &up).await;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "IONo": io_no,
        "IOID": ioid,
        "Delta": params.Qty,
        "NewQty": new_qty,
    }))))
}

// ============== 月结（月末把上月 EndQty → 本月 InitQty）==============
#[derive(Deserialize)]
pub struct MonthSettleRequest {
    pub from_ym: i32,  // 来源月份 YYYYMM，如 202605
    pub to_ym: i32,    // 目标月份 YYYYMM，如 202606
}

/// POST /api/inventory/month_settle
/// 触发月结：把 from_ym 月份的 EndQty 复制为 to_ym 月份的 InitQty
/// DB 规则：月初把上月 EndQty 复制为 InitQty
pub async fn month_settle(
    State(_config): State<Config>,
    Json(params): Json<MonthSettleRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    use crate::handlers::approval::month_end_settle;
    let mut conn = get_pool().get().await?;

    // 简单校验
    if params.from_ym < 200001 || params.from_ym > 209912 {
        return Ok(Json(ApiResponse::err("from_ym 格式应为 YYYYMM（如 202605）")));
    }
    if params.to_ym < 200001 || params.to_ym > 209912 {
        return Ok(Json(ApiResponse::err("to_ym 格式应为 YYYYMM（如 202606）")));
    }
    if params.to_ym <= params.from_ym {
        return Ok(Json(ApiResponse::err("to_ym 必须大于 from_ym")));
    }

    let rows = month_end_settle(&mut conn, params.from_ym, params.to_ym).await;
    if rows < 0 {
        return Ok(Json(ApiResponse::err("月结执行失败，请检查数据库连接")));
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "from_ym": params.from_ym,
        "to_ym": params.to_ym,
        "settled_count": rows,
    }))))
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
    let master = match conn.query(master_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    // 查明细
    let det_sql = "SELECT * FROM tStk_IODetail WHERE IOID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn.query(det_sql, &[&params.id]).await?.into_first_result().await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(serde_json::json!({ "master": master, "details": details }))))
}

pub async fn get_move_detail(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let master_sql = "SELECT * FROM tStk_Move WHERE MoveID = @p1";
    let master = match conn.query(master_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    let det_sql = "SELECT * FROM tStk_MoveDetail WHERE MoveID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn.query(det_sql, &[&params.id]).await?.into_first_result().await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(serde_json::json!({ "master": master, "details": details }))))
}

pub async fn get_check_detail(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let master_sql = "SELECT * FROM tStk_Tran WHERE TranID = @p1";
    let master = match conn.query(master_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => row_to_json(&r),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    let det_sql = "SELECT * FROM tStk_TranDetail WHERE TranID = @p1 ORDER BY RowNO";
    let rows: Vec<Row> = conn.query(det_sql, &[&params.id]).await?.into_first_result().await?;
    let details: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
    Ok(Json(ApiResponse::ok(serde_json::json!({ "master": master, "details": details }))))
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
    let dt = if tran_date.is_empty() { now() } else {
        chrono::NaiveDateTime::parse_from_str(&tran_date, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&tran_date, "%Y-%m-%d"))
            .unwrap_or_else(|_| now())
    };
    let upd = "UPDATE tStk_Tran SET StkID=@p1, BTPID=@p2, EmpID=@p3, Note=@p4, TranDate=@p5, LUTime=GETDATE() WHERE TranID=@p6";
    let p: Vec<&dyn tiberius::ToSql> = vec![&stk_id, &btp, &emp_uuid, &note, &dt, &params.tranid];
    if let Err(e) = conn.execute(upd, &p).await {
        return Ok(Json(ApiResponse::err(&format!("更新主表失败: {}", e))));
    }
    conn.execute("DELETE FROM tStk_TranDetail WHERE TranID = @p1", &[&params.tranid]).await?;
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
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::msg("盘点单更新成功")))
}

// ============== 删除单据（仅草稿状态）==============
pub async fn delete_io(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // 检查状态
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_IO WHERE IOID = @p1";
    let state = match conn.query(state_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if state != "D" && state != "N" {
        return Ok(Json(ApiResponse::err("只有草稿/新建状态的单据才能删除")));
    }
    conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.id]).await?;
    conn.execute("DELETE FROM tStk_IO WHERE IOID = @p1", &[&params.id]).await?;
    Ok(Json(ApiResponse::msg("入出库单已删除")))
}

pub async fn delete_move(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Move WHERE MoveID = @p1";
    let state = match conn.query(state_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if state != "D" && state != "N" {
        return Ok(Json(ApiResponse::err("只有草稿/新建状态的单据才能删除")));
    }
    conn.execute("DELETE FROM tStk_MoveDetail WHERE MoveID = @p1", &[&params.id]).await?;
    conn.execute("DELETE FROM tStk_Move WHERE MoveID = @p1", &[&params.id]).await?;
    Ok(Json(ApiResponse::msg("调拨单已删除")))
}

pub async fn delete_check(
    State(_config): State<Config>,
    Json(params): Json<DetailParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let state_sql = "SELECT ISNULL(State,'') AS S FROM tStk_Tran WHERE TranID = @p1";
    let state = match conn.query(state_sql, &[&params.id]).await?.into_row().await? {
        Some(r) => r.get::<&str, _>("S").unwrap_or("").to_string(),
        None => return Ok(Json(ApiResponse::err("单据不存在"))),
    };
    if state != "D" && state != "N" {
        return Ok(Json(ApiResponse::err("只有草稿/新建状态的单据才能删除")));
    }
    conn.execute("DELETE FROM tStk_TranDetail WHERE TranID = @p1", &[&params.id]).await?;
    conn.execute("DELETE FROM tStk_Tran WHERE TranID = @p1", &[&params.id]).await?;
    Ok(Json(ApiResponse::msg("盘点单已删除")))
}

// ============== 库存流水（tStk_StockTranHis）==============
pub async fn get_stock_flow(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT h.*, g.[GDSNO], g.[GDSDesc], g.[GDSSpec], sk.[StkName] \
                          FROM [tStk_StockTranHis] h \
                          LEFT JOIN [tBas_Goods] g ON h.[GDSID] = g.[GDSID] \
                          LEFT JOIN [tBas_Stock] sk ON h.[StkID] = sk.[StkID] \
                          WHERE 1=1".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSNO] LIKE @p{} OR g.[GDSDesc] LIKE @p{} OR CAST(h.[TranID] AS NVARCHAR(40)) LIKE @p{})", pidx, pidx+1, pidx+2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page, page_size)))
}
