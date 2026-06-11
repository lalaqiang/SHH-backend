use axum::extract::{State, Json};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};
use crate::handlers::base_data::try_get_value;

#[derive(Deserialize)]
pub struct ReportParams {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

pub async fn get_purchase_report(
    State(_config): State<Config>,
    Json(params): Json<ReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = r#"
        SELECT CONVERT(varchar(7), PoDate, 120) as month,
               SUM(TotalAmt) as total_amt,
               COUNT(*) as order_count
        FROM tPur_Order
        WHERE State <> 'D'"#
        .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sql.push_str(&format!(" AND PoDate >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sql.push_str(&format!(" AND PoDate <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }

    sql.push_str(" GROUP BY CONVERT(varchar(7), PoDate, 120) ORDER BY month");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let total_amt = row_get_f64(&row, "total_amt");
            let order_count = row.get::<i32, _>("order_count").unwrap_or(0);
            serde_json::json!({
                "month": row.get::<&str, _>("month").unwrap_or(""),
                "totalAmt": total_amt as i64,
                "orderCount": order_count
            })
        })
        .collect();

    let mut grand_total: f64 = 0.0;
    let mut grand_count: i32 = 0;
    for item in &items {
        if let Some(amt) = item.get("totalAmt").and_then(|v| v.as_i64()) {
            grand_total += amt as f64;
        }
        if let Some(cnt) = item.get("orderCount").and_then(|v| v.as_i64()) {
            grand_count += cnt as i32;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalAmt": grand_total as i64,
            "orderCount": grand_count
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_sales_report(
    State(_config): State<Config>,
    Json(params): Json<ReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = r#"
        SELECT CONVERT(varchar(7), InvDate, 120) as month,
               SUM(TotalAmt) as total_amt,
               SUM(CostAmt) as cost_amt
        FROM tSal_Inv
        WHERE State <> 'D'"#
        .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sql.push_str(&format!(" AND InvDate >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sql.push_str(&format!(" AND InvDate <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }

    sql.push_str(" GROUP BY CONVERT(varchar(7), InvDate, 120) ORDER BY month");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let total_amt = row_get_f64(&row, "total_amt");
            let cost_amt = row_get_f64(&row, "cost_amt");
            let profit = total_amt - cost_amt;
            serde_json::json!({
                "month": row.get::<&str, _>("month").unwrap_or(""),
                "totalAmt": total_amt as i64,
                "costAmt": cost_amt as i64,
                "profit": profit as i64
            })
        })
        .collect();

    let mut grand_sales: f64 = 0.0;
    let mut grand_cost: f64 = 0.0;
    for item in &items {
        if let Some(amt) = item.get("totalAmt").and_then(|v| v.as_i64()) {
            grand_sales += amt as f64;
        }
        if let Some(amt) = item.get("costAmt").and_then(|v| v.as_i64()) {
            grand_cost += amt as f64;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalAmt": grand_sales as i64,
            "costAmt": grand_cost as i64,
            "profit": (grand_sales - grand_cost) as i64
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_business_report(
    State(_config): State<Config>,
    Json(params): Json<ReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut purchase_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    let mut purchase_sql = r#"
        SELECT CONVERT(varchar(7), PoDate, 120) as month,
               SUM(TotalAmt) as purchase_amt,
               COUNT(*) as purchase_count
        FROM tPur_Order
        WHERE State <> 'D'"#
        .to_string();

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            purchase_sql.push_str(&format!(" AND PoDate >= @p{}", pidx));
            pidx += 1;
            purchase_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            purchase_sql.push_str(&format!(" AND PoDate <= @p{}", pidx));
            pidx += 1;
            purchase_params.push(Some(ed.clone()));
        }
    }

    purchase_sql.push_str(" GROUP BY CONVERT(varchar(7), PoDate, 120)");

    let purchase_refs: Vec<&dyn tiberius::ToSql> = purchase_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut purchase_map: std::collections::HashMap<String, (f64, i32)> =
        std::collections::HashMap::new();

    if let Ok(stream) = conn.query(&purchase_sql, &purchase_refs).await {
        if let Ok(rows) = stream.into_first_result().await {
            for row in &rows {
                let month = row
                    .get::<&str, _>("month")
                    .unwrap_or("")
                    .to_string();
                let amt = row_get_f64(&row, "purchase_amt");
                let cnt = row.get::<i32, _>("purchase_count").unwrap_or(0);
                purchase_map.insert(month, (amt, cnt));
            }
        }
    }

    let mut sales_params: Vec<Option<String>> = Vec::new();
    let mut sidx = 1;

    let mut sales_sql = r#"
        SELECT CONVERT(varchar(7), InvDate, 120) as month,
               SUM(TotalAmt) as sales_amt,
               SUM(CostAmt) as cost_amt
        FROM tSal_Inv
        WHERE State <> 'D'"#
        .to_string();

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sales_sql.push_str(&format!(" AND InvDate >= @p{}", sidx));
            sidx += 1;
            sales_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sales_sql.push_str(&format!(" AND InvDate <= @p{}", sidx));
            sidx += 1;
            sales_params.push(Some(ed.clone()));
        }
    }

    sales_sql.push_str(" GROUP BY CONVERT(varchar(7), InvDate, 120)");

    let sales_refs: Vec<&dyn tiberius::ToSql> = sales_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let mut sales_map: std::collections::HashMap<String, (f64, f64)> =
        std::collections::HashMap::new();

    if let Ok(stream) = conn.query(&sales_sql, &sales_refs).await {
        if let Ok(rows) = stream.into_first_result().await {
            for row in &rows {
                let month = row
                    .get::<&str, _>("month")
                    .unwrap_or("")
                    .to_string();
                let sales_amt = row_get_f64(&row, "sales_amt");
                let cost_amt = row_get_f64(&row, "cost_amt");
                sales_map.insert(month, (sales_amt, cost_amt));
            }
        }
    }

    let mut all_months: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for k in purchase_map.keys() {
        all_months.insert(k.clone());
    }
    for k in sales_map.keys() {
        all_months.insert(k.clone());
    }

    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut grand_purchase: f64 = 0.0;
    let mut grand_sales: f64 = 0.0;
    let mut grand_cost: f64 = 0.0;

    for month in &all_months {
        let (p_amt, p_cnt) = purchase_map.get(month).copied().unwrap_or((0.0, 0));
        let (s_amt, c_amt) = sales_map.get(month).copied().unwrap_or((0.0, 0.0));
        let profit = s_amt - c_amt;

        grand_purchase += p_amt;
        grand_sales += s_amt;
        grand_cost += c_amt;

        items.push(serde_json::json!({
            "month": month,
            "purchaseAmt": p_amt as i64,
            "purchaseCount": p_cnt,
            "salesAmt": s_amt as i64,
            "costAmt": c_amt as i64,
            "profit": profit as i64
        }));
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "purchaseAmt": grand_purchase as i64,
            "salesAmt": grand_sales as i64,
            "costAmt": grand_cost as i64,
            "profit": (grand_sales - grand_cost) as i64
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_stock_report(
    State(_config): State<Config>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"
        SELECT s.StockID, s.StockName,
               SUM(ISNULL(d.Qty, 0)) as total_qty,
               COUNT(DISTINCT d.GDSNO) as goods_count
        FROM tBas_Stock s
        LEFT JOIN tStk_Stock d ON s.StockID = d.StockID
        WHERE s.Used <> 'N'
        GROUP BY s.StockID, s.StockName
        ORDER BY s.StockID"#;

    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let total_qty = row_get_f64(&row, "total_qty");
            let goods_count = row.get::<i32, _>("goods_count").unwrap_or(0);
            serde_json::json!({
                "stockID": row.get::<&str, _>("StockID").unwrap_or(""),
                "stockName": row.get::<&str, _>("StockName").unwrap_or(""),
                "totalQty": total_qty as i64,
                "goodsCount": goods_count
            })
        })
        .collect();

    let mut grand_qty: f64 = 0.0;
    let mut grand_goods: i32 = 0;
    for item in &items {
        if let Some(qty) = item.get("totalQty").and_then(|v| v.as_i64()) {
            grand_qty += qty as f64;
        }
        if let Some(cnt) = item.get("goodsCount").and_then(|v| v.as_i64()) {
            grand_goods += cnt as i32;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalQty": grand_qty as i64,
            "goodsCount": grand_goods
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}
