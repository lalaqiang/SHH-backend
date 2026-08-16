use super::base_data::row_to_json;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::services::inventory_ledger;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tPur_Order WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PoNo LIKE @p1)");
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
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let disrate = json_f64(d, "DisRate");
    let curr = if json_str(d, "CurrCode").is_empty() {
        "CNY".to_string()
    } else {
        json_str(d, "CurrCode")
    };
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut poid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // 注意：tPur_Order 实际字段：POID, BTPID, SuppID, DeptID, EmpID, PoNo, PoDate, EndDate, CurrCode, StkID, DisRate, SumAmt, Note, State, EDate, EUser
        // POID 列 NOT NULL 且无默认值，必须显式 NEWID()
        // 使用 OUTPUT 子句直接获取插入的 POID，避免 SELECT by PoNo 在重复 PoNo 场景下错配
        let sql = "INSERT INTO tPur_Order (POID, PoNo, PoDate, StkID, SuppID, EmpID, DeptID, BTPID, DisRate, CurrCode, SumAmt, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.POID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &po_no, &dt, &stk_id, &supp_id, &emp_uuid, &dept_uuid, &btp_uuid,
            &disrate, &curr, &total_amt,
            &draft_state, &dt, &ZERO_UUID, &remark,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let poid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 POID".to_string()),
        };
        if poid.is_empty() {
            return Err("无法获取主表 POID".to_string());
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
            let barcode = json_str(det, "BarCode");
            let ain_price = json_f64(det, "AInPrice");
            let cnv_qty = json_f64(det, "CNVQty");
            let std_qty = json_f64(det, "StdQty");
            let dis_rate = json_f64(det, "DisRate");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let note = json_str(det, "Note");
            let pd_qty = json_f64(det, "PDQty");
            let pr_qty = json_f64(det, "PRQty");
            let stk_qty = json_f64(det, "StkQty");
            let pack_cnv_qty = json_f64(det, "PackCnvQty");
            let pack_qty = json_f64(det, "PackQty");
            let ds = "INSERT INTO tPur_OrderDetail (POID, PODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, BarCode, AInPrice, Price, \
                UnitNO, CNVQty, Qty, StdQty, DisRate, Amt, TaxRate, TaxAmt, Note, PDQty, PRQty, StkQty, PackCnvQty, PackQty) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, @p22, @p23)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &poid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &barcode, &ain_price, &price,
                &unit, &cnv_qty, &qty, &std_qty, &dis_rate, &amt, &tax_rate, &tax_amt, &note,
                &pd_qty, &pr_qty, &stk_qty, &pack_cnv_qty, &pack_qty,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        poid_out = poid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购订单保存失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "PoNo": po_no, "POID": poid_out }),
    )))
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
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tPur_Order WHERE POID=@p1",
                &[&params.poid],
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
    let po_no = json_str(d, "PoNo");
    if po_no.is_empty() {
        return Ok(Json(ApiResponse::err("PoNo 不能为空")));
    }
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tPur_Order SET SumAmt=@p1, Note=@p2, LUTime=GETDATE() WHERE POID=@p3";
    let p: Vec<&dyn tiberius::ToSql> = vec![&total_amt, &remark, &params.poid];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tPur_OrderDetail WHERE POID = @p1", &[&params.poid]).await.map_err(|e| e.to_string())?;
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
            let barcode = json_str(det, "BarCode");
            let ain_price = json_f64(det, "AInPrice");
            let cnv_qty = json_f64(det, "CNVQty");
            let std_qty = json_f64(det, "StdQty");
            let dis_rate = json_f64(det, "DisRate");
            let tax_rate = json_f64(det, "TaxRate");
            let tax_amt = json_f64(det, "TaxAmt");
            let note = json_str(det, "Note");
            let pd_qty = json_f64(det, "PDQty");
            let pr_qty = json_f64(det, "PRQty");
            let stk_qty = json_f64(det, "StkQty");
            let pack_cnv_qty = json_f64(det, "PackCnvQty");
            let pack_qty = json_f64(det, "PackQty");
            let ds = "INSERT INTO tPur_OrderDetail (POID, PODetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, BarCode, AInPrice, Price, \
                UnitNO, CNVQty, Qty, StdQty, DisRate, Amt, TaxRate, TaxAmt, Note, PDQty, PRQty, StkQty, PackCnvQty, PackQty) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20, @p21, @p22, @p23)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &params.poid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &barcode, &ain_price, &price,
                &unit, &cnv_qty, &qty, &std_qty, &dis_rate, &amt, &tax_rate, &tax_amt, &note,
                &pd_qty, &pr_qty, &stk_qty, &pack_cnv_qty, &pack_qty,
            ];
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购订单更新失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::msg("采购订单更新成功")))
}

// ============== 采购入库（tStk_IO, Kind='PD'，tPur_Inv 表不存在）==============
pub async fn get_purchase_inbound(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_IO WHERE Kind='PD' AND State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1)");
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

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut ioid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tPur_Inv 表不存在，统一写入 tStk_IO，Kind='PD'（采购入库，DIR_INBOUND）
        // IOID 列 NOT NULL 且无默认值，必须显式 NEWID()
        // 使用 OUTPUT 子句直接获取插入的 IOID，避免 SELECT by IONo 在重复单号场景下错配
        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, SuppID, EmpID, DeptID, BTPID, POID, DisRate, CurrCode, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, 'PD', @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 'N', @p13, @p14, @p15, @p16)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &stk_id, &supp_id, &emp_uuid, &dept_uuid, &btp_uuid, &po_uuid,
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
            let cnq = qty;
            let stdq = qty;
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
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, 0, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &ioid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &cnq, &stdq, &price, &amt,
                &aprice, &cprice, &tax_rate, &tax_amt, &dis_rate, &note, &barcode, &sou_id,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        ioid_out = ioid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购入库保存失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "IONo": io_no, "IOID": ioid_out }),
    )))
}

// ============== 采购退货（tStk_IO, Kind='PR'，tPur_Return 表不存在）==============
pub async fn get_purchase_return(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_IO WHERE Kind='PR' AND State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (IONo LIKE @p1)");
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
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // ===== P2-2 采购退货 POID 上下游校验 =====
    // 业务规则：累计退货(TH) 不能超过 累计入库(RI/PD)；
    //          已作废的 PO 禁止退货
    if po_uuid != ZERO_UUID {
        // 1) PO 存在性 + 状态
        let po_row = match conn
            .query(
                "SELECT CAST(POID AS NVARCHAR(40)) AS ID, ISNULL(SumQty, 0) AS Q, State \
             FROM tPur_Order WHERE POID = @p1",
                &[&po_uuid],
            )
            .await
        {
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
        let po_qty: f64 = row_get_f64(&r, "Q");

        // 2) 累计已入库 PD（已审核）
        let mut already_in: f64 = 0.0;
        let in_row = match conn
            .query(
                "SELECT ISNULL(SUM(d.Qty), 0) AS TotalIn \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.POID = @p1 AND io.Kind = 'PD' AND io.State IN ('S','Y')",
                &[&po_uuid],
            )
            .await
        {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = in_row {
            already_in = row_get_f64(&r, "TotalIn");
        }

        // 3) 累计已退货 PR（已审核）
        let mut already_ret: f64 = 0.0;
        let ret_row = match conn
            .query(
                "SELECT ISNULL(SUM(d.Qty), 0) AS TotalRet \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.POID = @p1 AND io.Kind IN ('PR') AND io.State IN ('S','Y')",
                &[&po_uuid],
            )
            .await
        {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = ret_row {
            already_ret = row_get_f64(&r, "TotalRet");
        }

        // 4) 本次退货不能超过已入库 - 已退货
        if total_qty.abs() > already_in - already_ret + 0.0001 {
            return Ok(Json(ApiResponse::err(&format!(
                "超量退货：PO数量={} 已入库={} 已退货={} 本次退货={}",
                po_qty,
                already_in,
                already_ret,
                total_qty.abs()
            ))));
        }
    }

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut ioid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // IOID 列 NOT NULL 且无默认值，必须显式 NEWID()；使用 OUTPUT 子句直接获取主键
        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, SuppID, EmpID, POID, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, 'PR', @p3, @p4, @p5, @p6, @p7, @p8, 'N', @p9, @p10, @p11, @p12)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &stk_id, &supp_id, &emp_uuid, &po_uuid, &total_amt, &total_qty,
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
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购退货保存失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "IONo": io_no, "IOID": ioid_out }),
    )))
}

// ============== 采购报价 ==============
pub async fn get_purchase_quotes(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tPur_Quote WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PqNo LIKE @p1)");
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
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut pqid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tPur_Quote 实际字段：PQID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
        let sql = "INSERT INTO tPur_Quote (PQID, PqNo, PqDate, StartDate, EndDate, SuppID, EmpID, BTPID, State, EDate, EUser, Note) \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11); \
            SELECT CAST(PQID AS NVARCHAR(40)) AS ID FROM tPur_Quote WHERE PqNo = @p12";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &pq_no, &dt, &dt, &end_dt, &supp_id, &emp_uuid, &btp_uuid,
            &draft_state, &dt, &ZERO_UUID, &remark, &pq_no,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let pqid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 PQID".to_string()),
        };
        if pqid.is_empty() {
            return Err("无法获取主表 PQID".to_string());
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
            let ds = "INSERT INTO tPur_QuoteDetail (PQID, PQDetailID, RowNO, GDSID, StkID, GDSNO, GDSDesc, UnitNO, Qty, Price, Amt) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &pqid, &row_no, &gdsid, &stk_id_d, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        pqid_out = pqid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购报价保存失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "PqNo": pq_no, "PQID": pqid_out }),
    )))
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
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tPur_Quote WHERE PQID=@p1",
                &[&params.pqid],
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
    let pq_no = json_str(d, "PqNo");
    if pq_no.is_empty() {
        return Ok(Json(ApiResponse::err("PqNo 不能为空")));
    }
    let remark = json_str(d, "Remark");
    let upd = "UPDATE tPur_Quote SET Note=@p1, LUTime=GETDATE() WHERE PqNo=@p2";
    let p: Vec<&dyn tiberius::ToSql> = vec![&remark, &pq_no];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tPur_QuoteDetail WHERE PQID = @p1", &[&params.pqid]).await.map_err(|e| e.to_string())?;
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
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购报价更新失败: {}",
            &e,
        ))));
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tPur_AdjPrice WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(" AND (PAPNo LIKE @p1)");
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
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化
    let mut paid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        crate::services::inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        // tPur_AdjPrice 实际字段：PAPID (uniqueidentifier NOT NULL, 无默认值) 需手动生成
        let sql = "INSERT INTO tPur_AdjPrice (PAPID, PAPNo, PAPDate, EmpID, DeptID, BTPID, State, EDate, EUser, Note) \
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9); \
            SELECT CAST(PAPID AS NVARCHAR(40)) AS ID FROM tPur_AdjPrice WHERE PAPNo = @p10";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &pap_no, &dt, &emp_uuid, &dept_uuid, &btp_uuid,
            &draft_state, &dt, &ZERO_UUID, &remark, &pap_no,
        ];
        let row_opt = conn.query(sql, &p).await.map_err(|e| format!("保存主表失败: {}", e))?
            .into_row().await.map_err(|e| e.to_string())?;
        let paid = match row_opt {
            Some(r) => r.get::<&str, _>("ID").unwrap_or("").to_string(),
            None => return Err("无法获取主表 PAPID".to_string()),
        };
        if paid.is_empty() {
            return Err("无法获取主表 PAPID".to_string());
        }
        for (i, det) in params.details.iter().enumerate() {
            let row_no = (i + 1) as i32;
            let gdsid = empty_or_zero(&json_str(det, "GDSID")).to_string();
            let gds_no = json_str(det, "GDSNO");
            let gds_desc = json_str(det, "GDSDesc");
            let unit = json_str(det, "UnitNO");
            let barcode = json_str(det, "BarCode");
            // tPur_AdjPriceDetail 价格变更字段
            let old_ain_price = json_f64(det, "OldAInPrice");
            let new_ain_price = json_f64(det, "NewAInPrice");
            let old_bprice = json_f64(det, "OldBPrice");
            let new_bprice = json_f64(det, "NewBPrice");
            let old_vprice = json_f64(det, "OldVPrice");
            let new_vprice = json_f64(det, "NewVPrice");
            let old_sprice = json_f64(det, "OldSPrice");
            let new_sprice = json_f64(det, "NewSPrice");
            let old_bprice2 = json_f64(det, "OldBPrice2");
            let new_bprice2 = json_f64(det, "NewBPrice2");
            let old_bprice3 = json_f64(det, "OldBPrice3");
            let new_bprice3 = json_f64(det, "NewBPrice3");
            let note = json_str(det, "Note");
            let ds = "INSERT INTO tPur_AdjPriceDetail (PAPID, PAPDetailID, RowNO, GDSID, GDSNO, GDSDesc, BarCode, UnitNO, \
                OldAInPrice, NewAInPrice, OldBPrice, NewBPrice, OldVPrice, NewVPrice, OldSPrice, NewSPrice, \
                OldBPrice2, NewBPrice2, OldBPrice3, NewBPrice3, Note) \
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11, @p12, @p13, @p14, @p15, @p16, @p17, @p18, @p19, @p20)";
            let dp: Vec<&dyn tiberius::ToSql> = vec![
                &paid, &row_no, &gdsid, &gds_no, &gds_desc, &barcode, &unit,
                &old_ain_price, &new_ain_price, &old_bprice, &new_bprice, &old_vprice, &new_vprice,
                &old_sprice, &new_sprice, &old_bprice2, &new_bprice2, &old_bprice3, &new_bprice3, &note,
            ];
            conn.execute(ds, &dp).await.map_err(|e| format!("保存明细(行{})失败: {}", i + 1, e))?;
        }

        paid_out = paid;
        crate::services::inventory_ledger::commit_tran(&mut conn).await.map_err(|e| e.to_string())?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        crate::services::inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&crate::utils::db_err(
            "采购调价保存失败: {}",
            &e,
        ))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "PAPNo": pap_no, "PAPID": paid_out }),
    )))
}

// ============== 采购综合查询 ==============
pub async fn get_purchase_query(
    State(_config): State<Config>,
    Json(params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let doc_type = params
        .get("doc_type")
        .and_then(|v| v.as_str())
        .unwrap_or("order");
    let keyword = params.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
    let page = params.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
    let page_size = params
        .get("page_size")
        .and_then(|v| v.as_u64())
        .unwrap_or(20);
    let (table, no_col) = match doc_type {
        "order" => ("tPur_Order", "PoNo"),
        "inbound" => ("tStk_IO", "IONo"),
        "return" => ("tStk_IO", "IONo"),
        "quote" => ("tPur_Quote", "PqNo"),
        "adjprice" => ("tPur_AdjPrice", "PAPNo"),
        _ => ("tPur_Order", "PoNo"),
    };
    let kind_filter = match doc_type {
        "inbound" => " AND Kind='PD'",
        "return" => " AND Kind='PR'",
        _ => "",
    };
    let base = format!("SELECT * FROM {} WHERE State <> 'D'{}", table, kind_filter);
    let mut base_q = base;
    if !keyword.is_empty() {
        base_q.push_str(&format!(" AND ({} LIKE @p1)", no_col));
    }
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_q);
    let paginated_sql = build_pagination_sql_with_sort(
        &base_q,
        page as u32,
        page_size as u32,
        params.get("sort_prop").and_then(|v| v.as_str()),
        params.get("sort_order").and_then(|v| v.as_str()),
    );
    let query_params: Vec<Option<String>> = if !keyword.is_empty() {
        vec![Some(format!("%{}%", keyword))]
    } else {
        vec![]
    };
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
        page as u32,
        page_size as u32,
    )))
}
