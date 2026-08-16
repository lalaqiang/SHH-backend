use axum::extract::{State, Json, Extension};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, row_get_f64};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct ReportParams {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    // P0-2 修复：扩展业务维度筛选字段（所有 Option，不传时 None，不影响其他报表）
    pub warehouse_id: Option<String>,   // 仓库 ID
    pub gdstype_id: Option<String>,     // 商品类型 ID
    pub brand_id: Option<String>,       // 品牌 ID
    pub supp_id: Option<String>,        // 供应商 ID
    pub cust_id: Option<String>,        // 客户 ID
    pub emp_id: Option<String>,         // 业务员/采购员 EmpID
    pub dept_id: Option<String>,        // 部门 ID
    pub keyword: Option<String>,        // 关键词模糊搜索
}

pub async fn get_purchase_report(
    State(_config): State<Config>,
    Json(params): Json<ReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = r#"
        SELECT CONVERT(varchar(7), PoDate, 120) as month,
               SUM(SumAmt) as total_amt,
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

    // P2-7：业务维度筛选 - 供应商/仓库/业务员
    if let Some(sid) = &params.supp_id {
        if !sid.is_empty() {
            sql.push_str(&format!(" AND SuppID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sid.clone()));
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            sql.push_str(&format!(" AND StkID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    if let Some(eid) = &params.emp_id {
        if !eid.is_empty() {
            sql.push_str(&format!(" AND EmpID = @p{}", pidx));
            query_params.push(Some(eid.clone()));
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
                "totalAmt": total_amt as f64,
                "orderCount": order_count
            })
        })
        .collect();

    let mut grand_total: f64 = 0.0;
    let mut grand_count: i32 = 0;
    for item in &items {
        // P0 修复：金额字段改用 as_f64() 反序列化，保留小数精度
        if let Some(amt) = item.get("totalAmt").and_then(|v| v.as_f64()) {
            grand_total += amt;
        }
        if let Some(cnt) = item.get("orderCount").and_then(|v| v.as_i64()) {
            grand_count += cnt as i32;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalAmt": grand_total as f64,
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

    // 注意：tSal_Inv 表本身没有 CostAmt 字段（成本金额），需从 tSal_InvDetail 计算 SUM(Qty * APrice)
    // tSal_InvDetail 字段：Qty, APrice（成本价），Price（销售价），Amt（销售金额）
    let mut sql = r#"
        SELECT CONVERT(varchar(7), i.SIDate, 120) as month,
               SUM(ISNULL(i.SumAmt, 0)) as total_amt,
               ISNULL(SUM(ISNULL(d.Qty, 0) * ISNULL(d.APrice, 0)), 0) as cost_amt
        FROM tSal_Inv i
        LEFT JOIN tSal_InvDetail d ON i.SIID = d.SIID
        WHERE i.State <> 'D'"#
        .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sql.push_str(&format!(" AND i.SIDate >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sql.push_str(&format!(" AND i.SIDate <= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }

    // P2-7：业务维度筛选 - 客户
    if let Some(cid) = &params.cust_id {
        if !cid.is_empty() {
            sql.push_str(&format!(" AND i.CustID = @p{}", pidx));
            query_params.push(Some(cid.clone()));
        }
    }

    sql.push_str(" GROUP BY CONVERT(varchar(7), i.SIDate, 120) ORDER BY month");

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
                "totalAmt": total_amt as f64,
                "costAmt": cost_amt as f64,
                "profit": profit as f64
            })
        })
        .collect();

    let mut grand_sales: f64 = 0.0;
    let mut grand_cost: f64 = 0.0;
    for item in &items {
        // P0 修复：金额字段改用 as_f64() 反序列化，保留小数精度
        if let Some(amt) = item.get("totalAmt").and_then(|v| v.as_f64()) {
            grand_sales += amt;
        }
        if let Some(amt) = item.get("costAmt").and_then(|v| v.as_f64()) {
            grand_cost += amt;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalAmt": grand_sales as f64,
            "costAmt": grand_cost as f64,
            "profit": (grand_sales - grand_cost) as f64
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
               SUM(SumAmt) as purchase_amt,
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

    // P2-7：业务维度筛选 - 供应商
    if let Some(sid) = &params.supp_id {
        if !sid.is_empty() {
            purchase_sql.push_str(&format!(" AND SuppID = @p{}", pidx));
            purchase_params.push(Some(sid.clone()));
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
        SELECT CONVERT(varchar(7), SIDate, 120) as month,
               SUM(SumAmt) as sales_amt,
               SUM(CostAmt) as cost_amt
        FROM tSal_Inv
        WHERE State <> 'D'"#
        .to_string();

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sales_sql.push_str(&format!(" AND SIDate >= @p{}", sidx));
            sidx += 1;
            sales_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sales_sql.push_str(&format!(" AND SIDate <= @p{}", sidx));
            sidx += 1;
            sales_params.push(Some(ed.clone()));
        }
    }

    // P2-7：业务维度筛选 - 客户
    if let Some(cid) = &params.cust_id {
        if !cid.is_empty() {
            sales_sql.push_str(&format!(" AND CustID = @p{}", sidx));
            sales_params.push(Some(cid.clone()));
        }
    }

    sales_sql.push_str(" GROUP BY CONVERT(varchar(7), SIDate, 120)");

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
            "purchaseAmt": p_amt as f64,
            "purchaseCount": p_cnt,
            "salesAmt": s_amt as f64,
            "costAmt": c_amt as f64,
            "profit": profit as f64
        }));
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "purchaseAmt": grand_purchase as f64,
            "salesAmt": grand_sales as f64,
            "costAmt": grand_cost as f64,
            "profit": (grand_sales - grand_cost) as f64
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}

pub async fn get_stock_report(
    State(_config): State<Config>,
    Json(params): Json<ReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // P0-2 修复：原函数签名缺 Json<ReportParams>，前端传任何筛选都被忽略
    // 现支持 warehouse_id / gdstype_id / brand_id 筛选，并增加金额维度 (Qty × AInPrice)
    // JOIN tBas_Goods 获取成本价 AInPrice，计算库存金额
    // 注意：tStk_Stock 用 GDSID（非 GDSNO）关联 tBas_Goods，原 SQL 用 GDSNO 导致 500 错误
    // 注意：tBas_Stock 主键是 StkID（不是 StockID），名称字段是 StkName（不是 StockName）
    let mut sql = r#"
        SELECT CONVERT(varchar(40), s.StkID) as StockID, s.StkName as StockName,
               SUM(ISNULL(d.Qty, 0)) as total_qty,
               SUM(ISNULL(d.Qty * ISNULL(g.AInPrice, 0), 0)) as total_amt,
               COUNT(DISTINCT d.GDSID) as goods_count
        FROM tBas_Stock s
        LEFT JOIN tStk_Stock d ON s.StkID = d.StkID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        WHERE s.Used <> 'N'"#
        .to_string();

    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    // 仓库筛选
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            sql.push_str(&format!(" AND s.StkID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    // 商品类型筛选
    if let Some(tid) = &params.gdstype_id {
        if !tid.is_empty() {
            sql.push_str(&format!(" AND g.GDSTypeID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(tid.clone()));
        }
    }
    // 品牌筛选
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            sql.push_str(&format!(" AND g.BrandID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(bid.clone()));
        }
    }
    // 关键词筛选（仓库名或商品名）
    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            sql.push_str(&format!(" AND (s.StockName LIKE @p{} OR g.GDSDesc LIKE @p{})", pidx, pidx + 1));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    sql.push_str(" GROUP BY s.StkID, s.StkName ORDER BY s.StkID");

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params
        .iter()
        .map(|v| v as &dyn tiberius::ToSql)
        .collect();

    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let total_qty = row_get_f64(&row, "total_qty");
            let total_amt = row_get_f64(&row, "total_amt");
            let goods_count = row.get::<i32, _>("goods_count").unwrap_or(0);
            // 使用 try_get_value 处理 GUID/字符串类型，避免 tiberius 解析错误
            let stock_id = try_get_value(&row, "StockID").as_str().map(|s| s.to_string()).unwrap_or_default();
            let stock_name = try_get_value(&row, "StockName").as_str().map(|s| s.to_string()).unwrap_or_default();
            serde_json::json!({
                "stockID": stock_id,
                "stockName": stock_name,
                "totalQty": total_qty as f64,
                "totalAmt": total_amt as f64,
                "goodsCount": goods_count
            })
        })
        .collect();

    let mut grand_qty: f64 = 0.0;
    let mut grand_amt: f64 = 0.0;
    let mut grand_goods: i32 = 0;
    for item in &items {
        if let Some(qty) = item.get("totalQty").and_then(|v| v.as_f64()) {
            grand_qty += qty;
        }
        if let Some(amt) = item.get("totalAmt").and_then(|v| v.as_f64()) {
            grand_amt += amt;
        }
        if let Some(cnt) = item.get("goodsCount").and_then(|v| v.as_i64()) {
            grand_goods += cnt as i32;
        }
    }

    let data = serde_json::json!({
        "items": items,
        "summary": {
            "totalQty": grand_qty as f64,
            "totalAmt": grand_amt as f64,
            "goodsCount": grand_goods
        }
    });

    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 第五梯队：销售毛利分析（按商品/客户/业务员三个维度）
// 数据源：tStk_IO (Kind=SD/SI 已审核) + tStk_IODetail (含 APrice 成本价)
// ============================================================================

#[derive(Deserialize)]
pub struct ProfitAnalysisParams {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub dim: Option<String>, // goods | cust | emp
    pub top_n: Option<i32>,  // 限制 TopN 默认 50
}

pub async fn get_profit_analysis(
    State(_config): State<Config>,
    Json(params): Json<ProfitAnalysisParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let dim = params.dim.as_deref().unwrap_or("goods");
    let top_n = params.top_n.unwrap_or(50);

    let (select_dim, join_clause, group_dim, order_dim) = match dim {
        "cust" => (
            "ISNULL(CONVERT(varchar(40), c.CustID),'') AS DimID, ISNULL(c.CustName,'(未填)') AS DimName",
            "LEFT JOIN tBas_Cust c ON m.CustID = c.CustID",
            "c.CustID, c.CustName",
            "c.CustName",
        ),
        "emp" => (
            "ISNULL(CAST(m.EmpID AS varchar(40)),'') AS DimID, \
             ISNULL((SELECT TOP 1 e.EmpName FROM tBas_Emp e WHERE e.EmpID = m.EmpID),'(未填)') AS DimName",
            "",
            "m.EmpID",
            "(SELECT TOP 1 e.EmpName FROM tBas_Emp e WHERE e.EmpID = m.EmpID)",
        ),
        _ => ( // goods
            "ISNULL(CONVERT(varchar(40), g.GDSID),'') AS DimID, ISNULL(g.GDSDesc,'(未填)') AS DimName",
            "LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID",
            "g.GDSID, g.GDSDesc",
            "g.GDSDesc",
        ),
    };

    // 改写：把 TOP N 嵌入正确位置（SQL Server 用 SELECT TOP N ...）
    let mut sql = format!(
        "SELECT TOP {} {} , \
         SUM(CASE WHEN m.Kind = 'SR' THEN -ISNULL(d.StdQty, d.Qty) ELSE ISNULL(d.StdQty, d.Qty) END) AS SaleQty, \
         SUM(CASE WHEN m.Kind = 'SR' THEN -ISNULL(d.Amt, 0) ELSE ISNULL(d.Amt, 0) END) AS SaleAmt, \
         SUM(CASE WHEN m.Kind = 'SR' THEN -ISNULL(d.StdQty, d.Qty) * ISNULL(NULLIF(d.APrice, 0), d.Price) \
                  ELSE ISNULL(d.StdQty, d.Qty) * ISNULL(NULLIF(d.APrice, 0), d.Price) END) AS CostAmt, \
         COUNT(DISTINCT m.IOID) AS DocCount \
         FROM tStk_IO m \
         INNER JOIN tStk_IODetail d ON m.IOID = d.IOID \
         {} \
         WHERE m.Kind IN ('SD', 'SI', 'POS', 'SR') AND m.State IN ('S', 'Y')",
        top_n, select_dim, join_clause
    );
    let mut qparams: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;
    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            sql.push_str(&format!(" AND m.IoDate >= @p{}", pidx));
            qparams.push(Some(sd.clone()));
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            sql.push_str(&format!(" AND m.IoDate <= @p{}", pidx));
            qparams.push(Some(ed.clone()));
        }
    }
    sql.push_str(&format!(" GROUP BY {} ORDER BY SaleAmt DESC", group_dim));
    let _ = order_dim; // suppress unused warning
    let param_refs: Vec<&dyn tiberius::ToSql> = qparams.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut grand_qty = 0.0;
    let mut grand_sale = 0.0;
    let mut grand_cost = 0.0;
    for row in &rows {
        let qty = row_get_f64(row, "SaleQty");
        let sale = row_get_f64(row, "SaleAmt");
        let cost = row_get_f64(row, "CostAmt");
        let profit = sale - cost;
        let margin = if sale > 0.0001 { (profit / sale) * 100.0 } else { 0.0 };
        grand_qty += qty;
        grand_sale += sale;
        grand_cost += cost;
        items.push(serde_json::json!({
            "dimId": try_get_value(row, "DimID").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "dimName": try_get_value(row, "DimName").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "saleQty": qty as f64,
            "saleAmt": sale as f64,
            "costAmt": cost as f64,
            "profit": profit as f64,
            "margin": format!("{:.2}", margin),
            "docCount": row.get::<i32, _>("DocCount").unwrap_or(0)
        }));
    }
    let grand_profit = grand_sale - grand_cost;
    let grand_margin = if grand_sale > 0.0001 { (grand_profit / grand_sale) * 100.0 } else { 0.0 };
    let data = serde_json::json!({
        "dim": dim,
        "items": items,
        "summary": {
            "saleQty": grand_qty as f64,
            "saleAmt": grand_sale as f64,
            "costAmt": grand_cost as f64,
            "profit": grand_profit as f64,
            "margin": format!("{:.2}", grand_margin)
        }
    });
    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 第五梯队：应收账款账龄分析（按客户 + 4 桶：0-30/31-60/61-90/90+）
// 数据源：派生 AR（tStk_IO 销售/退货单据 + tFin_Receipt 收款）—— FIFO 冲抵
// 原理：按 IoDate 升序累计销售额，已收金额按客户汇总冲抵最早单据，
//       跨切点的单据部分结清，未结部分按 IoDate 归入对应账龄桶。
// ============================================================================

#[derive(Deserialize)]
pub struct AgeAnalysisParams {
    pub as_of_date: Option<String>,  // 截止日期（默认今天）
}

pub async fn get_receivable_aging(
    State(_config): State<Config>,
    Json(params): Json<AgeAnalysisParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let as_of = params.as_of_date.unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let sql = r#"
        WITH Sales AS (
            SELECT io.CustID, ISNULL(c.CustName,'(未填)') AS CustName, io.IOID, io.IoDate,
                CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt
                     WHEN io.Kind = 'SR' THEN -io.SumAmt ELSE 0 END AS Amt
            FROM tStk_IO io
            LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
            WHERE io.State IN ('S','Y') AND io.CustID IS NOT NULL AND io.Kind IN ('SD','SI','POS','SR')
        ),
        Receipts AS (
            SELECT CustID, ISNULL(SUM(RecAmt), 0) AS ReceivedAmt
            FROM tFin_Receipt WHERE State IN ('S','Y') AND CustID IS NOT NULL
            GROUP BY CustID
        ),
        Running AS (
            SELECT s.CustID, s.CustName, s.IoDate, s.Amt,
                ISNULL(r.ReceivedAmt, 0) AS ReceivedAmt,
                SUM(s.Amt) OVER (PARTITION BY s.CustID ORDER BY s.IoDate, s.IOID) AS RunTotal
            FROM Sales s LEFT JOIN Receipts r ON r.CustID = s.CustID
        )
        SELECT
            CONVERT(varchar(40), CustID) AS DimID, MAX(CustName) AS DimName,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 0 AND 30 THEN
                CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END ELSE 0 END) AS B0_30,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 31 AND 60 THEN
                CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END ELSE 0 END) AS B31_60,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 61 AND 90 THEN
                CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END ELSE 0 END) AS B61_90,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) > 90 THEN
                CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END ELSE 0 END) AS B90Plus,
            SUM(CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END) AS Total
        FROM Running
        GROUP BY CustID
        HAVING SUM(CASE WHEN RunTotal <= ReceivedAmt THEN 0
                        WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                        ELSE Amt END) > 0.0001
        ORDER BY Total DESC"#;
    let p1: &dyn tiberius::ToSql = &as_of;
    let stream = conn.query(sql, &[p1]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut s0 = 0.0; let mut s30 = 0.0; let mut s60 = 0.0; let mut s90 = 0.0; let mut st = 0.0;
    for row in &rows {
        let b0 = row_get_f64(row, "B0_30");
        let b30 = row_get_f64(row, "B31_60");
        let b60 = row_get_f64(row, "B61_90");
        let b90 = row_get_f64(row, "B90Plus");
        let total = row_get_f64(row, "Total");
        s0 += b0; s30 += b30; s60 += b60; s90 += b90; st += total;
        items.push(serde_json::json!({
            "dimId": try_get_value(row, "DimID").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "dimName": try_get_value(row, "DimName").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "b0_30": b0 as f64,
            "b31_60": b30 as f64,
            "b61_90": b60 as f64,
            "b90Plus": b90 as f64,
            "total": total as f64
        }));
    }
    let data = serde_json::json!({
        "asOfDate": as_of,
        "kind": "receivable",
        "items": items,
        "summary": {
            "b0_30": s0 as f64,
            "b31_60": s30 as f64,
            "b61_90": s60 as f64,
            "b90Plus": s90 as f64,
            "total": st as f64
        }
    });
    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 第五梯队：应付账款账龄分析（按供应商 + 4 桶）
// 数据源：派生 AP（tStk_IO 采购/退货单据 + tFin_Payment 付款）—— FIFO 冲抵
// ============================================================================

pub async fn get_payable_aging(
    State(_config): State<Config>,
    Json(params): Json<AgeAnalysisParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let as_of = params.as_of_date.unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());

    let sql = r#"
        WITH Purchases AS (
            SELECT io.SuppID, ISNULL(s.SuppName,'(未填)') AS SuppName, io.IOID, io.IoDate,
                CASE WHEN io.Kind = 'PD' THEN io.SumAmt
                     WHEN io.Kind = 'PR' THEN -io.SumAmt ELSE 0 END AS Amt
            FROM tStk_IO io
            LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
            WHERE io.State IN ('S','Y') AND io.SuppID IS NOT NULL AND io.Kind IN ('PD','PR')
        ),
        Payments AS (
            SELECT SuppID, ISNULL(SUM(PayAmt), 0) AS PaidAmt
            FROM tFin_Payment WHERE State IN ('S','Y') AND SuppID IS NOT NULL
            GROUP BY SuppID
        ),
        Running AS (
            SELECT p.SuppID, p.SuppName, p.IoDate, p.Amt,
                ISNULL(pm.PaidAmt, 0) AS PaidAmt,
                SUM(p.Amt) OVER (PARTITION BY p.SuppID ORDER BY p.IoDate, p.IOID) AS RunTotal
            FROM Purchases p LEFT JOIN Payments pm ON pm.SuppID = p.SuppID
        )
        SELECT
            CONVERT(varchar(40), SuppID) AS DimID, MAX(SuppName) AS DimName,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 0 AND 30 THEN
                CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END ELSE 0 END) AS B0_30,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 31 AND 60 THEN
                CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END ELSE 0 END) AS B31_60,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) BETWEEN 61 AND 90 THEN
                CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END ELSE 0 END) AS B61_90,
            SUM(CASE WHEN DATEDIFF(DAY, IoDate, @p1) > 90 THEN
                CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END ELSE 0 END) AS B90Plus,
            SUM(CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END) AS Total
        FROM Running
        GROUP BY SuppID
        HAVING SUM(CASE WHEN RunTotal <= PaidAmt THEN 0
                        WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                        ELSE Amt END) > 0.0001
        ORDER BY Total DESC"#;
    let p1: &dyn tiberius::ToSql = &as_of;
    let stream = conn.query(sql, &[p1]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut s0 = 0.0; let mut s30 = 0.0; let mut s60 = 0.0; let mut s90 = 0.0; let mut st = 0.0;
    for row in &rows {
        let b0 = row_get_f64(row, "B0_30");
        let b30 = row_get_f64(row, "B31_60");
        let b60 = row_get_f64(row, "B61_90");
        let b90 = row_get_f64(row, "B90Plus");
        let total = row_get_f64(row, "Total");
        s0 += b0; s30 += b30; s60 += b60; s90 += b90; st += total;
        items.push(serde_json::json!({
            "dimId": try_get_value(row, "DimID").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "dimName": try_get_value(row, "DimName").as_str().map(|s| s.to_string()).unwrap_or_default(),
            "b0_30": b0 as f64,
            "b31_60": b30 as f64,
            "b61_90": b60 as f64,
            "b90Plus": b90 as f64,
            "total": total as f64
        }));
    }
    let data = serde_json::json!({
        "asOfDate": as_of,
        "kind": "payable",
        "items": items,
        "summary": {
            "b0_30": s0 as f64,
            "b31_60": s30 as f64,
            "b61_90": s60 as f64,
            "b90Plus": s90 as f64,
            "total": st as f64
        }
    });
    Ok(Json(ApiResponse::ok(data)))
}

// ============================================================================
// 第六梯队：库存周转率分析（按商品 + 分类）
// 计算公式：
//   期间出库量 = tStk_IO Kind in (SD/SI/DBO/OTO/O) 在 [start,end] 内的 StdQty 之和
//   期间入库量 = tStk_IO Kind in (RI/PD/SR/DBI/OTI) 在 [start,end] 内的 StdQty 之和
//   期末库存   = tStk_Stock.Qty 累计到 end（按 GDSID 汇总）
//   期初库存   = 期末库存 - 期间入库量 + 期间出库量
//   平均库存   = (期初库存 + 期末库存) / 2
//   周转率     = 期间出库量 / 平均库存（平均库存 <= 0 视为 ∞）
//   周转天数   = 期间天数 / 周转率
// ============================================================================

#[derive(Deserialize)]
pub struct StockTurnoverParams {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub top_n: Option<i32>,
}

pub async fn get_stock_turnover(
    State(_config): State<Config>,
    Json(params): Json<StockTurnoverParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let top_n = params.top_n.unwrap_or(50);
    // 默认最近 90 天
    let end_date = params.end_date.clone()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let start_date = params.start_date.clone().unwrap_or_else(|| {
        let n = chrono::Local::now() - chrono::Duration::days(90);
        n.format("%Y-%m-%d").to_string()
    });

    // 1) 按 GDSID 聚合 tStk_Stock 期末库存
    // 注意：GDSID 是 uniqueidentifier 类型，需 CONVERT 为 varchar 才能用 tiberius 读取
    let end_stock_sql = "SELECT CONVERT(varchar(40), GDSID) AS GDSID, SUM(ISNULL(Qty, 0)) AS EndQty FROM tStk_Stock GROUP BY GDSID";
    let s1 = conn.query(end_stock_sql, &[]).await?;
    let end_stock_rows: Vec<Row> = s1.into_first_result().await?;
    let mut end_stock: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for r in &end_stock_rows {
        let gid = try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default();
        let q = row_get_f64(r, "EndQty");
        if !gid.is_empty() { end_stock.insert(gid, q); }
    }

    // 2) 按 GDSID 聚合期间出库量（出库类 Kind: SD/SI/POS/PR/OTO/RI/ADJ/O/REQ/DBO）
    // 依据 doc_graph::kind_direction：TH 是调拨类（DIR_TRANSFER=0），不应计入出库
    let out_sql = "SELECT CONVERT(varchar(40), d.GDSID) AS GDSID, SUM(ISNULL(d.StdQty, d.Qty)) AS OutQty \
                   FROM tStk_IO m INNER JOIN tStk_IODetail d ON m.IOID = d.IOID \
                   WHERE m.Kind IN ('SD','SI','POS','PR','OTO','RI','ADJ','O','REQ','DBO') \
                     AND m.State IN ('S','Y') \
                     AND CONVERT(varchar(10), m.IoDate, 120) BETWEEN @p1 AND @p2 \
                   GROUP BY d.GDSID";
    let p_sd: &dyn tiberius::ToSql = &start_date;
    let p_ed: &dyn tiberius::ToSql = &end_date;
    let s2 = conn.query(out_sql, &[p_sd, p_ed]).await?;
    let out_rows: Vec<Row> = s2.into_first_result().await?;
    let mut out_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for r in &out_rows {
        let gid = try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default();
        let q = row_get_f64(r, "OutQty");
        if !gid.is_empty() { out_map.insert(gid, q); }
    }

    // 3) 按 GDSID 聚合期间入库量（入库类 Kind: PD/SR/OTI/DBI）
    // 依据 doc_graph::kind_direction：RI/ADJ 是出库类（-1），不应计入入库
    let in_sql = "SELECT CONVERT(varchar(40), d.GDSID) AS GDSID, SUM(ISNULL(d.StdQty, d.Qty)) AS InQty \
                  FROM tStk_IO m INNER JOIN tStk_IODetail d ON m.IOID = d.IOID \
                  WHERE m.Kind IN ('PD','SR','OTI','DBI') \
                    AND m.State IN ('S','Y') \
                    AND CONVERT(varchar(10), m.IoDate, 120) BETWEEN @p1 AND @p2 \
                  GROUP BY d.GDSID";
    let s3 = conn.query(in_sql, &[p_sd, p_ed]).await?;
    let in_rows: Vec<Row> = s3.into_first_result().await?;
    let mut in_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for r in &in_rows {
        let gid = try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default();
        let q = row_get_f64(r, "InQty");
        if !gid.is_empty() { in_map.insert(gid, q); }
    }

    // 4) 拉取商品基础信息（JOIN 关联表获取 GDSTypeName/BrandName/UnitName）
    let gds_sql = "SELECT TOP 5000 CONVERT(varchar(40), g.GDSID) AS GDSID, g.GDSNO, g.GDSDesc, \
                   ISNULL(t.GDSTypeName,'') AS GDSTypeName, ISNULL(b.BrandName,'') AS BrandName, \
                   ISNULL(u.UnitName,'') AS UnitName, \
                   ISNULL(g.SPrice,0) AS SPrice, ISNULL(g.AInPrice,0) AS AInPrice \
                   FROM tBas_Goods g \
                   LEFT JOIN tBas_GDSType t ON g.GDSTypeID = t.GDSTypeID \
                   LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID \
                   LEFT JOIN tBas_Unit u ON g.UnitNO = u.UnitNO \
                   WHERE g.State <> 'D'";
    let s4 = conn.query(gds_sql, &[]).await?;
    let gds_rows: Vec<Row> = s4.into_first_result().await?;
    // 5) 聚合计算
    let mut items: Vec<serde_json::Value> = Vec::new();
    let mut grand_out = 0.0;
    let mut grand_end = 0.0;
    for r in &gds_rows {
        let gid = try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default();
        if gid.is_empty() { continue; }
        // 只统计有出库或库存的商品
        let out_q = *out_map.get(&gid).unwrap_or(&0.0);
        let in_q = *in_map.get(&gid).unwrap_or(&0.0);
        let end_q = *end_stock.get(&gid).unwrap_or(&0.0);
        if out_q <= 0.0001 && end_q <= 0.0001 { continue; }
        let begin_q = end_q - in_q + out_q;
        let avg = (begin_q + end_q) / 2.0;
        let turnover = if avg > 0.0001 { out_q / avg } else { 0.0 };
        // 期间天数
        let days = days_between(&start_date, &end_date).max(1);
        let turnover_days = if turnover > 0.0001 { (days as f64) / turnover } else { 0.0 };
        grand_out += out_q;
        grand_end += end_q;
        let s_price = row_get_f64(r, "SPrice");
        let a_in = row_get_f64(r, "AInPrice");
        items.push(serde_json::json!({
            "gdsId": gid,
            "gdsNo": r.get::<&str, _>("GDSNO").unwrap_or(""),
            "gdsName": r.get::<&str, _>("GDSDesc").unwrap_or(""),
            "gdsType": r.get::<&str, _>("GDSTypeName").unwrap_or(""),
            "brandName": r.get::<&str, _>("BrandName").unwrap_or(""),
            "unitName": r.get::<&str, _>("UnitName").unwrap_or(""),
            "outQty": out_q,
            "inQty": in_q,
            "beginQty": begin_q,
            "endQty": end_q,
            "avgQty": avg,
            "turnover": format!("{:.4}", turnover),
            "turnoverDays": format!("{:.2}", turnover_days),
            "sPrice": s_price,
            "aInPrice": a_in,
        }));
    }
    // 按出库量降序
    items.sort_by(|a, b| {
        let av = a.get("outQty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let bv = b.get("outQty").and_then(|v| v.as_f64()).unwrap_or(0.0);
        bv.partial_cmp(&av).unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(top_n as usize);
    let data = serde_json::json!({
        "startDate": start_date,
        "endDate": end_date,
        "items": items,
        "summary": {
            "itemCount": items.len(),
            "totalOutQty": grand_out,
            "totalEndQty": grand_end,
        }
    });
    Ok(Json(ApiResponse::ok(data)))
}

fn days_between(s: &str, e: &str) -> i64 {
    let parse = |t: &str| chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").ok();
    match (parse(s), parse(e)) {
        (Some(a), Some(b)) => (b - a).num_days(),
        _ => 0,
    }
}

// ============================================================================
// 第七梯队：预警中心聚合接口
// 聚合 4 类预警：
//   1) 低库存（tStk_Stock.Qty < tBas_Goods.BttomStkQty）
//   2) 超期应收（派生 AR：tStk_IO 销售 + tFin_Receipt 收款，FIFO 未结且超期）
//   3) 超期应付（派生 AP：tStk_IO 采购 + tFin_Payment 付款，FIFO 未结且超期）
//   4) 零价格商品（tBas_Goods.SPrice=0 或 AInPrice=0 且 State<>'D'）
// ============================================================================

#[derive(Deserialize)]
pub struct AlertCenterParams {
    pub over_days: Option<i32>,    // 应收/应付超期阈值（默认 30）
    pub low_stock: Option<bool>,   // 是否包含低库存（默认 true）
    pub over_recv: Option<bool>,   // 是否包含超期应收
    pub over_pay: Option<bool>,    // 是否包含超期应付
    pub zero_price: Option<bool>,  // 是否包含零价格商品
    pub top_n: Option<i32>,        // 每类 TopN
    // P1-3 修复：统一超期天数口径，与 ReceivableAging/PayableAging 的 as_of_date 对齐
    // 不传时默认今天（GETDATE），保持向后兼容
    pub as_of_date: Option<String>,
}

pub async fn get_alert_center(
    State(_config): State<Config>,
    Json(params): Json<AlertCenterParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let over_days = params.over_days.unwrap_or(30);
    let top_n = params.top_n.unwrap_or(20) as usize;
    let do_low = params.low_stock.unwrap_or(true);
    let do_recv = params.over_recv.unwrap_or(true);
    let do_pay = params.over_pay.unwrap_or(true);
    let do_zero = params.zero_price.unwrap_or(true);
    // P1-3：超期天数基准日，默认今天；前端可传 as_of_date 指定历史日期，与账龄分析口径一致
    let as_of_date = params.as_of_date.as_deref().unwrap_or("").trim();
    let as_of_expr = if as_of_date.is_empty() {
        "GETDATE()".to_string()
    } else {
        // 直接拼接 ISO 日期字符串（YYYY-MM-DD），SQL Server 可隐式转换为 DATETIME
        format!("CAST('{}' AS DATETIME)", as_of_date.replace('\'', ""))
    };

    let mut groups: Vec<serde_json::Value> = Vec::new();
    let mut total_count = 0usize;

    // 1) 低库存
    if do_low {
        let sql = r#"
            SELECT TOP 1000 CONVERT(varchar(40), g.GDSID) AS GDSID, g.GDSNO, g.GDSDesc,
                ISNULL(b.BrandName,'') AS BrandName, ISNULL(u.UnitName,'') AS UnitName,
                ISNULL(SUM(s.Qty), 0) AS Qty,
                ISNULL(g.BttomStkQty, 0) AS Bottom
            FROM tBas_Goods g
            LEFT JOIN tStk_Stock s ON g.GDSID = s.GDSID
            LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
            LEFT JOIN tBas_Unit u ON g.UnitNO = u.UnitNO
            WHERE g.State <> 'D' AND ISNULL(g.BttomStkQty, 0) > 0
            GROUP BY g.GDSID, g.GDSNO, g.GDSDesc, b.BrandName, u.UnitName, g.BttomStkQty
            HAVING ISNULL(SUM(s.Qty), 0) < ISNULL(g.BttomStkQty, 0)"#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        let mut items: Vec<serde_json::Value> = Vec::new();
        for r in rows.iter().take(top_n) {
            let qty = row_get_f64(r, "Qty");
            let bottom = row_get_f64(r, "Bottom");
            let shortage = bottom - qty;
            items.push(serde_json::json!({
                "id": try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default(),
                "code": r.get::<&str, _>("GDSNO").unwrap_or(""),
                "name": r.get::<&str, _>("GDSDesc").unwrap_or(""),
                "brandName": r.get::<&str, _>("BrandName").unwrap_or(""),
                "unitName": r.get::<&str, _>("UnitName").unwrap_or(""),
                "qty": qty as f64,
                "threshold": bottom as f64,
                "shortage": shortage as f64,
                "level": if shortage > bottom { "critical" } else { "warning" }
            }));
        }
        total_count += items.len();
        groups.push(serde_json::json!({
            "key": "low_stock",
            "title": "低库存预警",
            "icon": "Goods",
            "total": items.len(),
            "items": items,
            "link": "/report/stock-query"
        }));
    }

    // 2) 超期应收（派生 AR：tStk_IO 销售/退货 + tFin_Receipt 收款，FIFO 冲抵）
    if do_recv {
        let sql = format!(r#"
            WITH Sales AS (
                SELECT io.IOID, io.IONo, io.CustID, io.IoDate, io.SumAmt,
                    ISNULL(c.CustName,'(未填)') AS CustName,
                    CASE WHEN io.Kind IN ('SD','SI','POS') THEN io.SumAmt
                         WHEN io.Kind = 'SR' THEN -io.SumAmt ELSE 0 END AS Amt
                FROM tStk_IO io
                LEFT JOIN tBas_Cust c ON c.CustID = io.CustID
                WHERE io.State IN ('S','Y') AND io.CustID IS NOT NULL AND io.Kind IN ('SD','SI','POS','SR')
            ), Receipts AS (
                SELECT CustID, ISNULL(SUM(RecAmt), 0) AS ReceivedAmt
                FROM tFin_Receipt WHERE State IN ('S','Y') AND CustID IS NOT NULL GROUP BY CustID
            ), Running AS (
                SELECT s.IOID, s.IONo, s.CustName, s.IoDate, s.SumAmt, s.Amt,
                    ISNULL(r.ReceivedAmt, 0) AS ReceivedAmt,
                    SUM(s.Amt) OVER (PARTITION BY s.CustID ORDER BY s.IoDate, s.IOID) AS RunTotal
                FROM Sales s LEFT JOIN Receipts r ON r.CustID = s.CustID
            )
            SELECT TOP 1000 CONVERT(varchar(40), IOID) AS IOID, IONo, CustName, IoDate, SumAmt AS TotalAmt,
                CASE WHEN RunTotal <= ReceivedAmt THEN 0
                     WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                     ELSE Amt END AS RemainAmt,
                DATEDIFF(DAY, IoDate, {as_of}) AS OverDays
            FROM Running
            WHERE (CASE WHEN RunTotal <= ReceivedAmt THEN 0
                        WHEN RunTotal - Amt < ReceivedAmt THEN RunTotal - ReceivedAmt
                        ELSE Amt END) > 0.0001
              AND DATEDIFF(DAY, IoDate, {as_of}) > {od}
            ORDER BY OverDays DESC"#, as_of = as_of_expr, od = over_days);
        let stream = conn.query(&sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        let mut items: Vec<serde_json::Value> = Vec::new();
        for r in rows.iter().take(top_n) {
            let over_d: i32 = r.get::<i32, _>("OverDays").unwrap_or(0);
            let total = row_get_f64(r, "TotalAmt");
            let remain = row_get_f64(r, "RemainAmt");
            let level = if over_d > 90 { "critical" } else if over_d > 60 { "danger" } else { "warning" };
            items.push(serde_json::json!({
                "id": try_get_value(r, "IOID").as_str().map(|s| s.to_string()).unwrap_or_default(),
                "docNo": r.get::<&str, _>("IONo").unwrap_or(""),
                "partyName": r.get::<&str, _>("CustName").unwrap_or(""),
                "totalAmt": total as f64,
                "remainAmt": remain as f64,
                "overDays": over_d,
                "level": level
            }));
        }
        total_count += items.len();
        groups.push(serde_json::json!({
            "key": "over_recv",
            "title": "超期应收",
            "icon": "CreditCard",
            "total": items.len(),
            "items": items,
            "link": "/report/receivable-aging"
        }));
    }

    // 3) 超期应付（派生 AP：tStk_IO 采购/退货 + tFin_Payment 付款，FIFO 冲抵）
    if do_pay {
        let sql = format!(r#"
            WITH Purchases AS (
                SELECT io.IOID, io.IONo, io.SuppID, io.IoDate, io.SumAmt,
                    ISNULL(s.SuppName,'(未填)') AS SuppName,
                    CASE WHEN io.Kind = 'PD' THEN io.SumAmt
                         WHEN io.Kind = 'PR' THEN -io.SumAmt ELSE 0 END AS Amt
                FROM tStk_IO io
                LEFT JOIN tBas_Supp s ON s.SuppID = io.SuppID
                WHERE io.State IN ('S','Y') AND io.SuppID IS NOT NULL AND io.Kind IN ('PD','PR')
            ), Payments AS (
                SELECT SuppID, ISNULL(SUM(PayAmt), 0) AS PaidAmt
                FROM tFin_Payment WHERE State IN ('S','Y') AND SuppID IS NOT NULL GROUP BY SuppID
            ), Running AS (
                SELECT p.IOID, p.IONo, p.SuppName, p.IoDate, p.SumAmt, p.Amt,
                    ISNULL(pm.PaidAmt, 0) AS PaidAmt,
                    SUM(p.Amt) OVER (PARTITION BY p.SuppID ORDER BY p.IoDate, p.IOID) AS RunTotal
                FROM Purchases p LEFT JOIN Payments pm ON pm.SuppID = p.SuppID
            )
            SELECT TOP 1000 CONVERT(varchar(40), IOID) AS IOID, IONo, SuppName, IoDate, SumAmt AS TotalAmt,
                CASE WHEN RunTotal <= PaidAmt THEN 0
                     WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                     ELSE Amt END AS RemainAmt,
                DATEDIFF(DAY, IoDate, {as_of}) AS OverDays
            FROM Running
            WHERE (CASE WHEN RunTotal <= PaidAmt THEN 0
                        WHEN RunTotal - Amt < PaidAmt THEN RunTotal - PaidAmt
                        ELSE Amt END) > 0.0001
              AND DATEDIFF(DAY, IoDate, {as_of}) > {od}
            ORDER BY OverDays DESC"#, as_of = as_of_expr, od = over_days);
        let stream = conn.query(&sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        let mut items: Vec<serde_json::Value> = Vec::new();
        for r in rows.iter().take(top_n) {
            let over_d: i32 = r.get::<i32, _>("OverDays").unwrap_or(0);
            let total = row_get_f64(r, "TotalAmt");
            let remain = row_get_f64(r, "RemainAmt");
            let level = if over_d > 90 { "critical" } else if over_d > 60 { "danger" } else { "warning" };
            items.push(serde_json::json!({
                "id": try_get_value(r, "IOID").as_str().map(|s| s.to_string()).unwrap_or_default(),
                "docNo": r.get::<&str, _>("IONo").unwrap_or(""),
                "partyName": r.get::<&str, _>("SuppName").unwrap_or(""),
                "totalAmt": total as f64,
                "remainAmt": remain as f64,
                "overDays": over_d,
                "level": level
            }));
        }
        total_count += items.len();
        groups.push(serde_json::json!({
            "key": "over_pay",
            "title": "超期应付",
            "icon": "Wallet",
            "total": items.len(),
            "items": items,
            "link": "/report/payable-aging"
        }));
    }

    // 4) 零价格商品
    if do_zero {
        let sql = r#"
            SELECT TOP 1000 CONVERT(varchar(40), GDSID) AS GDSID, GDSNO, GDSDesc, ISNULL(SPrice,0) AS SPrice, ISNULL(AInPrice,0) AS AInPrice
            FROM tBas_Goods
            WHERE State <> 'D' AND (ISNULL(SPrice, 0) <= 0.0001 OR ISNULL(AInPrice, 0) <= 0.0001)
            ORDER BY GDSNO"#;
        let stream = conn.query(sql, &[]).await?;
        let rows: Vec<Row> = stream.into_first_result().await?;
        let mut items: Vec<serde_json::Value> = Vec::new();
        for r in rows.iter().take(top_n) {
            let s_price = row_get_f64(r, "SPrice");
            let a_in = row_get_f64(r, "AInPrice");
            let missing: Vec<&str> = if s_price <= 0.0001 && a_in <= 0.0001 { vec!["SPrice", "AInPrice"] }
                                       else if s_price <= 0.0001 { vec!["SPrice"] }
                                       else { vec!["AInPrice"] };
            items.push(serde_json::json!({
                "id": try_get_value(r, "GDSID").as_str().map(|s| s.to_string()).unwrap_or_default(),
                "code": r.get::<&str, _>("GDSNO").unwrap_or(""),
                "name": r.get::<&str, _>("GDSDesc").unwrap_or(""),
                "sPrice": s_price,
                "aInPrice": a_in,
                "missing": missing,
                "level": "warning"
            }));
        }
        total_count += items.len();
        groups.push(serde_json::json!({
            "key": "zero_price",
            "title": "零价格商品",
            "icon": "PriceTag",
            "total": items.len(),
            "items": items,
            "link": "/base/goods"
        }));
    }

    let data = serde_json::json!({
        "groups": groups,
        "totalCount": total_count,
        "overDays": over_days,
    });
    Ok(Json(ApiResponse::ok(data)))
}

/// P1-4 修复：销售任务汇总报表
///
/// 替代前端 N+1 查询模式（先拉 N 条任务，再逐个调 getSalesTaskRecords 共 N 次请求）。
/// 单次请求完成：1) 拉所有任务 2) 拉所有销售记录 3) Rust 端按 TaskID 聚合 actual
///
/// 同时修复 PValue 解析 bug：旧前端用 parseFloat(整个JSON字符串) 永远返回 NaN，actual 始终为 0。
#[derive(Deserialize)]
pub struct SalesTaskSummaryParams {
    pub month: Option<String>,      // YYYY-MM，按 EndDate 月份筛选
    pub state: Option<String>,      // 任务状态筛选
    pub stk_id: Option<String>,     // 仓库筛选（暂未在 SQL 中实现，前端按 PValue.StkID 过滤）
}

pub async fn get_sales_task_summary(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<SalesTaskSummaryParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let emp_id = claims.emp_id.clone();

    // 1) 拉取当前用户所有销售任务（最多 1000 条，避免极端数据量）
    // 注意：tSys_Parameters 表没有 State 字段，状态字段在 PValue JSON 中（默认 N=草稿）
    let task_sql = r#"SELECT TOP 1000 [ParametersID], [PName], [PValue], [EDate]
        FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_task' AND [EUser] = @p1
        ORDER BY [EDate] DESC"#;
    let task_stream = conn.query(task_sql, &[&emp_id.as_str()]).await?;
    let task_rows: Vec<Row> = task_stream.into_first_result().await?;

    // 2) 拉取当前用户所有销售记录（最多 50000 条，足够日常使用）
    let rec_sql = r#"SELECT TOP 50000 [PValue]
        FROM [tSys_Parameters]
        WHERE [PKind] = 'sales_record' AND [EUser] = @p1"#;
    let rec_stream = conn.query(rec_sql, &[&emp_id.as_str()]).await?;
    let rec_rows: Vec<Row> = rec_stream.into_first_result().await?;

    // 3) 在 Rust 端按 TaskID 聚合 SalesAmt
    //    PValue 格式: {"TaskID":"xxx","RecordDate":"2026-01-15","SalesAmt":500.0,"StkID":"..."}
    let mut actual_map: HashMap<String, f64> = HashMap::new();
    for r in rec_rows.iter() {
        let pval: &str = r.get::<&str, _>("PValue").unwrap_or("");
        if pval.is_empty() { continue; }
        // 用 serde_json 解析以正确取值（旧前端 parseFloat 整个 JSON 是 bug）
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(pval) {
            let tid = v.get("TaskID").and_then(|x| x.as_str()).unwrap_or("");
            let amt = v.get("SalesAmt").and_then(|x| x.as_f64()).unwrap_or(0.0);
            if !tid.is_empty() {
                *actual_map.entry(tid.to_string()).or_insert(0.0) += amt;
            }
        }
    }

    // 4) 组装任务列表
    //    PValue 格式: {"TaskName":"...","TargetAmt":1000,"StartDate":"2026-01-01","EndDate":"2026-12-31","StkID":"..."}
    let mut tasks: Vec<serde_json::Value> = Vec::new();
    for r in task_rows.iter() {
        let pid = r.get::<&str, _>("ParametersID").unwrap_or("").to_string();
        let pname = r.get::<&str, _>("PName").unwrap_or("").to_string();
        let edate = {
            let val = try_get_value(r, "EDate");
            match val {
                serde_json::Value::String(s) => s,
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            }
        };
        let pval: &str = r.get::<&str, _>("PValue").unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(pval).unwrap_or(serde_json::json!({}));

        let target = parsed.get("TargetAmt").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let start_date = parsed.get("StartDate").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let end_date = parsed.get("EndDate").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let stk_id = parsed.get("StkID").and_then(|x| x.as_str()).unwrap_or("").to_string();
        // 状态从 PValue JSON 中取，默认 'N'（草稿）
        let state = parsed.get("State").and_then(|x| x.as_str()).unwrap_or("N").to_string();

        // 月份过滤（按 EndDate 月份）
        if let Some(m) = &params.month {
            if !m.is_empty() && !end_date.starts_with(m) {
                continue;
            }
        }
        // 状态过滤
        if let Some(s) = &params.state {
            if !s.is_empty() && state != *s {
                continue;
            }
        }
        // 仓库过滤
        if let Some(sid) = &params.stk_id {
            if !sid.is_empty() && stk_id != *sid {
                continue;
            }
        }

        let actual = *actual_map.get(&pid).unwrap_or(&0.0);
        let rate = if target > 0.0 { (actual / target) * 100.0 } else { 0.0 };

        tasks.push(serde_json::json!({
            "ParametersID": pid,
            "PName": pname,
            "State": state,
            "EDate": edate,
            "target": target,
            "actual": actual,
            "rate": rate,
            "startDate": start_date,
            "deadline": end_date,
            "stkId": stk_id,
        }));
    }

    // 5) 计算汇总
    let task_count = tasks.len();
    let total_target: f64 = tasks.iter().map(|t| t.get("target").and_then(|v| v.as_f64()).unwrap_or(0.0)).sum();
    let total_actual: f64 = tasks.iter().map(|t| t.get("actual").and_then(|v| v.as_f64()).unwrap_or(0.0)).sum();
    let avg_rate = if total_target > 0.0 { (total_actual / total_target) * 100.0 } else { 0.0 };

    let data = serde_json::json!({
        "list": tasks,
        "summary": {
            "taskCount": task_count,
            "totalTarget": total_target,
            "totalActual": total_actual,
            "avgRate": avg_rate,
        },
    });
    Ok(Json(ApiResponse::ok(data)))
}
