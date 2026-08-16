use super::base_data::row_to_json;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::services::inventory_ledger;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use axum::{Json, extract::State};
use serde::Deserialize;
use tiberius::Row;

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

// 销售退货 = 写入 tStk_IO (Kind='SR') + tStk_IODetail
// 因为数据库里没有 tSal_Return 表（已 sqlcmd 确认）

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn list_sales_return(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);
    let mut base_query = "SELECT * FROM tStk_IO WHERE Kind='SR' AND State <> 'D'".to_string();
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
pub struct CreateSalesReturnRequest {
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn create_sales_return(
    State(_config): State<Config>,
    Json(params): Json<CreateSalesReturnRequest>,
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
    let so_uuid = empty_or_zero(&json_str(d, "SOID")).to_string();
    let total_amt: f64 = params
        .details
        .iter()
        .map(|x| json_f64(x, "Amt").max(json_f64(x, "Qty") * json_f64(x, "Price")))
        .sum();
    let total_qty: f64 = params.details.iter().map(|x| json_f64(x, "Qty")).sum();
    let remark = json_str(d, "Remark");
    let dt = now();
    let draft_state: &str = crate::handlers::doc_state::STATE_NEW;

    // ===== P2-1 销售退货 SOID 上下游校验 =====
    // 业务规则：累计退货(SR) 不能超过 累计出库(SD)；
    //          已作废的 SO 禁止退货
    if so_uuid != ZERO_UUID {
        // 1) SO 存在性 + 状态
        let so_row = match conn
            .query(
                "SELECT CAST(SOID AS NVARCHAR(40)) AS ID, ISNULL(SumQty, 0) AS Q, State \
             FROM tSal_Order WHERE SOID = @p1",
                &[&so_uuid],
            )
            .await
        {
            Ok(s) => match s.into_row().await {
                Ok(r) => r,
                Err(_) => None,
            },
            Err(_) => None,
        };

        let r = match so_row {
            Some(r) => r,
            None => return Ok(Json(ApiResponse::err("销售订单不存在"))),
        };
        let state = r.get::<&str, _>("State").unwrap_or("").to_string();
        if state == "D" || state == "C" {
            return Ok(Json(ApiResponse::err("该销售订单已作废，无法退货")));
        }
        let so_qty: f64 = row_get_f64(&r, "Q");

        // 2) 累计已出库 (SD/SI/POS 已审核)
        let mut already_out: f64 = 0.0;
        let out_row = match conn
            .query(
                "SELECT ISNULL(SUM(d.Qty), 0) AS TotalOut \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.SOID = @p1 AND io.Kind IN ('SD','SI','POS') AND io.State IN ('S','Y')",
                &[&so_uuid],
            )
            .await
        {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = out_row {
            already_out = row_get_f64(&r, "TotalOut");
        }

        // 3) 累计已退货 (SR 已审核)
        let mut already_ret: f64 = 0.0;
        let ret_row = match conn
            .query(
                "SELECT ISNULL(SUM(d.Qty), 0) AS TotalRet \
             FROM tStk_IODetail d \
             INNER JOIN tStk_IO io ON io.IOID = d.IOID \
             WHERE io.SOID = @p1 AND io.Kind = 'SR' AND io.State IN ('S','Y')",
                &[&so_uuid],
            )
            .await
        {
            Ok(s) => s.into_row().await.ok().flatten(),
            Err(_) => None,
        };
        if let Some(r) = ret_row {
            already_ret = row_get_f64(&r, "TotalRet");
        }

        // 4) 本次退货后: 已出 - 已退 - 本次 <= 0 表示超退
        if total_qty.abs() > already_out - already_ret + 0.0001 {
            return Ok(Json(ApiResponse::err(&format!(
                "超量退货：SO数量={} 已出库={} 已退货={} 本次退货={}",
                so_qty,
                already_out,
                already_ret,
                total_qty.abs()
            ))));
        }
    }

    // 事务包裹：INSERT 主表 + INSERT 明细 原子化，任一明细失败回滚，避免主表残留无明细的脏数据
    // 销售退货：库存 +Qty，由 /api/doc/approve 审核时统一写入三件套
    let mut ioid_out: String = String::new();
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await.map_err(|e| e.to_string())?;

        let sql = "INSERT INTO tStk_IO (IOID, IONo, IoDate, Kind, StkID, CustID, EmpID, DeptID, SOID, SumAmt, SumQty, ScanMode, State, EDate, EUser, Note) \
            OUTPUT CAST(INSERTED.IOID AS NVARCHAR(40)) AS ID \
            VALUES (NEWID(), @p1, @p2, 'SR', @p3, @p4, @p5, @p6, @p7, @p8, @p9, 'N', @p10, @p11, @p12, @p13)";
        let p: Vec<&dyn tiberius::ToSql> = vec![
            &io_no, &dt, &stk_id, &cust_id, &emp_uuid, &dept_uuid, &so_uuid, &total_amt, &total_qty,
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
        return Ok(Json(ApiResponse::err(&format!("销售退货保存失败: {}", e))));
    }
    Ok(Json(ApiResponse::ok(
        serde_json::json!({ "IONo": io_no, "IOID": ioid_out }),
    )))
}

#[derive(Deserialize)]
pub struct UpdateSalesReturnRequest {
    pub ioid: String,
    pub data: serde_json::Value,
    pub details: Vec<serde_json::Value>,
}

pub async fn update_sales_return(
    State(_config): State<Config>,
    Json(params): Json<UpdateSalesReturnRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    // ===== 编辑锁 =====
    {
        let state_check = conn
            .query(
                "SELECT State FROM tStk_IO WHERE IOID=@p1 AND Kind='SR'",
                &[&params.ioid],
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

    let upd = "UPDATE tStk_IO SET StkID=@p1, CustID=@p2, SumAmt=@p3, SumQty=@p4, Note=@p5, LUTime=GETDATE() WHERE IONo=@p6 AND Kind='SR'";
    let p: Vec<&dyn tiberius::ToSql> =
        vec![&stk_id, &cust_id, &total_amt, &total_qty, &remark, &io_no];
    // 事务包裹：UPDATE 主表 + DELETE 旧明细 + INSERT 新明细 原子化，避免中途失败导致明细丢失
    let tx_result: std::result::Result<(), String> = async {
        inventory_ledger::begin_tran(&mut conn).await?;
        conn.execute(upd, &p).await.map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM tStk_IODetail WHERE IOID = @p1", &[&params.ioid]).await.map_err(|e| e.to_string())?;
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
                &params.ioid, &row_no, &gdsid, &stk_id, &gds_no, &gds_desc, &unit, &qty, &price, &amt,
                &aprice, &cprice, &tax_rate, &tax_amt, &dis_rate, &note, &barcode, &sou_id,
            ];
            conn.execute(ds, &dp).await.map_err(|e| e.to_string())?;
        }
        inventory_ledger::commit_tran(&mut conn).await?;
        Ok(())
    }.await;
    if let Err(e) = tx_result {
        inventory_ledger::rollback_tran(&mut conn).await;
        return Ok(Json(ApiResponse::err(&format!("销售退货更新失败: {}", e))));
    }
    Ok(Json(ApiResponse::msg("销售退货更新成功")))
}
