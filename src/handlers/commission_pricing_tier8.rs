use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::middleware::auth::Claims;
use crate::utils::{ApiResponse, row_get_f64};
use axum::{Extension, Json, extract::State};
use serde::Deserialize;
use tiberius::Row;

// =====================================================================
// 提成计算引擎
// =====================================================================

#[derive(Deserialize)]
pub struct CommissionCalcParams {
    pub emp_id: Option<String>,
    pub dept_id: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub period: Option<String>,
}

fn apply_commission_rate(net_amt: f64, base_rate: f64) -> f64 {
    let rate = base_rate.max(0.0).min(1.0);
    (net_amt * rate).max(0.0)
}

fn resolve_period(
    start_date: Option<&str>,
    end_date: Option<&str>,
    period: Option<&str>,
) -> (String, String) {
    if let (Some(s), Some(e)) = (start_date, end_date) {
        if !s.is_empty() && !e.is_empty() {
            return (s.to_string(), e.to_string());
        }
    }
    if let Some(p) = period {
        if !p.is_empty() {
            let parts: Vec<&str> = p.split('-').collect();
            if parts.len() == 2 {
                let year: i32 = parts[0].parse().unwrap_or(2026);
                let month: i32 = parts[1].parse().unwrap_or(1);
                let start = format!("{:04}-{:02}-01", year, month);
                let next_year = if month == 12 { year + 1 } else { year };
                let next_month = if month == 12 { 1 } else { month + 1 };
                if let Ok(d) = chrono::NaiveDate::parse_from_str(
                    &format!("{:04}-{:02}-01", next_year, next_month),
                    "%Y-%m-%d",
                ) {
                    let last = (d - chrono::Duration::days(1))
                        .format("%Y-%m-%d")
                        .to_string();
                    return (start, last);
                }
            }
        }
    }
    let today = chrono::Local::now();
    (
        today.format("%Y-%m-01").to_string(),
        today.format("%Y-%m-%d").to_string(),
    )
}

pub async fn calculate_employee_commission(
    State(_config): State<Config>,
    Json(params): Json<CommissionCalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let (start_date, end_date) = resolve_period(
        params.start_date.as_deref(),
        params.end_date.as_deref(),
        params.period.as_deref(),
    );

    let emp_id = params.emp_id.as_deref().unwrap_or("");
    if emp_id.is_empty() {
        return Ok(Json(ApiResponse::err("emp_id 不能为空")));
    }

    let emp_sql = "SELECT EmpID, EmpName, DeptID FROM tBas_Emp WHERE EmpID = @p1 AND State <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&emp_id]).await?;
    let emp_row = match emp_stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("员工不存在"))),
    };
    let emp_name: String = emp_row.get::<&str, _>("EmpName").unwrap_or("").to_string();
    // tBas_Emp 表没有 CommRate 字段，base_rate 后面会用 tpl_rate 替代
    let base_rate: f64 = 0.0;

    // 销售汇总
    let sale_sql = r#"
        SELECT
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty * d.Price ELSE 0 END), 0) AS SaleAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty * d.Price ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty ELSE 0 END), 0)
              - ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty ELSE 0 END), 0) AS NetQty,
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty * d.Price ELSE 0 END), 0)
              - ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty * d.Price ELSE 0 END), 0) AS NetAmt
        FROM tStk_IO io
        INNER JOIN tStk_IODetail d ON io.IOID = d.IOID
        WHERE io.State IN ('S', 'Y')
          AND io.EmpID = @p1
          AND io.IODate >= @p2
          AND io.IODate < DATEADD(day, 1, @p3)
    "#;
    let sale_stream = conn
        .query(sale_sql, &[&emp_id, &start_date, &end_date])
        .await?;
    let (sale_amt, return_amt, net_amt, net_qty) = if let Some(row) = sale_stream.into_row().await?
    {
        (
            row_get_f64(&row, "SaleAmt"),
            row_get_f64(&row, "ReturnAmt"),
            row_get_f64(&row, "NetAmt"),
            row_get_f64(&row, "NetQty") as i32,
        )
    } else {
        (0.0, 0.0, 0.0, 0)
    };

    // 注意：tSys_Parameters 表没有 State 字段，不能用 State <> 'D' 条件
    // PValue 是 nvarchar 类型，需用 TRY_CAST 转换为 FLOAT
    let tpl_sql = "SELECT TOP 1 TRY_CAST(PValue AS FLOAT) AS PValue, PName FROM tSys_Parameters WHERE PKind = 'commission' AND (PTerm = 'ALL' OR PTerm = 'SAL') ORDER BY EDate DESC";
    let tpl_stream = conn.query(tpl_sql, &[]).await?;
    let (tpl_rate, tpl_name) = if let Some(r) = tpl_stream.into_row().await? {
        (
            r.get::<f64, _>("PValue").unwrap_or(0.0) / 100.0,
            r.get::<&str, _>("PName").unwrap_or("").to_string(),
        )
    } else {
        (0.0, String::new())
    };

    let actual_rate = base_rate.max(tpl_rate);
    let comm_amt = apply_commission_rate(net_amt, actual_rate);

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "empId": emp_id,
        "empName": emp_name,
        "startDate": start_date,
        "endDate": end_date,
        "saleAmt": sale_amt,
        "returnAmt": return_amt,
        "netAmt": net_amt,
        "netQty": net_qty,
        "baseRate": base_rate,
        "tplRate": tpl_rate,
        "tplName": tpl_name,
        "actualRate": actual_rate,
        "commAmt": comm_amt,
    }))))
}

pub async fn calculate_all_commission(
    State(_config): State<Config>,
    Json(params): Json<CommissionCalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let (start_date, end_date) = resolve_period(
        params.start_date.as_deref(),
        params.end_date.as_deref(),
        params.period.as_deref(),
    );

    // 注意：tSys_Parameters 表没有 State 字段，不能用 State <> 'D' 条件
    // 提成率从 PValue 字段读取（nvarchar 类型，需转换为 float）
    let tpl_sql = "SELECT TOP 1 TRY_CAST(PValue AS FLOAT) AS PValue FROM tSys_Parameters WHERE PKind = 'commission' AND (PTerm = 'ALL' OR PTerm = 'SAL') ORDER BY EDate DESC";
    let tpl_stream = conn.query(tpl_sql, &[]).await?;
    let tpl_rate: f64 = if let Ok(Some(r)) = tpl_stream.into_row().await {
        r.get::<f64, _>("PValue").unwrap_or(0.0) / 100.0
    } else {
        0.0
    };

    // 注意：tBas_Emp 表没有 CommRate 字段，提成率统一使用模板提成率 tpl_rate
    // WorkState 在数据库中是 char 类型，'1' 表示在职
    // 注意：EmpID 是 uniqueidentifier 类型，tiberius 无法直接读取为 String，需 CONVERT(varchar(40))
    let mut emp_sql = "SELECT CONVERT(varchar(40), EmpID) AS EmpID, EmpName FROM tBas_Emp WHERE State <> 'D' AND WorkState = '1'".to_string();
    if let Some(did) = &params.dept_id {
        if !did.is_empty() {
            let safe = did.replace('\'', "''");
            emp_sql.push_str(&format!(" AND DeptID = '{}'", safe));
        }
    }
    emp_sql.push_str(" ORDER BY EmpID");

    let stream = conn.query(&emp_sql, &[]).await?;
    let emp_rows: Vec<Row> = stream.into_first_result().await?;
    let mut results = Vec::new();
    let mut total_sale = 0.0;
    let mut total_return = 0.0;
    let mut total_comm = 0.0;

    let sale_sql = r#"
        SELECT
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty * d.Price ELSE 0 END), 0) AS SaleAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty * d.Price ELSE 0 END), 0) AS ReturnAmt,
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty ELSE 0 END), 0)
              - ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty ELSE 0 END), 0) AS NetQty,
            ISNULL(SUM(CASE WHEN io.Kind = 'SD' THEN d.Qty * d.Price ELSE 0 END), 0)
              - ISNULL(SUM(CASE WHEN io.Kind = 'SR' THEN d.Qty * d.Price ELSE 0 END), 0) AS NetAmt
        FROM tStk_IO io
        INNER JOIN tStk_IODetail d ON io.IOID = d.IOID
        WHERE io.State IN ('S', 'Y')
          AND io.EmpID = @p1
          AND io.IODate >= @p2
          AND io.IODate < DATEADD(day, 1, @p3)
    "#;

    for er in emp_rows {
        let eid: String = er.get::<&str, _>("EmpID").unwrap_or("").to_string();
        let ename: String = er.get::<&str, _>("EmpName").unwrap_or("").to_string();
        // tBas_Emp 表没有 CommRate 字段，统一使用模板提成率 tpl_rate 作为 base_rate
        let base_rate: f64 = tpl_rate;

        if eid.is_empty() {
            continue;
        }

        let sale_stream = match conn.query(sale_sql, &[&eid, &start_date, &end_date]).await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (sale_amt, return_amt, net_amt, net_qty) =
            if let Ok(Some(row)) = sale_stream.into_row().await {
                (
                    row_get_f64(&row, "SaleAmt"),
                    row_get_f64(&row, "ReturnAmt"),
                    row_get_f64(&row, "NetAmt"),
                    row_get_f64(&row, "NetQty") as i32,
                )
            } else {
                (0.0, 0.0, 0.0, 0)
            };

        if net_amt.abs() < 0.01 && sale_amt.abs() < 0.01 {
            continue;
        }

        let actual_rate = base_rate.max(tpl_rate);
        let comm_amt = apply_commission_rate(net_amt, actual_rate);

        total_sale += sale_amt;
        total_return += return_amt;
        total_comm += comm_amt;

        results.push(serde_json::json!({
            "empId": eid,
            "empName": ename,
            "saleAmt": sale_amt,
            "returnAmt": return_amt,
            "netAmt": net_amt,
            "netQty": net_qty,
            "baseRate": base_rate,
            "tplRate": tpl_rate,
            "actualRate": actual_rate,
            "commAmt": comm_amt,
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "startDate": start_date,
        "endDate": end_date,
        "tplRate": tpl_rate,
        "totalSale": total_sale,
        "totalReturn": total_return,
        "totalComm": total_comm,
        "items": results,
    }))))
}

pub async fn get_commission_details(
    State(_config): State<Config>,
    Json(params): Json<CommissionCalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let (start_date, end_date) = resolve_period(
        params.start_date.as_deref(),
        params.end_date.as_deref(),
        params.period.as_deref(),
    );
    let emp_id = params.emp_id.as_deref().unwrap_or("");
    if emp_id.is_empty() {
        return Ok(Json(ApiResponse::err("emp_id 不能为空")));
    }

    let sql = r#"
        SELECT io.IOID, io.IOBillNo, io.Kind, CONVERT(varchar(10), io.IODate, 120) AS IODate, c.CustName,
               d.GDSID, g.GDSNO, g.GDSDesc, d.Qty, d.Price, d.Amt
        FROM tStk_IO io
        INNER JOIN tStk_IODetail d ON io.IOID = d.IOID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Cust c ON io.CustID = c.CustID
        WHERE io.State IN ('S', 'Y')
          AND io.Kind IN ('SD', 'SR')
          AND io.EmpID = @p1
          AND io.IODate >= @p2
          AND io.IODate < DATEADD(day, 1, @p3)
        ORDER BY io.IODate DESC, io.IOBillNo
    "#;
    let stream = conn.query(sql, &[&emp_id, &start_date, &end_date]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut items = Vec::new();
    let mut total_amt = 0.0;
    for r in rows.iter() {
        let amt: f64 = r.get::<f64, _>("Amt").unwrap_or(0.0);
        total_amt += amt;
        items.push(serde_json::json!({
            "ioId": r.get::<&str, _>("IOID").unwrap_or(""),
            "billNo": r.get::<&str, _>("IOBillNo").unwrap_or(""),
            "kind": r.get::<&str, _>("Kind").unwrap_or(""),
            "ioDate": r.get::<&str, _>("IODate").unwrap_or(""),
            "custName": r.get::<&str, _>("CustName").unwrap_or(""),
            "gdsId": r.get::<&str, _>("GDSID").unwrap_or(""),
            "gdsNo": r.get::<&str, _>("GDSNO").unwrap_or(""),
            "gdsDesc": r.get::<&str, _>("GDSDesc").unwrap_or(""),
            "qty": r.get::<f64, _>("Qty").unwrap_or(0.0),
            "price": r.get::<f64, _>("Price").unwrap_or(0.0),
            "amt": amt,
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "empId": emp_id,
        "startDate": start_date,
        "endDate": end_date,
        "totalAmt": total_amt,
        "count": items.len(),
        "items": items,
    }))))
}

// =====================================================================
// 价格模板应用引擎
// =====================================================================

#[derive(Deserialize)]
pub struct PricingCalcParams {
    pub cust_id: Option<String>,
    pub gds_id: Option<String>,
    pub brand_id: Option<String>,
    pub template_id: Option<String>,
}

pub async fn apply_pricing_for_customer(
    State(_config): State<Config>,
    Json(params): Json<PricingCalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let cust_id = params.cust_id.as_deref().unwrap_or("");
    let gds_id = params.gds_id.as_deref().unwrap_or("");
    if cust_id.is_empty() || gds_id.is_empty() {
        return Ok(Json(ApiResponse::err("cust_id 和 gds_id 必填")));
    }

    let gds_sql = "SELECT GDSID, GDSNO, GDSDesc, ISNULL(SPrice, 0) AS BasePrice, BrandID FROM tBas_Goods WHERE GDSID = @p1 AND State <> 'D'";
    let stream = conn.query(gds_sql, &[&gds_id]).await?;
    let gds_row = match stream.into_row().await? {
        Some(r) => r,
        None => return Ok(Json(ApiResponse::err("商品不存在"))),
    };
    let base_price: f64 = row_get_f64(&gds_row, "BasePrice");
    let gds_no: String = gds_row.get::<&str, _>("GDSNO").unwrap_or("").to_string();
    let gds_desc: String = gds_row.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
    let brand_id: String = gds_row.get::<&str, _>("BrandID").unwrap_or("").to_string();

    let mut rate = 1.0_f64;
    let mut template_id: String = String::new();
    let mut template_name: String = String::new();
    if !brand_id.is_empty() {
        let pl_sql = "SELECT cp.PLID, p.PName, ISNULL(p.PValue, 1) AS Rate FROM tBas_CustPriceTac cp LEFT JOIN tSys_Parameters p ON cp.PLID = p.ParametersID WHERE cp.CustID = @p1 AND cp.BrandID = @p2";
        let pl_stream = conn.query(pl_sql, &[&cust_id, &brand_id]).await?;
        if let Some(row) = pl_stream.into_row().await? {
            template_id = row.get::<&str, _>("PLID").unwrap_or("").to_string();
            template_name = row.get::<&str, _>("PName").unwrap_or("").to_string();
            rate = row_get_f64(&row, "Rate");
            if rate == 0.0 {
                rate = 1.0;
            }
        }
    }

    let final_price = base_price * rate;
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "custId": cust_id,
        "gdsId": gds_id,
        "gdsNo": gds_no,
        "gdsDesc": gds_desc,
        "brandId": brand_id,
        "basePrice": base_price,
        "rate": rate,
        "templateId": template_id,
        "templateName": template_name,
        "finalPrice": final_price,
    }))))
}

pub async fn get_customer_price_list(
    State(_config): State<Config>,
    Json(params): Json<PricingCalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let cust_id = params.cust_id.as_deref().unwrap_or("");
    if cust_id.is_empty() {
        return Ok(Json(ApiResponse::err("cust_id 必填")));
    }

    let bind_sql = "SELECT cp.BrandID, b.BrandName, cp.PLID, p.PName AS TemplateName, ISNULL(p.PValue, 1) AS Rate FROM tBas_CustPriceTac cp LEFT JOIN tBas_Brand b ON cp.BrandID = b.BrandID LEFT JOIN tSys_Parameters p ON cp.PLID = p.ParametersID WHERE cp.CustID = @p1";
    let bind_stream = conn.query(bind_sql, &[&cust_id]).await?;
    let bind_rows: Vec<Row> = bind_stream.into_first_result().await?;

    let mut gds_sql = "SELECT g.GDSID, g.GDSNO, g.GDSDesc, g.BrandID, b.BrandName, ISNULL(g.SPrice, 0) AS BasePrice FROM tBas_Goods g LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID WHERE g.State <> 'D'".to_string();
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            let safe = bid.replace('\'', "''");
            gds_sql.push_str(&format!(" AND g.BrandID = '{}'", safe));
        }
    }
    gds_sql.push_str(" ORDER BY g.GDSNO");

    let gds_stream = conn.query(&gds_sql, &[]).await?;
    let gds_rows: Vec<Row> = gds_stream.into_first_result().await?;

    let mut brand_rate: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();
    for r in bind_rows.iter() {
        let bid: String = r.get::<&str, _>("BrandID").unwrap_or("").to_string();
        if !bid.is_empty() {
            brand_rate.insert(
                bid,
                (
                    r.get::<&str, _>("PLID").unwrap_or("").to_string(),
                    r.get::<&str, _>("TemplateName").unwrap_or("").to_string(),
                    r.get::<f64, _>("Rate").unwrap_or(1.0),
                ),
            );
        }
    }

    let mut items = Vec::new();
    let mut total_base = 0.0;
    let mut total_final = 0.0;
    for r in gds_rows.iter() {
        let base_price: f64 = r.get::<f64, _>("BasePrice").unwrap_or(0.0);
        let brand_id: String = r.get::<&str, _>("BrandID").unwrap_or("").to_string();
        let (tpl_id, tpl_name, rate) =
            brand_rate
                .get(&brand_id)
                .cloned()
                .unwrap_or((String::new(), String::new(), 1.0));
        let final_price = base_price * rate;
        total_base += base_price;
        total_final += final_price;
        items.push(serde_json::json!({
            "gdsId": r.get::<&str, _>("GDSID").unwrap_or(""),
            "gdsNo": r.get::<&str, _>("GDSNO").unwrap_or(""),
            "gdsDesc": r.get::<&str, _>("GDSDesc").unwrap_or(""),
            "brandId": brand_id,
            "brandName": r.get::<&str, _>("BrandName").unwrap_or(""),
            "basePrice": base_price,
            "rate": rate,
            "templateId": tpl_id,
            "templateName": tpl_name,
            "finalPrice": final_price,
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "custId": cust_id,
        "totalBase": total_base,
        "totalFinal": total_final,
        "saved": total_base - total_final,
        "count": items.len(),
        "items": items,
    }))))
}

#[derive(Deserialize)]
pub struct BulkPricingParams {
    pub template_id: Option<String>,
    pub brand_id: Option<String>,
    pub cust_ids: Option<Vec<String>>,
    pub rate: Option<f64>,
    pub overwrite: Option<bool>,
}

pub async fn bulk_apply_pricing_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<BulkPricingParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let template_id = params.template_id.as_deref().unwrap_or("");
    let brand_id = params.brand_id.as_deref().unwrap_or("");
    let cust_ids = params.cust_ids.clone().unwrap_or_default();
    if template_id.is_empty() || brand_id.is_empty() || cust_ids.is_empty() {
        return Ok(Json(ApiResponse::err(
            "template_id、brand_id、cust_ids 必填",
        )));
    }

    let overwrite = params.overwrite.unwrap_or(false);
    let mut success = 0_i32;
    let mut skip = 0_i32;

    for cid in &cust_ids {
        if cid.is_empty() {
            continue;
        }
        let check_sql =
            "SELECT COUNT(*) AS cnt FROM tBas_CustPriceTac WHERE CustID = @p1 AND BrandID = @p2";
        let check_stream = conn.query(check_sql, &[&cid.as_str(), &brand_id]).await?;
        let exists = if let Some(row) = check_stream.into_row().await? {
            row.get::<i32, _>("cnt").unwrap_or(0) > 0
        } else {
            false
        };

        if exists && !overwrite {
            skip += 1;
            continue;
        }

        if exists {
            let upd_sql =
                "UPDATE tBas_CustPriceTac SET PLID = @p1 WHERE CustID = @p2 AND BrandID = @p3";
            conn.execute(upd_sql, &[&template_id, &cid.as_str(), &brand_id])
                .await?;
        } else {
            let ins_sql =
                "INSERT INTO tBas_CustPriceTac (CustID, BrandID, PLID) VALUES (@p1, @p2, @p3)";
            conn.execute(ins_sql, &[&cid.as_str(), &brand_id, &template_id])
                .await?;
        }
        success += 1;
    }

    let _ = claims;
    Ok(Json(ApiResponse::ok(serde_json::json!({
        "success": success,
        "skip": skip,
        "total": cust_ids.len(),
    }))))
}
