use super::base_data::row_to_json;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::services::inventory_ledger;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use axum::{Json, extract::State};
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

fn json_i32(v: &serde_json::Value, key: &str) -> i32 {
    v.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32
}

fn empty_or_zero(s: &str) -> &str {
    if s.is_empty() { ZERO_UUID } else { s }
}

fn now() -> chrono::NaiveDateTime {
    chrono::Local::now().naive_local()
}

// ============== 销售订单 ==============
pub async fn get_sales_orders(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tSal_Order WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (SoNo LIKE @p1 OR CustName LIKE @p2)");
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

#[derive(Deserialize)]
pub struct CreateOrderRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_sales_order(
    State(_config): State<Config>,
    Json(params): Json<CreateOrderRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let so_no = json_str(d, "SoNo");
    if so_no.is_empty() {
        return Ok(Json(ApiResponse::err("SoNo 不能为空")));
    }
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    let stk_id = empty_or_zero(&json_str(d, "StkID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let sq_uuid = empty_or_zero(&json_str(d, "SQID")).to_string();
    let disrate = json_f64(d, "DisRate");
    let downpay = json_f64(d, "DownPay");
    let curr = if json_str(d, "CurrCode").is_empty() {
        "CNY".to_string()
    } else {
        json_str(d, "CurrCode")
    };
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚，避免主表残留无明细的脏数据
    let mut soid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tSal_Order 实际字段（无 CustName/DeaTypeID/TermDay/SumAmt/SumQty）
        // SOID 列 NOT NULL 且无默认值（数据库查询确认 default=NULL），必须显式 NEWID()
        // 使用 OUTPUT 子句直接获取插入的 SOID，避免 SELECT by SoNo 在重复单号场景下错配
        let sql = "INSERT INTO tSal_Order (SOID, SoNo, SoDate, CustID, StkID, EmpID, DeptID, BTPID, SQID, DisRate, DownPay, CurrCode, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.SOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &so_no, &dt, &cust_id, &stk_id, &emp_uuid, &dept_uuid, &btp_uuid, &sq_uuid,
            &disrate, &downpay, &curr,
            &draft_state, &dt, &ZERO_UUID, &remark,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let soid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 SOID".to_string()),
        };
        if soid.is_empty() {
            return Err("无法获取主表 SOID".to_string());
        }
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
            let sq_id = empty_or_zero(&json_str(det, "SQID")).to_string();
            let sq_detail_id = empty_or_zero(&json_str(det, "SQDetailID")).to_string();
            let barcode = json_str(det, "BarCode");
            let cnv_qty = json_f64(det, "CNVQty");
            let std_qty = json_f64(det, "StdQty");
            let c_price = json_f64(det, "CPrice");
            let ain_price = json_f64(det, "AInPrice");
            let dis_rate = json_f64(det, "DisRate");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let note = json_str(det, "Note");
            let a_price = json_f64(det, "APrice");
            let ds = "INSERT INTO tSal_OrderDetail (SOID, SODetailID, RowNO, GDSID, UnitNO, StkID, SQID, SQDetailID, GDSNO, GDSDesc, \
                BarCode, Qty, CNVQty, StdQty, CPrice, AInPrice, Price, DisRate, Amt, TaxRate, TaxAmt, Note, APrice) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, @p22)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &soid, &row_no, &gdsid, &unit, &stk_id_d, &sq_id, &sq_detail_id, &gds_no, &gds_desc,
                &barcode, &qty, &cnv_qty, &std_qty, &c_price, &ain_price, &price, &dis_rate, &amt,
                &tax_rate, &tax_amt, &note, &a_price,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        soid_out = soid;
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售订单保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "SoNo": so_no, "SOID": soid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateOrderRequest {
    pub soid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_sales_order(
    State(_config): State<Config>,
    Json(params): Json<UpdateOrderRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tSal_Order WHERE SOID=@p1",
                &[&params.soid],
            )
            .await?;
        if let Some(row) = state_check.into_row().await? {
            let state: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            if !crate::handlers::doc_state::is_editable(&state) {
                let msg = format!(
                    "单据已{}，不可编辑，请先反审",
                    crate::handlers::doc_state::label(&state)
                );
                return Ok(Json(ApiResponse::err(&msg)));
            }
        }
    }
    let d = &params.data;
    let so_no = json_str(d, "SoNo");
    if so_no.is_empty() {
        return Ok(Json(ApiResponse::err("SoNo 不能为空")));
    }
    let disrate = json_f64(d, "DisRate");
    let downpay = json_f64(d, "DownPay");
    let remark = json_str(d, "Remark");
    let upd =
        "UPDATE tSal_Order SET DisRate=@p1, DownPay=@p2, Note=@p3, LUTime=GETDATE() WHERE SoNo=@p4";
    let p: Vec<&dyn tiberius::ToSql> = vec![&disrate, &downpay, &remark, &so_no];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tSal_OrderDetail WHERE SOID = @p1", &[&params.soid]).await.map_err(|e| e.to_string())?;
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
            let sq_id = empty_or_zero(&json_str(det, "SQID")).to_string();
            let sq_detail_id = empty_or_zero(&json_str(det, "SQDetailID")).to_string();
            let barcode = json_str(det, "BarCode");
            let cnv_qty = json_f64(det, "CNVQty");
            let std_qty = json_f64(det, "StdQty");
            let c_price = json_f64(det, "CPrice");
            let ain_price = json_f64(det, "AInPrice");
            let dis_rate = json_f64(det, "DisRate");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let note = json_str(det, "Note");
            let a_price = json_f64(det, "APrice");
            let ds = "INSERT INTO tSal_OrderDetail (SOID, SODetailID, RowNO, GDSID, UnitNO, StkID, SQID, SQDetailID, GDSNO, GDSDesc, \
                BarCode, Qty, CNVQty, StdQty, CPrice, AInPrice, Price, DisRate, Amt, TaxRate, TaxAmt, Note, APrice) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, @p22)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.soid, &row_no, &gdsid, &unit, &stk_id_d, &sq_id, &sq_detail_id, &gds_no, &gds_desc,
                &barcode, &qty, &cnv_qty, &std_qty, &c_price, &ain_price, &price, &dis_rate, &amt,
                &tax_rate, &tax_amt, &note, &a_price,
            ];
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售订单更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("销售订单更新成功")))
}

// ============== 销售出库 ==============
pub async fn get_sales_outbound(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_IO WHERE State <> 'D' AND Kind = 'SD'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1 OR CustName LIKE @p2)");
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

#[derive(Deserialize)]
pub struct CreateOutboundRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_sales_outbound(
    State(_config): State<Config>,
    Json(params): Json<CreateOutboundRequest>,
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
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    let stk_id = json_str(d, "StkID");
    if stk_id.is_empty() {
        return Ok(Json(ApiResponse::err("StkID 不能为空")));
    }
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let so_uuid = empty_or_zero(&json_str(d, "SOID")).to_string();
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let disrate = json_f64(d, "DisRate");
    let curr = if json_str(d, "CurrCode").is_empty() {
        "CNY".to_string()
    } else {
        json_str(d, "CurrCode")
    };
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚，避免主表残留无明细的脏数据
    // 销售出库 = 写入 tStk_IO (Kind='SD') + tStk_IODetail
    // 库存减少在 /api/doc/approve 审核时统一写入三件套
    let mut ioid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, CustID, EmpID, DeptID, BTPID, SOID, DisRate, CurrCode, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, 'SD', @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 'N', @p13, @p14, @p15, @p16)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &stk_id, &cust_id, &emp_uuid, &dept_uuid, &btp_uuid, &so_uuid,
            &disrate, &curr, &total_amt, &total_qty,
            &draft_state, &dt, &ZERO_UUID, &remark,
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
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let aprice = json_f64(det, "APrice");
            let cprice = json_f64(det, "CPrice");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let dis_rate = json_f64(det, "DisRate");
            let note = json_str(det, "Note");
            let barcode = json_str(det, "BarCode");
            let sou_id = empty_or_zero(&json_str(det, "SouID")).to_string();
            let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, \
                Price, Amt, AccCheckFlg, APrice, CPrice, TaxRate, TaxAmt, DisRate, Note, BarCode, SouID) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p8, @p8, @p9, @p10, 0, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &ioid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
                &aprice, &cprice, &tax_rate, &tax_amt, &dis_rate, &note, &barcode, &sou_id,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        ioid_out = ioid;
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售出库保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "IONo": io_no, "IOID": ioid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateOutboundRequest {
    pub soid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_sales_outbound(
    State(_config): State<Config>,
    Json(params): Json<UpdateOutboundRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tStk_IO WHERE IOID=@p1 AND Kind='SD'",
                &[&params.soid],
            )
            .await?;
        if let Some(row) = state_check.into_row().await? {
            let state: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            if !crate::handlers::doc_state::is_editable(&state) {
                let msg = format!(
                    "单据已{}，不可编辑，请先反审",
                    crate::handlers::doc_state::label(&state)
                );
                return Ok(Json(ApiResponse::err(&msg)));
            }
        }
    }
    let d = &params.data;
    let io_no = json_str(d, "IONo");
    if io_no.is_empty() {
        return Ok(Json(ApiResponse::err("IONo 不能为空")));
    }
    let stk_id = json_str(d, "StkID");
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");
    // 销售出库走 tStk_IO Kind='SD'（tSal_Inv 字段过简，无法保留业务字段）
    let upd = "UPDATE tStk_IO SET StkID=@p1, CustID=@p2, SumAmt=@p3, SumQty=@p4, Note=@p5, LUTime=GETDATE() WHERE IONo=@p6 AND Kind='SD'";
    let p: Vec<&dyn tiberius::ToSql> =
        vec![&stk_id, &cust_id, &total_amt, &total_qty, &remark, &io_no];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.soid]).await.map_err(|e| e.to_string())?;
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let aprice = json_f64(det, "APrice");
            let cprice = json_f64(det, "CPrice");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let dis_rate = json_f64(det, "DisRate");
            let note = json_str(det, "Note");
            let barcode = json_str(det, "BarCode");
            let sou_id = empty_or_zero(&json_str(det, "SouID")).to_string();
            let ds = "INSERT INTO tStk_IODetail (IOID, IODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, CNVQty, StdQty, \
                Price, Amt, AccCheckFlg, APrice, CPrice, TaxRate, TaxAmt, DisRate, Note, BarCode, SouID) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p8, @p8, @p9, @p10, 0, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.soid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
                &aprice, &cprice, &tax_rate, &tax_amt, &dis_rate, &note, &barcode, &sou_id,
            ];
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售出库更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("销售出库更新成功")))
}

// 销售退货函数已迁移到 sales_return.rs（list_sales_return/create_sales_return/update_sales_return）

// ============== 销售报价 ==============
pub async fn get_sales_quotes(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tSal_Quote WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (SqNo LIKE @p1 OR CustName LIKE @p2)");
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

#[derive(Deserialize)]
pub struct CreateQuoteRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_sales_quote(
    State(_config): State<Config>,
    Json(params): Json<CreateQuoteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let sq_no = json_str(d, "SQNo");
    if sq_no.is_empty() {
        return Ok(Json(ApiResponse::err("SQNo 不能为空")));
    }
    let cust_id = empty_or_zero(&json_str(d, "CustID")).to_string();
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let stk_uuid = empty_or_zero(&json_str(d, "StkID")).to_string();
    let active_days = json_i32(d, "ActiveDays");
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚，避免主表残留无明细的脏数据
    let mut sqid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tSal_Quote 实际字段：SQID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
        let sql = "INSERT INTO tSal_Quote (SQID, SQNo, SQDate, CustID, EmpID, DeptID, BTPID, StkID, ActiveDays, State, EDate, EUser, Note) \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12); \
            SELECT CAST(SQID AS NVARCHAR(40)) AS ID FROM tSal_Quote WHERE SQNo = @p13";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &sq_no, &dt, &cust_id, &emp_uuid, &dept_uuid, &btp_uuid, &stk_uuid, &active_days,
            &draft_state, &dt, &ZERO_UUID, &remark, &sq_no,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let sqid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 SQID".to_string()),
        };
        if sqid.is_empty() {
            return Err("无法获取主表 SQID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tSal_QuoteDetail (SQID, SQDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &sqid, &row_no, &gdsid, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        sqid_out = sqid;
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售报价保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "SQNo": sq_no, "SQID": sqid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateQuoteRequest {
    pub sqid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_sales_quote(
    State(_config): State<Config>,
    Json(params): Json<UpdateQuoteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tSal_Quote WHERE SQID=@p1",
                &[&params.sqid],
            )
            .await?;
        if let Some(row) = state_check.into_row().await? {
            let state: String = row.get::<&str, _>(0).unwrap_or("").to_string();
            if !crate::handlers::doc_state::is_editable(&state) {
                let msg = format!(
                    "单据已{}，不可编辑，请先反审",
                    crate::handlers::doc_state::label(&state)
                );
                return Ok(Json(ApiResponse::err(&msg)));
            }
        }
    }
    let d = &params.data;
    let sq_no = json_str(d, "SQNo");
    if sq_no.is_empty() {
        return Ok(Json(ApiResponse::err("SQNo 不能为空")));
    }
    let active_days = json_i32(d, "ActiveDays");
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tSal_Quote SET ActiveDays=@p1, Note=@p2, LUTime=GETDATE() WHERE SQID=@p3";
    let p: Vec<&dyn tiberius::ToSql> = vec![&active_days, &remark, &params.sqid];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tSal_QuoteDetail WHERE SQID = @p1", &[&params.sqid]).await.map_err(|e| e.to_string())?;
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let qty = json_f64(det, "Qty");
            let price = json_f64(det, "Price");
            let amt = json_f64(det, "Amt");
            let ds = "INSERT INTO tSal_QuoteDetail (SQID, SQDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.sqid, &row_no, &gdsid, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售报价更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("销售报价更新成功")))
}

// ============== 销售调价 ==============
pub async fn get_sales_adjprice(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tSal_AdjPrice WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (SAPNo LIKE @p1)");
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
pub struct CreateAdjpriceRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_sales_adjprice(
    State(_config): State<Config>,
    Json(params): Json<CreateAdjpriceRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.details.is_empty() {
        return Ok(Json(ApiResponse::err("明细不能为空")));
    }
    let mut conn = get_pool().get().await?;
    let d = &params.data;
    let sap_no = json_str(d, "SAPNo");
    if sap_no.is_empty() {
        return Ok(Json(ApiResponse::err("SAPNo 不能为空")));
    }
    let emp_uuid = empty_or_zero(&json_str(d, "EmpID")).to_string();
    let dept_uuid = empty_or_zero(&json_str(d, "DeptID")).to_string();
    let btp_uuid = empty_or_zero(&json_str(d, "BTPID")).to_string();
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚，避免主表残留无明细的脏数据
    let mut sapid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tSal_AdjPrice 实际字段：SAPID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
        let sql = "INSERT INTO tSal_AdjPrice (SAPID, SAPNo, SAPDate, EmpID, DeptID, BTPID, State, EDate, EUser, Note) \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9); \
            SELECT CAST(SAPID AS NVARCHAR(40)) AS ID FROM tSal_AdjPrice WHERE SAPNo = @p10";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &sap_no, &dt, &emp_uuid, &dept_uuid, &btp_uuid,
            &draft_state, &dt, &ZERO_UUID, &remark, &sap_no,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let sapid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 SAPID".to_string()),
        };
        if sapid.is_empty() {
            return Err("无法获取主表 SAPID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let old_price = json_f64(det, "OldPrice");
            let new_price = json_f64(det, "NewPrice");
            let note = json_str(det, "Note");
            let ds = "INSERT INTO tSal_AdjPriceDetail (SAPID, SAPDetailID, RowNO, GDSID, GDSNO, GDSDesc, UnitNO, OldPrice, NewPrice, Note) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &sapid, &row_no, &gdsid, &gds_no, &gds_desc, &unit, &old_price, &new_price, &note,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        sapid_out = sapid;
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售调价保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "SAPNo": sap_no, "SAPID": sapid_out }),
    )))
}
