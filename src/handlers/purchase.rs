use axum::{extract::State, Json};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use super::base_data::try_get_value;

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

// ============== 采购订单 ==============
pub async fn get_purchase_orders(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tPur_Order WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PoNo LIKE @p1)");
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
pub struct CreateOrderRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_purchase_order(
    State(_config): State<Config>,
    Json(params): Json<CreateOrderRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let po_no = json_str(d, "PoNo");
    if po_no.is_empty() {
        return Ok(Json(ApiResponse::err("PoNo 不能为空")));
    }
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let stk_id = empty_or_zero(&json_str(d, "StkID")).to_string();
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let disrate = json_f64(d, "DisRate");
    let curr = if json_str(d, "CurrCode").is_empty() { "CNY".to_string() } else { json_str(d, "CurrCode") };
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // 注意：tPur_Order 实际字段：POID, BTPID, SuppID, DeptID, EmpID, PoNo, PoDate, EndDate, CurrCode, StkID, DisRate, SumAmt, Note, State, EDate, EUser
    let sql = "INSERT INTO tPur_Order (PoNo, PoDate, StkID, SuppID, EmpID, DeptID, BTPID, DisRate, CurrCode, SumAmt, State, EDate, EUser, Note) \
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &po_no, &dt, &stk_id, &supp_id, &emp_uuid, &dept_uuid, &btp_uuid,
        &disrate, &curr, &total_amt,
        &draft_state, &dt, &ZERO_UUID, &remark,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let poid: String = {
        let q = "SELECT CAST(POID AS NVARCHAR(40)) AS ID FROM tPur_Order WHERE PoNo = @p1";
        match conn.query(q, &[&po_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let stk_id_d = empty_or_zero(&json_str(det, "StkID")).to_string();
        let ds = "INSERT INTO tPur_OrderDetail (POID, PODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &poid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "PoNo": po_no, "POID": poid }))))
}

#[derive(Deserialize)]
pub struct UpdateOrderRequest {
    pub poid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_purchase_order(
    State(_config): State<Config>,
    Json(params): Json<UpdateOrderRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let po_no = json_str(d, "PoNo");
    if po_no.is_empty() {
        return Ok(Json(ApiResponse::err("PoNo 不能为空")));
    }
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tPur_Order SET SumAmt=@p1, Note=@p2, LUTime=GETDATE() WHERE PoNo=@p3";
    let p: Vec<&dyn tiberius::ToSql> = vec![&total_amt, &remark, &po_no];
    conn.execute(upd, &p).await?;
    conn.execute("DELETE FROM tPur_OrderDetail WHERE POID = @p1", &[&params.poid]).await?;
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let stk_id_d = empty_or_zero(&json_str(det, "StkID")).to_string();
        let ds = "INSERT INTO tPur_OrderDetail (POID, PODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &params.poid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
        ];
        conn.execute(ds, &dp).await?;
    }
    Ok(Json(ApiResponse::msg("采购订单更新成功")))
}

// ============== 采购入库（tStk_IO, Kind='RI'，tPur_Inv 表不存在）==============
pub async fn get_purchase_inbound(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tStk_IO WHERE Kind='RI' AND State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1)");
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
pub struct CreateInboundRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_purchase_inbound(
    State(_config): State<Config>,
    Json(params): Json<CreateInboundRequest>,
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
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let stk_id = json_str(d, "StkID");
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let po_uuid = empty_or_zero(&json_str(d, "POID")).to_string();
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let disrate = json_f64(d, "DisRate");
    let curr = if json_str(d, "CurrCode").is_empty() { "CNY".to_string() } else { json_str(d, "CurrCode") };
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // tPur_Inv 表不存在，统一写入 tStk_IO，Kind='RI'
    let sql = "INSERT INTO tStk_IO (IONo, IoDate, Kind, StkID, SuppID, EmpID, DeptID, BTPID, POID, DisRate, CurrCode, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
        VALUES (@p1, @p2, 'RI', @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 'N', @p13, @p14, @p15, @p16)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &io_no, &dt, &stk_id, &supp_id, &emp_uuid, &dept_uuid, &btp_uuid, &po_uuid,
        &disrate, &curr, &total_amt, &total_qty,
        &draft_state, &dt, &ZERO_UUID, &remark,
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
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let cnq = qty;
        let stdq = qty;
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, Price, Amt, AccCheckFlg) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 0)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &ioid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &cnq, &stdq, &price, &amt,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "IONo": io_no, "IOID": ioid }))))
}

// ============== 采购退货（tStk_IO, Kind='TH'，tPur_Return 表不存在）==============
pub async fn get_purchase_return(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tStk_IO WHERE Kind='TH' AND State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1)");
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
pub struct CreateReturnRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_purchase_return(
    State(_config): State<Config>,
    Json(params): Json<CreateReturnRequest>,
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
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let stk_id = json_str(d, "StkID");
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let po_uuid = empty_or_zero(&json_str(d, "POID")).to_string();
    let total_amt: f64 = params.details.iter().map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price"))).sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // ===== P2-2 采购退货 POID 上下游校验 =====
    // 业务规则：累计退货(TH) 不能超过 累计入库(RI/PD)；
    //          已作废的 PO 禁止退货
    if po_uuid != ZERO_UUID {
        // 1) PO 存在性 + 状态
        let po_row = match conn.query(
            "SELECT CAST(POID AS NVARCHAR(40)) AS ID, ISNULL(SumQty, 0) AS Q, State \
             FROM tPur_Order WHERE POID = @p1",
            &[&po_uuid],
        ).await {
            Ok(s) => match s.into_row().await {
                Ok(r) => r,
                Err(_) => None,
            },
            Err(_) => None,
        };

        let r = match po_row {
            Some(r) => r,
            None => return Ok(Json(ApiResponse::err("采购订单不存在"))),
        };
        let state = r.get::<&str, _>("State").unwrap_or("").to_string();
        if state == "D" || state == "C" {
            return Ok(Json(ApiResponse::err("该采购订单已作废，无法退货")));
        }
        let po_qty: f64 = r.get::<f64, _>("Q").unwrap_or(0.0);

        // 2) 累计已入库 RI/PD（已审核）
        let mut already_in: f64 = 0.0;
        let in_row = match conn.query(
            "SELECT ISNULL(SUM(d.Qty), 0) AS TotalIn \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.POID = @p1 AND io.Kind IN ('RI','PD') AND io.State IN ('S','Y')",
            &[&po_uuid],
        ).await {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = in_row {
            already_in = r.get::<f64, _>("TotalIn").unwrap_or(0.0);
        }

        // 3) 累计已退货 TH（已审核）
        let mut already_ret: f64 = 0.0;
        let ret_row = match conn.query(
            "SELECT ISNULL(SUM(d.Qty), 0) AS TotalRet \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.POID = @p1 AND io.Kind IN ('TH','PR') AND io.State IN ('S','Y')",
            &[&po_uuid],
        ).await {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = ret_row {
            already_ret = r.get::<f64, _>("TotalRet").unwrap_or(0.0);
        }

        // 4) 本次退货不能超过已入库 - 已退货
        if total_qty.abs() > already_in - already_ret + 0.0001 {
            return Ok(Json(ApiResponse::err(&format!(
                "超量退货：PO数量={} 已入库={} 已退货={} 本次退货={}",
                po_qty, already_in, already_ret, total_qty.abs()
            ))));
        }
    }

    let sql = "INSERT INTO tStk_IO (IONo, IoDate, Kind, StkID, SuppID, EmpID, POID, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
        VALUES (@p1, @p2, 'TH', @p3, @p4, @p5, @p6, @p7, @p8, 'N', @p9, @p10, @p11, @p12)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &io_no, &dt, &stk_id, &supp_id, &emp_uuid, &po_uuid, &total_amt, &total_qty,
        &draft_state, &dt, &ZERO_UUID, &remark,
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
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, Price, Amt, AccCheckFlg) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p8, @p8, @p9, @p10, 0)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &ioid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "IONo": io_no, "IOID": ioid }))))
}

// ============== 采购报价 ==============
pub async fn get_purchase_quotes(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tPur_Quote WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PqNo LIKE @p1)");
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
pub struct CreateQuoteRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_purchase_quote(
    State(_config): State<Config>,
    Json(params): Json<CreateQuoteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let pq_no = json_str(d, "PqNo");
    if pq_no.is_empty() {
        return Ok(Json(ApiResponse::err("PqNo 不能为空")));
    }
    let supp_id = empty_or_zero(&json_str(d, "SuppID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let remark = json_str(d, "Remark");
    let dt = now();
    let end_dt = dt + chrono::Duration::days(30);
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // tPur_Quote 实际字段：PQID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
    let sql = "INSERT INTO tPur_Quote (PQID, PqNo, PqDate, StartDate, EndDate, SuppID, EmpID, BTPID, State, EDate, EUser, Note) \
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &pq_no, &dt, &dt, &end_dt, &supp_id, &emp_uuid, &btp_uuid,
        &draft_state, &dt, &ZERO_UUID, &remark,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let pqid: String = {
        let q = "SELECT CAST(PQID AS NVARCHAR(40)) AS ID FROM tPur_Quote WHERE PqNo = @p1";
        match conn.query(q, &[&pq_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let stk_id_d = empty_or_zero(&json_str(det, "StkID")).to_string();
        let ds = "INSERT INTO tPur_QuoteDetail (PQID, PQDetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &pqid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "PqNo": pq_no, "PQID": pqid }))))
}

#[derive(Deserialize)]
pub struct UpdateQuoteRequest {
    pub pqid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_purchase_quote(
    State(_config): State<Config>,
    Json(params): Json<UpdateQuoteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let pq_no = json_str(d, "PqNo");
    if pq_no.is_empty() {
        return Ok(Json(ApiResponse::err("PqNo 不能为空")));
    }
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tPur_Quote SET Note=@p1, LUTime=GETDATE() WHERE PqNo=@p2";
    let p: Vec<&dyn tiberius::ToSql> = vec![&remark, &pq_no];
    conn.execute(upd, &p).await?;
    conn.execute("DELETE FROM tPur_QuoteDetail WHERE PQID = @p1", &[&params.pqid]).await?;
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        let qty = json_f64(det, "Qty");
        let price = json_f64(det, "Price");
        let amt = json_f64(det, "Amt");
        let stk_id_d = empty_or_zero(&json_str(det, "StkID")).to_string();
        let ds = "INSERT INTO tPur_QuoteDetail (PQID, PQDetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &params.pqid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
        ];
        conn.execute(ds, &dp).await?;
    }
    Ok(Json(ApiResponse::msg("采购报价更新成功")))
}

// ============== 采购调价（实际表 tPur_AdjPrice，主键 PAPID，编号 PAPNo）==============
pub async fn get_purchase_adjprice(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let mut base_query = "SELECT * FROM tPur_AdjPrice WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PAPNo LIKE @p1)");
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
pub struct CreateAdjpriceRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_purchase_adjprice(
    State(_config): State<Config>,
    Json(params): Json<CreateAdjpriceRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let pap_no = json_str(d, "PAPNo");
    if pap_no.is_empty() {
        return Ok(Json(ApiResponse::err("PAPNo 不能为空")));
    }
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_DRAFT;

    // tPur_AdjPrice 实际字段：PAPID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
    let sql = "INSERT INTO tPur_AdjPrice (PAPID, PAPNo, PAPDate, EmpID, DeptID, BTPID, State, EDate, EUser, Note) \
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
    let p: Vec<&dyn tiberius::ToSql> = vec![
        &pap_no, &dt, &emp_uuid, &dept_uuid, &btp_uuid,
        &draft_state, &dt, &ZERO_UUID, &remark,
    ];
    if let Err(e) = conn.execute(sql, &p).await {
        return Ok(Json(ApiResponse::err(&format!("保存主表失败: {}", e))));
    }
    let paid: String = {
        let q = "SELECT CAST(PAPID AS NVARCHAR(40)) AS ID FROM tPur_AdjPrice WHERE PAPNo = @p1";
        match conn.query(q, &[&pap_no]).await?.into_row().await? {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => String::new(),
        }
    };
    for (i, det) in params.details.iter().enumerate() {
        let row_no = (i + 1) as i32;
        let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
        let gds_no = json_str(det, "GDSNO");
        let gds_desc = json_str(det, "GDSDesc");
        let unit = json_str(det, "UnitNO");
        // tPur_AdjPriceDetail 用 OldAInPrice/NewAInPrice 字段（其它还有 OldBPrice/NewBPrice 等）
        let old_price = json_f64(det, "OldAInPrice");
        let new_price = json_f64(det, "NewAInPrice");
        let note = json_str(det, "Note");
        let ds = "INSERT INTO tPur_AdjPriceDetail (PAPID, PAPDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, OldAInPrice, NewAInPrice, Note) \
            VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
        let dp: Vec<&dyn tiberius::ToSql> = vec![
            &paid, &row_no, &gdsid, &gds_no, &gds_desc, &unit, &old_price, &new_price, &note,
        ];
        if let Err(e) = conn.execute(ds, &dp).await {
            return Ok(Json(ApiResponse::err(&format!("保存明细(行{})失败: {}", i + 1, e))));
        }
    }
    Ok(Json(ApiResponse::ok(serde_json::json!({ "PAPNo": pap_no, "PAPID": paid }))))
}

// ============== 采购综合查询 ==============
pub async fn get_purchase_query(
    State(_config): State<Config>,
    Json(params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let doc_type = params.get("doc_type").and_then(|v| v.as_str()).unwrap_or("order");
    let keyword = params.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
    let page = params.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
    let page_size = params.get("page_size").and_then(|v| v.as_u64()).unwrap_or(20);
    let (table, no_col) = match doc_type {
        "order" => ("tPur_Order", "PoNo"),
        "inbound" => ("tStk_IO", "IONo"),
        "return" => ("tStk_IO", "IONo"),
        "quote" => ("tPur_Quote", "PqNo"),
        "adjprice" => ("tPur_AdjPrice", "PAPNo"),
        _ => ("tPur_Order", "PoNo"),
    };
    let kind_filter = match doc_type {
        "inbound" => " AND Kind='RI'",
        "return" => " AND Kind='TH'",
        _ => "",
    };
    let base = format!("SELECT * FROM {} WHERE State <> 'D'{}", table, kind_filter);
    let mut base_q = base;
    if !keyword.is_empty() {
        base_q.push_str(&format!(" AND ({} LIKE @p1)", no_col));
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_q);
    let paginated_sql = build_pagination_sql_with_sort(&base_q, page as u32, page_size as u32,
        params.get("sort_prop").and_then(|v| v.as_str()),
        params.get("sort_order").and_then(|v| v.as_str()));
    let query_params: Vec<Option<String>> = if !keyword.is_empty() {
        vec![Some(format!("%{}%", keyword))]
    } else {
        vec![]
    };
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let mut total: i32 = 0;
    if let Some(row) = conn.query(&count_sql, &param_refs).await?.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }
    let rows: Vec<Row> = conn.query(&paginated_sql, &param_refs).await?.into_first_result().await?;
    Ok(Json(ApiResponse::ok_paginated(rows.iter().map(row_to_json).collect(), total as u64, page as u32, page_size as u32)))
}
