//! 提成 handler：重算销售单提成 + 提成报表（汇总/明细）
//!
//! 参考 88 文件 commission report 实现：
//! - 汇总报表：按门店+员工分组，聚合销售额和提成
//! - 明细报表：按门店+员工+品牌+提成比例分组
//! - 数据源：tSal_Inv + tSal_InvDetail（State IN ('S','Y')）

use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::services::commission_service;
use crate::utils::{ApiResponse, row_get_f64};
use axum::body::Body;
use axum::extract::{Json, State};
use axum::response::Response;
use rust_xlsxwriter::{Format, Workbook};
use serde::Deserialize;
use tiberius::{Row, ToSql};

// =====================================================================
// 重算销售单提成
// =====================================================================

#[derive(Deserialize)]
pub struct RecalcParams {
    pub siid: String,
}

/// POST /api/commission/recalc-invoice
/// 重算指定销售单的提成（前端保存销售单后调用）
pub async fn recalc_invoice(
    State(_config): State<Config>,
    Json(params): Json<RecalcParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    if params.siid.is_empty() {
        return Ok(Json(ApiResponse::err("销售单 ID 不能为空")));
    }
    let mut conn = get_pool().get().await?;
    match commission_service::recalc_invoice_commission(&mut conn, &params.siid).await {
        Ok(updated) => Ok(Json(ApiResponse::ok(serde_json::json!({
            "siid": params.siid,
            "updated": updated,
        })))),
        Err(e) => Ok(Json(ApiResponse::err(&e))),
    }
}

// =====================================================================
// 提成报表（汇总 + 明细）
// =====================================================================

#[derive(Deserialize)]
pub struct CommissionReportParams {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub warehouse_id: Option<String>,      // 门店 ID
    pub emp_id: Option<String>,            // 员工 ID
    pub brand_id: Option<String>,          // 品牌 ID
    pub brand_level: Option<String>,       // 品牌分类 A/B/C/D
    pub doc_no: Option<String>,            // 单据号模糊查询
    pub sales_person_name: Option<String>, // 销售员姓名模糊查询
    pub product_keyword: Option<String>,   // 商品编码/名称模糊查询
    pub category_id: Option<String>,       // 商品分类 ID
    // 分页参数（仅 commission-detail 接口使用，默认不分页）
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// POST /api/report/commission-summary
/// 提成汇总报表：按门店+员工分组
/// 筛选：日期/门店/员工/品牌/品牌等级
pub async fn get_commission_summary(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = r#"
        SELECT
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(p.PName, '') AS TemplateName,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS CommissionAmount,
            COUNT(DISTINCT i.SIID) AS OrderCount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        LEFT JOIN tSys_Parameters p ON s.CommissionTemplateID = p.ParametersID
        WHERE i.State IN ('S', 'Y')
    "#
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
            sql.push_str(&format!(" AND i.SIDate < DATEADD(day, 1, @p{})", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            sql.push_str(&format!(
                " AND i.StkID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    if let Some(eid) = &params.emp_id {
        if !eid.is_empty() {
            sql.push_str(&format!(
                " AND i.EmpID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(eid.clone()));
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            sql.push_str(&format!(
                " AND g.BrandID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(bid.clone()));
        }
    }
    if let Some(bl) = &params.brand_level {
        if !bl.is_empty() {
            sql.push_str(&format!(" AND b.Level = @p{}", pidx));
            query_params.push(Some(bl.clone()));
        }
    }

    sql.push_str(" GROUP BY i.EmpID, e.EmpName, i.StkID, s.StkName, p.PName");
    sql.push_str(" ORDER BY e.EmpName, s.StkName");

    let param_refs: Vec<&dyn ToSql> = query_params.iter().map(|v| v as &dyn ToSql).collect();
    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;

    let mut items = Vec::new();
    let mut total_sales = 0.0;
    let mut total_comm = 0.0;

    for row in &rows {
        let sales = row_get_f64(row, "SalesAmount");
        let comm = row_get_f64(row, "CommissionAmount");
        total_sales += sales;
        total_comm += comm;

        items.push(serde_json::json!({
            "empId": row.get::<&str, _>("EmpID").unwrap_or(""),
            "empName": row.get::<&str, _>("EmpName").unwrap_or(""),
            "stkId": row.get::<&str, _>("StkID").unwrap_or(""),
            "stkName": row.get::<&str, _>("StkName").unwrap_or(""),
            "templateName": row.get::<&str, _>("TemplateName").unwrap_or(""),
            "salesAmount": sales,
            "commissionAmount": comm,
            "orderCount": row.get::<i32, _>("OrderCount").unwrap_or(0),
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "items": items,
        "summary": {
            "totalSales": total_sales,
            "totalCommission": total_comm,
            "totalOrders": items.iter().map(|i| i["orderCount"].as_i64().unwrap_or(0)).sum::<i64>(),
        },
    }))))
}

/// POST /api/report/commission-detail
/// 提成明细报表：逐行明细（单号/商品/品牌/数量/金额/提成）
/// ★ 对齐 88 项目 CommissionDetails.vue：返回每条销售明细行
/// 筛选：日期/门店/员工/品牌/品牌分类/单据号/销售员/商品关键词/分类
pub async fn get_commission_detail(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // ★ 逐行明细：不分组，返回每条销售明细行
    let mut sql = r#"
        SELECT
            i.SINO AS OrderNo,
            CONVERT(varchar(10), i.SIDate, 120) AS OrderDate,
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            CONVERT(varchar(40), d.GDSID) AS GDSID,
            ISNULL(g.GDSNO, '') AS GDSNO,
            ISNULL(g.GDSDesc, '') AS GDSDesc,
            ISNULL(un.UnitName, '') AS UnitName,
            CONVERT(varchar(40), g.BrandID) AS BrandID,
            ISNULL(b.BrandName, '') AS BrandName,
            ISNULL(b.Level, '') AS BrandLevel,
            CONVERT(varchar(40), gt.GDSTypeID) AS CategoryID,
            ISNULL(gt.GDSTypeName, '') AS CategoryName,
            ISNULL(d.Qty, 0) AS Qty,
            ISNULL(d.Price, 0) AS Price,
            ISNULL(d.Amt, 0) AS Amt,
            ISNULL(g.AInPrice, 0) AS CostPrice,
            ISNULL(g.AInPrice, 0) * ISNULL(d.Qty, 0) AS CostAmount,
            ISNULL(d.Amt, 0) - ISNULL(g.AInPrice, 0) * ISNULL(d.Qty, 0) AS Profit,
            ISNULL(d.CommissionRate, 0) AS CommissionRate,
            ISNULL(d.CommissionType, 0) AS CommissionType,
            ISNULL(d.Commission, 0) AS Commission
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
        LEFT JOIN tBas_Unit un ON g.UnitNO = un.UnitNO
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        WHERE i.State IN ('S', 'Y')
    "#
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
            sql.push_str(&format!(" AND i.SIDate < DATEADD(day, 1, @p{})", pidx));
            pidx += 1;
            query_params.push(Some(ed.clone()));
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            sql.push_str(&format!(
                " AND i.StkID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(wid.clone()));
        }
    }
    if let Some(eid) = &params.emp_id {
        if !eid.is_empty() {
            sql.push_str(&format!(
                " AND i.EmpID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(eid.clone()));
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            sql.push_str(&format!(
                " AND g.BrandID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            pidx += 1;
            query_params.push(Some(bid.clone()));
        }
    }
    if let Some(bl) = &params.brand_level {
        if !bl.is_empty() {
            sql.push_str(&format!(" AND b.Level = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(bl.clone()));
        }
    }
    if let Some(dn) = &params.doc_no {
        if !dn.is_empty() {
            sql.push_str(&format!(" AND i.SINO LIKE '%' + @p{} + '%'", pidx));
            pidx += 1;
            query_params.push(Some(dn.clone()));
        }
    }
    if let Some(spn) = &params.sales_person_name {
        if !spn.is_empty() {
            sql.push_str(&format!(" AND e.EmpName LIKE '%' + @p{} + '%'", pidx));
            pidx += 1;
            query_params.push(Some(spn.clone()));
        }
    }
    if let Some(pk) = &params.product_keyword {
        if !pk.is_empty() {
            sql.push_str(&format!(
                " AND (g.GDSNO LIKE '%' + @p{} + '%' OR g.GDSDesc LIKE '%' + @p{} + '%')",
                pidx, pidx
            ));
            pidx += 1;
            query_params.push(Some(pk.clone()));
        }
    }
    if let Some(cid) = &params.category_id {
        if !cid.is_empty() {
            sql.push_str(&format!(
                " AND g.GDSTypeID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(Some(cid.clone()));
        }
    }

    sql.push_str(" ORDER BY i.SIDate DESC, i.SINO, d.RowNO");

    // 判断是否分页（对齐 88 项目 GetCommissionDetails 分页支持）
    let page = params.page.unwrap_or(0);
    let page_size = params.page_size.unwrap_or(0);
    let is_paged = page > 0 && page_size > 0;

    // 分页时的合计查询（基于全量数据，不随分页变化）
    let mut total_count: i64 = 0;
    let mut total_sales: f64 = 0.0;
    let mut total_comm: f64 = 0.0;
    let mut total_profit: f64 = 0.0;

    if is_paged {
        // 执行 COUNT + SUM 聚合查询（复用 WHERE 条件）
        let agg_sql = format!(
            r#"
            SELECT COUNT(*) AS TotalCount,
                   ISNULL(SUM(d.Amt), 0) AS TotalSales,
                   ISNULL(SUM(d.Commission), 0) AS TotalCommission,
                   ISNULL(SUM(ISNULL(d.Amt, 0) - ISNULL(g.AInPrice, 0) * ISNULL(d.Qty, 0)), 0) AS TotalProfit
            FROM tSal_Inv i
            INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
            LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
            LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
            LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
            LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
            LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
            {where_clause}
            "#,
            where_clause = {
                // 提取 WHERE 子句（从 " WHERE" 开始到 " ORDER BY" 之前）
                let full_sql = &sql;
                let where_start = full_sql.find(" WHERE").unwrap_or(0);
                let order_start = full_sql.find(" ORDER BY").unwrap_or(full_sql.len());
                &full_sql[where_start..order_start]
            }
        );

        let agg_param_refs: Vec<&dyn ToSql> =
            query_params.iter().map(|v| v as &dyn ToSql).collect();
        let agg_stream = conn.query(&agg_sql, &agg_param_refs).await?;
        if let Some(agg_row) = agg_stream.into_row().await? {
            // SQL Server COUNT(*) 返回 i32；兼容 i32/i64 两种情况，避免类型转换 panic
            total_count = agg_row
                .try_get::<i32, _>("TotalCount")
                .ok()
                .flatten()
                .map(|v| v as i64)
                .or_else(|| agg_row.try_get::<i64, _>("TotalCount").ok().flatten())
                .unwrap_or(0);
            total_sales = row_get_f64(&agg_row, "TotalSales");
            total_comm = row_get_f64(&agg_row, "TotalCommission");
            total_profit = row_get_f64(&agg_row, "TotalProfit");
        }

        // 添加分页（OFFSET/FETCH，整数内联无注入风险）
        let offset = ((page - 1) * page_size) as i64;
        let fetch = page_size as i64;
        sql.push_str(&format!(
            " OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
            offset, fetch
        ));
    }

    let param_refs: Vec<&dyn ToSql> = query_params.iter().map(|v| v as &dyn ToSql).collect();
    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;

    let mut items = Vec::new();

    for row in &rows {
        let sales = row_get_f64(row, "Amt");
        let comm = row_get_f64(row, "Commission");
        let profit = row_get_f64(row, "Profit");

        // 不分页时，合计在循环中累加；分页时合计已从聚合查询获取
        if !is_paged {
            total_sales += sales;
            total_comm += comm;
            total_profit += profit;
        }

        let ctype: i32 = row.get::<i32, _>("CommissionType").unwrap_or(0);
        let ctype_text = match ctype {
            1 => "商品规则",
            2 => "品牌规则",
            _ => "默认率",
        };

        items.push(serde_json::json!({
            "orderNo": row.get::<&str, _>("OrderNo").unwrap_or(""),
            "orderDate": row.get::<&str, _>("OrderDate").unwrap_or(""),
            "empId": row.get::<&str, _>("EmpID").unwrap_or(""),
            "empName": row.get::<&str, _>("EmpName").unwrap_or(""),
            "stkId": row.get::<&str, _>("StkID").unwrap_or(""),
            "stkName": row.get::<&str, _>("StkName").unwrap_or(""),
            "gdsId": row.get::<&str, _>("GDSID").unwrap_or(""),
            "gdsNo": row.get::<&str, _>("GDSNO").unwrap_or(""),
            "gdsDesc": row.get::<&str, _>("GDSDesc").unwrap_or(""),
            "unitName": row.get::<&str, _>("UnitName").unwrap_or(""),
            "brandId": row.get::<&str, _>("BrandID").unwrap_or(""),
            "brandName": row.get::<&str, _>("BrandName").unwrap_or(""),
            "brandLevel": row.get::<&str, _>("BrandLevel").unwrap_or(""),
            "categoryId": row.get::<&str, _>("CategoryID").unwrap_or(""),
            "categoryName": row.get::<&str, _>("CategoryName").unwrap_or(""),
            "qty": row_get_f64(row, "Qty"),
            "price": row_get_f64(row, "Price"),
            "amt": sales,
            "costPrice": row_get_f64(row, "CostPrice"),
            "costAmount": row_get_f64(row, "CostAmount"),
            "profit": profit,
            "commissionRate": row_get_f64(row, "CommissionRate"),
            "commissionType": ctype,
            "commissionTypeText": ctype_text,
            "commission": comm,
        }));
    }

    // 分页时 total_count 从聚合查询获取；不分页时从 items 长度获取
    let row_count = if is_paged {
        total_count
    } else {
        items.len() as i64
    };

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "items": items,
        "summary": {
            "totalSales": total_sales,
            "totalCommission": total_comm,
            "totalProfit": total_profit,
            "rowCount": row_count,
        },
    }))))
}

// =====================================================================
// 综合提成报表（对齐 88 项目 commission-unified）
// 一次返回 summary（按门店+员工汇总）+ details（按门店+员工+品牌+提成比例分组）
// =====================================================================

/// POST /api/report/commission-unified
/// 综合提成报表：一次返回汇总+品牌明细，前端双面板展示
/// 筛选：日期/门店/员工/品牌/品牌分类
pub async fn get_commission_unified(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 构造通用 WHERE 条件片段（复用筛选逻辑）
    let mut where_sql = String::from(" WHERE i.State IN ('S', 'Y')");
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1usize;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            where_sql.push_str(&format!(" AND i.SIDate >= @p{}", pidx));
            query_params.push(Some(sd.clone()));
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            where_sql.push_str(&format!(" AND i.SIDate < DATEADD(day, 1, @p{})", pidx));
            query_params.push(Some(ed.clone()));
            pidx += 1;
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            where_sql.push_str(&format!(
                " AND i.StkID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(Some(wid.clone()));
            pidx += 1;
        }
    }
    if let Some(eid) = &params.emp_id {
        if !eid.is_empty() {
            where_sql.push_str(&format!(
                " AND i.EmpID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(Some(eid.clone()));
            pidx += 1;
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            where_sql.push_str(&format!(
                " AND g.BrandID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(Some(bid.clone()));
            pidx += 1;
        }
    }
    if let Some(bl) = &params.brand_level {
        if !bl.is_empty() {
            where_sql.push_str(&format!(" AND b.Level = @p{}", pidx));
            query_params.push(Some(bl.clone()));
            pidx += 1;
        }
    }
    if let Some(spn) = &params.sales_person_name {
        if !spn.is_empty() {
            where_sql.push_str(&format!(" AND e.EmpName LIKE '%' + @p{} + '%'", pidx));
            query_params.push(Some(spn.clone()));
        }
    }

    // 参数引用（&dyn ToSql）需要在两个查询中各用一份，所以先收集成 Vec<String>
    let owned_params: Vec<String> = query_params.iter().filter_map(|v| v.clone()).collect();

    // ---- 汇总查询：按门店+员工分组 ----
    let summary_sql = format!(
        r#"
        SELECT
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(p.PName, '') AS TemplateName,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS CommissionAmount,
            COUNT(DISTINCT i.SIID) AS OrderCount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        LEFT JOIN tSys_Parameters p ON s.CommissionTemplateID = p.ParametersID
        {where_sql}
        GROUP BY i.EmpID, e.EmpName, i.StkID, s.StkName, p.PName
        ORDER BY e.EmpName, s.StkName
        "#,
        where_sql = where_sql
    );

    let summary_param_refs: Vec<&dyn ToSql> =
        owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let summary_stream = conn.query(&summary_sql, &summary_param_refs).await?;
    let summary_rows: Vec<Row> = summary_stream.into_first_result().await?;

    let mut summary_items = Vec::new();
    let mut total_sales = 0.0f64;
    let mut total_comm = 0.0f64;
    let mut total_orders = 0i64;

    for row in &summary_rows {
        let sales = row_get_f64(row, "SalesAmount");
        let comm = row_get_f64(row, "CommissionAmount");
        let order_count = row.get::<i32, _>("OrderCount").unwrap_or(0) as i64;
        total_sales += sales;
        total_comm += comm;
        total_orders += order_count;

        summary_items.push(serde_json::json!({
            "empId": row.get::<&str, _>("EmpID").unwrap_or(""),
            "empName": row.get::<&str, _>("EmpName").unwrap_or(""),
            "stkId": row.get::<&str, _>("StkID").unwrap_or(""),
            "stkName": row.get::<&str, _>("StkName").unwrap_or(""),
            "templateName": row.get::<&str, _>("TemplateName").unwrap_or(""),
            "salesAmount": sales,
            "commissionAmount": comm,
            "orderCount": order_count,
        }));
    }

    // ---- 品牌明细查询：按门店+员工+品牌+提成比例分组 ----
    let detail_sql = format!(
        r#"
        SELECT
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(b.BrandName, '') AS BrandName,
            ISNULL(b.Level, '') AS BrandLevel,
            ISNULL(d.CommissionRate, 0) AS CommissionRate,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS CommissionAmount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        {where_sql}
        GROUP BY i.EmpID, e.EmpName, i.StkID, s.StkName, b.BrandName, b.Level, d.CommissionRate
        ORDER BY e.EmpName, s.StkName, b.Level, b.BrandName
        "#,
        where_sql = where_sql
    );

    let detail_param_refs: Vec<&dyn ToSql> = owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let detail_stream = conn.query(&detail_sql, &detail_param_refs).await?;
    let detail_rows: Vec<Row> = detail_stream.into_first_result().await?;

    let mut detail_items = Vec::new();
    let mut detail_total_sales = 0.0f64;
    let mut detail_total_comm = 0.0f64;

    for row in &detail_rows {
        let sales = row_get_f64(row, "SalesAmount");
        let comm = row_get_f64(row, "CommissionAmount");
        detail_total_sales += sales;
        detail_total_comm += comm;

        detail_items.push(serde_json::json!({
            "empId": row.get::<&str, _>("EmpID").unwrap_or(""),
            "empName": row.get::<&str, _>("EmpName").unwrap_or(""),
            "stkId": row.get::<&str, _>("StkID").unwrap_or(""),
            "stkName": row.get::<&str, _>("StkName").unwrap_or(""),
            "brandName": row.get::<&str, _>("BrandName").unwrap_or(""),
            "brandLevel": row.get::<&str, _>("BrandLevel").unwrap_or(""),
            "commissionRate": row_get_f64(row, "CommissionRate"),
            "salesAmount": sales,
            "commissionAmount": comm,
        }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "summary": summary_items,
        "details": detail_items,
        "totalSalesAmount": total_sales,
        "totalCommission": total_comm,
        "totalOrders": total_orders,
        // 明细的小计（用于校验和调试）
        "detailTotalSales": detail_total_sales,
        "detailTotalCommission": detail_total_comm,
    }))))
}

// =====================================================================
// Excel 导出（对齐 88 项目，使用 rust_xlsxwriter 生成 xlsx）
// 6 个导出端点：
//   1. POST /api/report/commission-unified/export-summary  综合汇总导出
//   2. POST /api/report/commission-unified/export-detail   综合明细导出
//   3. POST /api/report/commission/export-excel            旧版3-sheet报表导出
//   4. POST /api/report/commission-detail/export-excel     提成明细导出(18字段)
//   5. POST /api/commission-template/export-products       商品规则导出
//   6. POST /api/commission-template/export-brands         品牌规则导出
// =====================================================================

/// RFC 5987 编码（支持中文文件名）
/// 将中文字符串编码为 `filename*=UTF-8''<encoded>` 格式可用的值
fn rfc5987_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.as_bytes() {
        if byte.is_ascii_alphanumeric()
            || *byte == b'-'
            || *byte == b'_'
            || *byte == b'.'
            || *byte == b'~'
        {
            result.push(*byte as char);
        } else {
            result.push_str(&format!("%{:02X}", byte));
        }
    }
    result
}

/// 构建 xlsx 二进制响应
fn build_xlsx_response(mut workbook: Workbook, filename: &str) -> Response {
    let buf = match workbook.save_to_buffer() {
        Ok(b) => b,
        Err(e) => {
            let body =
                serde_json::json!({"success":false,"message":&format!("生成Excel失败: {}", e)})
                    .to_string();
            return axum::response::Response::builder()
                .status(500)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };
    let encoded = rfc5987_encode(filename);
    axum::response::Response::builder()
        .status(200)
        .header(
            "Content-Type",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        )
        .header(
            "Content-Disposition",
            format!("attachment; filename*=UTF-8''{}", encoded),
        )
        .body(Body::from(buf))
        .unwrap()
}

/// 构建表头格式（加粗 + 灰底）
fn header_format() -> Format {
    Format::new().set_bold().set_background_color("E0E0E0")
}

/// 构建通用 WHERE 条件片段和参数（复用筛选逻辑）
/// 返回 (where_sql, owned_params)
/// where_sql 形如 " WHERE i.State IN ('S', 'Y') AND i.SIDate >= @p1 AND ..."
/// owned_params 是按 @pN 顺序的参数值列表
fn build_report_where(params: &CommissionReportParams) -> (String, Vec<String>) {
    let mut where_sql = String::from(" WHERE i.State IN ('S', 'Y')");
    let mut query_params: Vec<String> = Vec::new();
    let mut pidx = 1usize;

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            where_sql.push_str(&format!(" AND i.SIDate >= @p{}", pidx));
            query_params.push(sd.clone());
            pidx += 1;
        }
    }
    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            where_sql.push_str(&format!(" AND i.SIDate < DATEADD(day, 1, @p{})", pidx));
            query_params.push(ed.clone());
            pidx += 1;
        }
    }
    if let Some(wid) = &params.warehouse_id {
        if !wid.is_empty() {
            where_sql.push_str(&format!(
                " AND i.StkID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(wid.clone());
            pidx += 1;
        }
    }
    if let Some(eid) = &params.emp_id {
        if !eid.is_empty() {
            where_sql.push_str(&format!(
                " AND i.EmpID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(eid.clone());
            pidx += 1;
        }
    }
    if let Some(bid) = &params.brand_id {
        if !bid.is_empty() {
            where_sql.push_str(&format!(
                " AND g.BrandID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(bid.clone());
            pidx += 1;
        }
    }
    if let Some(bl) = &params.brand_level {
        if !bl.is_empty() {
            where_sql.push_str(&format!(" AND b.Level = @p{}", pidx));
            query_params.push(bl.clone());
            pidx += 1;
        }
    }
    if let Some(spn) = &params.sales_person_name {
        if !spn.is_empty() {
            where_sql.push_str(&format!(" AND e.EmpName LIKE '%' + @p{} + '%'", pidx));
            query_params.push(spn.clone());
        }
    }

    (where_sql, query_params)
}

/// 构建明细报表专用 WHERE 条件（含单据号/销售员/商品关键词/分类筛选）
fn build_detail_where(params: &CommissionReportParams) -> (String, Vec<String>) {
    let (mut where_sql, mut query_params) = build_report_where(params);
    let mut pidx = query_params.len() + 1;

    if let Some(dn) = &params.doc_no {
        if !dn.is_empty() {
            where_sql.push_str(&format!(" AND i.SINO LIKE '%' + @p{} + '%'", pidx));
            query_params.push(dn.clone());
            pidx += 1;
        }
    }
    if let Some(pk) = &params.product_keyword {
        if !pk.is_empty() {
            where_sql.push_str(&format!(
                " AND (g.GDSNO LIKE '%' + @p{} + '%' OR g.GDSDesc LIKE '%' + @p{} + '%')",
                pidx, pidx
            ));
            query_params.push(pk.clone());
            pidx += 1;
        }
    }
    if let Some(cid) = &params.category_id {
        if !cid.is_empty() {
            where_sql.push_str(&format!(
                " AND g.GDSTypeID = CAST(@p{} AS uniqueidentifier)",
                pidx
            ));
            query_params.push(cid.clone());
        }
    }

    (where_sql, query_params)
}

// ---------------------------------------------------------------------
// 导出 1：综合提成汇总导出 Excel
// 对齐 88 项目 ExportCommissionUnifiedSummary
// 字段：序号/负责人/门店名称/提成模板/总销售额/总提成（含合计行）
// ---------------------------------------------------------------------

/// POST /api/report/commission-unified/export-summary
pub async fn export_commission_unified_summary(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    let (where_sql, owned_params) = build_report_where(&params);

    let sql = format!(
        r#"
        SELECT
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(p.PName, '') AS TemplateName,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS CommissionAmount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        LEFT JOIN tSys_Parameters p ON s.CommissionTemplateID = p.ParametersID
        {where_sql}
        GROUP BY i.EmpID, e.EmpName, i.StkID, s.StkName, p.PName
        ORDER BY e.EmpName, s.StkName
        "#,
        where_sql = where_sql
    );

    let param_refs: Vec<&dyn ToSql> = owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let stream = match conn.query(&sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取数据失败: {}", e)),
    };

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let _ = worksheet.set_name("提成汇总表");

    let hfmt = header_format();
    let headers = [
        "序号",
        "负责人",
        "门店名称",
        "提成模板",
        "总销售额",
        "总提成",
    ];
    for (col, h) in headers.iter().enumerate() {
        let _ = worksheet.write_string_with_format(0, col as u16, *h, &hfmt);
    }

    let mut total_sales = 0.0f64;
    let mut total_comm = 0.0f64;
    for (i, row) in rows.iter().enumerate() {
        let r = (i + 2) as u32;
        let sales = row_get_f64(row, "SalesAmount");
        let comm = row_get_f64(row, "CommissionAmount");
        total_sales += sales;
        total_comm += comm;
        let _ = worksheet.write_number(r, 0, (i + 1) as f64);
        let _ = worksheet.write_string(r, 1, row.get::<&str, _>("EmpName").unwrap_or(""));
        let _ = worksheet.write_string(r, 2, row.get::<&str, _>("StkName").unwrap_or(""));
        let _ = worksheet.write_string(r, 3, row.get::<&str, _>("TemplateName").unwrap_or(""));
        let _ = worksheet.write_number(r, 4, sales);
        let _ = worksheet.write_number(r, 5, comm);
    }

    // 合计行（紧接最后一行）
    let summary_row = (rows.len() as u32) + 2;
    let _ = worksheet.write_string(summary_row, 0, "合计");
    let _ = worksheet.write_number(summary_row, 4, total_sales);
    let _ = worksheet.write_number(summary_row, 5, total_comm);

    // 列宽
    let _ = worksheet.set_column_width(0, 8); // A 序号
    let _ = worksheet.set_column_width(1, 15); // B 负责人
    let _ = worksheet.set_column_width(2, 15); // C 门店名称
    let _ = worksheet.set_column_width(3, 15); // D 提成模板
    let _ = worksheet.set_column_width(4, 12); // E 总销售额
    let _ = worksheet.set_column_width(5, 12); // F 总提成

    let now = chrono::Local::now().format("%Y%m%d").to_string();
    let filename = format!("提成汇总表_{}.xlsx", now);
    build_xlsx_response(workbook, &filename)
}

// ---------------------------------------------------------------------
// 导出 2：综合提成明细导出 Excel
// 对齐 88 项目 ExportCommissionUnifiedDetail
// 字段：序号/负责人/门店/品牌分类/品牌/提成比例/销售金额/提成金额（含合计行）
// ---------------------------------------------------------------------

/// POST /api/report/commission-unified/export-detail
pub async fn export_commission_unified_detail(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    let (where_sql, owned_params) = build_report_where(&params);

    let sql = format!(
        r#"
        SELECT
            CONVERT(varchar(40), i.EmpID) AS EmpID,
            ISNULL(e.EmpName, '') AS EmpName,
            CONVERT(varchar(40), i.StkID) AS StkID,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(b.BrandName, '') AS BrandName,
            ISNULL(b.Level, '') AS BrandLevel,
            ISNULL(d.CommissionRate, 0) AS CommissionRate,
            SUM(ISNULL(d.Amt, 0)) AS SalesAmount,
            SUM(ISNULL(d.Commission, 0)) AS CommissionAmount
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        {where_sql}
        GROUP BY i.EmpID, e.EmpName, i.StkID, s.StkName, b.BrandName, b.Level, d.CommissionRate
        ORDER BY e.EmpName, s.StkName, b.Level, b.BrandName
        "#,
        where_sql = where_sql
    );

    let param_refs: Vec<&dyn ToSql> = owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let stream = match conn.query(&sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取数据失败: {}", e)),
    };

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let _ = worksheet.set_name("提成明细表");

    let hfmt = header_format();
    let headers = [
        "序号",
        "负责人",
        "门店",
        "品牌分类",
        "品牌",
        "提成比例",
        "销售金额",
        "提成金额",
    ];
    for (col, h) in headers.iter().enumerate() {
        let _ = worksheet.write_string_with_format(0, col as u16, *h, &hfmt);
    }

    let mut total_sales = 0.0f64;
    let mut total_comm = 0.0f64;
    for (i, row) in rows.iter().enumerate() {
        let r = (i + 2) as u32;
        let sales = row_get_f64(row, "SalesAmount");
        let comm = row_get_f64(row, "CommissionAmount");
        let rate = row_get_f64(row, "CommissionRate");
        total_sales += sales;
        total_comm += comm;
        let _ = worksheet.write_number(r, 0, (i + 1) as f64);
        let _ = worksheet.write_string(r, 1, row.get::<&str, _>("EmpName").unwrap_or(""));
        let _ = worksheet.write_string(r, 2, row.get::<&str, _>("StkName").unwrap_or(""));
        let _ = worksheet.write_string(r, 3, row.get::<&str, _>("BrandLevel").unwrap_or(""));
        let _ = worksheet.write_string(r, 4, row.get::<&str, _>("BrandName").unwrap_or(""));
        let _ = worksheet.write_string(r, 5, format!("{:.2}%", rate * 100.0));
        let _ = worksheet.write_number(r, 6, sales);
        let _ = worksheet.write_number(r, 7, comm);
    }

    let summary_row = (rows.len() as u32) + 2;
    let _ = worksheet.write_string(summary_row, 0, "合计");
    let _ = worksheet.write_number(summary_row, 6, total_sales);
    let _ = worksheet.write_number(summary_row, 7, total_comm);

    let _ = worksheet.set_column_width(0, 8); // A 序号
    let _ = worksheet.set_column_width(1, 12); // B 负责人
    let _ = worksheet.set_column_width(2, 12); // C 门店
    let _ = worksheet.set_column_width(3, 12); // D 品牌分类
    let _ = worksheet.set_column_width(4, 12); // E 品牌
    let _ = worksheet.set_column_width(5, 10); // F 提成比例
    let _ = worksheet.set_column_width(6, 10); // G 销售金额
    let _ = worksheet.set_column_width(7, 10); // H 提成金额

    let now = chrono::Local::now().format("%Y%m%d").to_string();
    let filename = format!("提成明细表_{}.xlsx", now);
    build_xlsx_response(workbook, &filename)
}

// ---------------------------------------------------------------------
// 导出 3：旧版按单据提成报表导出（3 sheet）
// 对齐 88 项目 exportCommissionReportToExcel
// Sheet1: 单据明细（单据编号/门店/销售员/销售日期/销售金额/提成金额）
// Sheet2: 门店汇总（门店/订单数/销售金额/提成金额）
// Sheet3: 销售员汇总（销售员/订单数/销售金额/提成金额）
// ---------------------------------------------------------------------

/// POST /api/report/commission/export-excel
pub async fn export_commission_report_excel(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    let (where_sql, owned_params) = build_report_where(&params);

    // 按单据分组查询
    let sql = format!(
        r#"
        SELECT
            i.SINO AS DocNo,
            ISNULL(s.StkName, '') AS WarehouseName,
            ISNULL(e.EmpName, '') AS SalesPersonName,
            CONVERT(varchar(10), i.SIDate, 120) AS SalesDate,
            ISNULL(d.TotalAmount, 0) AS TotalAmount,
            ISNULL(d.TotalCommission, 0) AS TotalCommission
        FROM tSal_Inv i
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        LEFT JOIN (
            SELECT SIID, SUM(Amt) AS TotalAmount, SUM(Commission) AS TotalCommission
            FROM tSal_InvDetail
            GROUP BY SIID
        ) d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON 1=0
        LEFT JOIN tBas_Brand b ON 1=0
        {where_sql}
        ORDER BY i.SIDate DESC, i.SINO
        "#,
        where_sql = where_sql
    );

    // 注：build_report_where 中引用了 g.BrandID 和 b.Level，
    // 但按单据分组不需要关联 goods/brand，这里用 LEFT JOIN 1=0 占位以保持 WHERE 兼容
    // 实际上品牌/品牌分类筛选在按单据查询时无意义，会被忽略（1=0 使关联为空）

    let param_refs: Vec<&dyn ToSql> = owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let stream = match conn.query(&sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取数据失败: {}", e)),
    };

    let mut workbook = Workbook::new();

    // Sheet1: 单据明细
    let ws1 = workbook.add_worksheet();
    let _ = ws1.set_name("Commission Details");
    let headers1 = [
        "单据编号",
        "门店",
        "销售员",
        "销售日期",
        "销售金额",
        "提成金额",
    ];
    for (col, h) in headers1.iter().enumerate() {
        let _ = ws1.write_string(0, col as u16, *h);
    }

    let mut total_sales = 0.0f64;
    let mut total_comm = 0.0f64;
    // 同时收集门店/销售员汇总数据
    let mut wh_map: std::collections::HashMap<String, (i64, f64, f64)> =
        std::collections::HashMap::new();
    let mut sp_map: std::collections::HashMap<String, (i64, f64, f64)> =
        std::collections::HashMap::new();

    for (i, row) in rows.iter().enumerate() {
        let r = (i + 2) as u32;
        let doc_no = row.get::<&str, _>("DocNo").unwrap_or("").to_string();
        let wh_name = row
            .get::<&str, _>("WarehouseName")
            .unwrap_or("")
            .to_string();
        let sp_name = row
            .get::<&str, _>("SalesPersonName")
            .unwrap_or("")
            .to_string();
        let sales_date = row.get::<&str, _>("SalesDate").unwrap_or("").to_string();
        let amt = row_get_f64(row, "TotalAmount");
        let comm = row_get_f64(row, "TotalCommission");
        total_sales += amt;
        total_comm += comm;

        let _ = ws1.write_string(r, 0, &doc_no);
        let _ = ws1.write_string(r, 1, &wh_name);
        let _ = ws1.write_string(r, 2, &sp_name);
        let _ = ws1.write_string(r, 3, &sales_date);
        let _ = ws1.write_number(r, 4, amt);
        let _ = ws1.write_number(r, 5, comm);

        // 汇总
        let wh_entry = wh_map.entry(wh_name).or_insert((0, 0.0, 0.0));
        wh_entry.0 += 1;
        wh_entry.1 += amt;
        wh_entry.2 += comm;

        let sp_entry = sp_map.entry(sp_name).or_insert((0, 0.0, 0.0));
        sp_entry.0 += 1;
        sp_entry.1 += amt;
        sp_entry.2 += comm;
    }

    // 合计行（空一行）
    let summary_row = (rows.len() as u32) + 3;
    let _ = ws1.write_string(summary_row, 0, "合计");
    let _ = ws1.write_number(summary_row, 4, total_sales);
    let _ = ws1.write_number(summary_row, 5, total_comm);

    let _ = ws1.set_column_width(0, 20);
    let _ = ws1.set_column_width(1, 15);
    let _ = ws1.set_column_width(2, 15);
    let _ = ws1.set_column_width(3, 12);
    let _ = ws1.set_column_width(4, 15);
    let _ = ws1.set_column_width(5, 15);

    // Sheet2: 门店汇总
    if !wh_map.is_empty() {
        let ws2 = workbook.add_worksheet();
        let _ = ws2.set_name("Warehouse Summary");
        let headers2 = ["门店", "订单数", "销售金额", "提成金额"];
        for (col, h) in headers2.iter().enumerate() {
            let _ = ws2.write_string(0, col as u16, *h);
        }
        let mut wh_list: Vec<_> = wh_map.into_iter().collect();
        wh_list.sort_by(|a, b| a.0.cmp(&b.0));
        for (i, (name, (cnt, amt, comm))) in wh_list.iter().enumerate() {
            let r = (i + 2) as u32;
            let _ = ws2.write_string(r, 0, name);
            let _ = ws2.write_number(r, 1, *cnt as f64);
            let _ = ws2.write_number(r, 2, *amt);
            let _ = ws2.write_number(r, 3, *comm);
        }
        let _ = ws2.set_column_width(0, 20);
        let _ = ws2.set_column_width(1, 15);
        let _ = ws2.set_column_width(2, 15);
        let _ = ws2.set_column_width(3, 15);
    }

    // Sheet3: 销售员汇总
    if !sp_map.is_empty() {
        let ws3 = workbook.add_worksheet();
        let _ = ws3.set_name("SalesPerson Summary");
        let headers3 = ["销售员", "订单数", "销售金额", "提成金额"];
        for (col, h) in headers3.iter().enumerate() {
            let _ = ws3.write_string(0, col as u16, *h);
        }
        let mut sp_list: Vec<_> = sp_map.into_iter().collect();
        sp_list.sort_by(|a, b| a.0.cmp(&b.0));
        for (i, (name, (cnt, amt, comm))) in sp_list.iter().enumerate() {
            let r = (i + 2) as u32;
            let _ = ws3.write_string(r, 0, name);
            let _ = ws3.write_number(r, 1, *cnt as f64);
            let _ = ws3.write_number(r, 2, *amt);
            let _ = ws3.write_number(r, 3, *comm);
        }
        let _ = ws3.set_column_width(0, 20);
        let _ = ws3.set_column_width(1, 15);
        let _ = ws3.set_column_width(2, 15);
        let _ = ws3.set_column_width(3, 15);
    }

    let now = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let filename = format!("commission_report_{}.xlsx", now);
    build_xlsx_response(workbook, &filename)
}

// ---------------------------------------------------------------------
// 导出 4：提成明细报表导出 Excel（18 字段）
// 对齐 88 项目 exportCommissionDetailsToExcel
// 字段：单据编号/销售日期/门店/负责人/商品编码/商品名称/品牌/品牌分类/分类/
//       数量/单价/金额/成本价/成本/利润/提成类型/提成比例/提成金额
// ---------------------------------------------------------------------

/// POST /api/report/commission-detail/export-excel
pub async fn export_commission_detail_excel(
    State(_config): State<Config>,
    Json(params): Json<CommissionReportParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    let (where_sql, owned_params) = build_detail_where(&params);

    let sql = format!(
        r#"
        SELECT
            i.SINO AS OrderNo,
            CONVERT(varchar(10), i.SIDate, 120) AS OrderDate,
            ISNULL(s.StkName, '') AS StkName,
            ISNULL(e.EmpName, '') AS EmpName,
            ISNULL(g.GDSNO, '') AS GDSNO,
            ISNULL(g.GDSDesc, '') AS GDSDesc,
            ISNULL(b.BrandName, '') AS BrandName,
            ISNULL(b.Level, '') AS BrandLevel,
            ISNULL(gt.GDSTypeName, '') AS CategoryName,
            ISNULL(d.Qty, 0) AS Qty,
            ISNULL(d.Price, 0) AS Price,
            ISNULL(d.Amt, 0) AS Amt,
            ISNULL(g.AInPrice, 0) AS CostPrice,
            ISNULL(g.AInPrice, 0) * ISNULL(d.Qty, 0) AS CostAmount,
            ISNULL(d.Amt, 0) - ISNULL(g.AInPrice, 0) * ISNULL(d.Qty, 0) AS Profit,
            ISNULL(d.CommissionType, 0) AS CommissionType,
            ISNULL(d.CommissionRate, 0) AS CommissionRate,
            ISNULL(d.Commission, 0) AS Commission
        FROM tSal_Inv i
        INNER JOIN tSal_InvDetail d ON i.SIID = d.SIID
        LEFT JOIN tBas_Goods g ON d.GDSID = g.GDSID
        LEFT JOIN tBas_Brand b ON g.BrandID = b.BrandID
        LEFT JOIN tBas_GDSType gt ON g.GDSTypeID = gt.GDSTypeID
        LEFT JOIN tBas_Emp e ON i.EmpID = e.EmpID
        LEFT JOIN tBas_Stock s ON i.StkID = s.StkID
        {where_sql}
        ORDER BY i.SIDate DESC, i.SINO, d.RowNO
        "#,
        where_sql = where_sql
    );

    let param_refs: Vec<&dyn ToSql> = owned_params.iter().map(|s| s as &dyn ToSql).collect();
    let stream = match conn.query(&sql, &param_refs).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取数据失败: {}", e)),
    };

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let _ = worksheet.set_name("提成明细");

    let hfmt = header_format();
    let headers = [
        "单据编号",
        "销售日期",
        "门店",
        "负责人",
        "商品编码",
        "商品名称",
        "品牌",
        "品牌分类",
        "分类",
        "数量",
        "单价",
        "金额",
        "成本单价",
        "成本金额",
        "毛利",
        "提成类型",
        "提成比例",
        "提成金额",
    ];
    for (col, h) in headers.iter().enumerate() {
        let _ = worksheet.write_string_with_format(0, col as u16, *h, &hfmt);
    }

    let mut total_qty = 0.0f64;
    let mut total_amt = 0.0f64;
    let mut total_cost = 0.0f64;
    let mut total_profit = 0.0f64;
    let mut total_comm = 0.0f64;

    for (i, row) in rows.iter().enumerate() {
        let r = (i + 2) as u32;
        let qty = row_get_f64(row, "Qty");
        let amt = row_get_f64(row, "Amt");
        let cost = row_get_f64(row, "CostAmount");
        let profit = row_get_f64(row, "Profit");
        let comm = row_get_f64(row, "Commission");
        let ctype: i32 = row
            .try_get::<i32, _>("CommissionType")
            .ok()
            .flatten()
            .unwrap_or(0);
        let ctype_str = match ctype {
            1 => "商品规则",
            2 => "品牌规则",
            _ => "默认率",
        };
        let rate = row_get_f64(row, "CommissionRate");

        total_qty += qty;
        total_amt += amt;
        total_cost += cost;
        total_profit += profit;
        total_comm += comm;

        let _ = worksheet.write_string(r, 0, row.get::<&str, _>("OrderNo").unwrap_or(""));
        let _ = worksheet.write_string(r, 1, row.get::<&str, _>("OrderDate").unwrap_or(""));
        let _ = worksheet.write_string(r, 2, row.get::<&str, _>("StkName").unwrap_or(""));
        let _ = worksheet.write_string(r, 3, row.get::<&str, _>("EmpName").unwrap_or(""));
        let _ = worksheet.write_string(r, 4, row.get::<&str, _>("GDSNO").unwrap_or(""));
        let _ = worksheet.write_string(r, 5, row.get::<&str, _>("GDSDesc").unwrap_or(""));
        let _ = worksheet.write_string(r, 6, row.get::<&str, _>("BrandName").unwrap_or(""));
        let _ = worksheet.write_string(r, 7, row.get::<&str, _>("BrandLevel").unwrap_or(""));
        let _ = worksheet.write_string(r, 8, row.get::<&str, _>("CategoryName").unwrap_or(""));
        let _ = worksheet.write_number(r, 9, qty);
        let _ = worksheet.write_number(r, 10, row_get_f64(row, "Price"));
        let _ = worksheet.write_number(r, 11, amt);
        let _ = worksheet.write_number(r, 12, row_get_f64(row, "CostPrice"));
        let _ = worksheet.write_number(r, 13, cost);
        let _ = worksheet.write_number(r, 14, profit);
        let _ = worksheet.write_string(r, 15, ctype_str);
        let _ = worksheet.write_string(r, 16, format!("{:.2}%", rate * 100.0));
        let _ = worksheet.write_number(r, 17, comm);
    }

    // 合计行（紧接最后一行）
    let summary_row = (rows.len() as u32) + 2;
    let _ = worksheet.write_string(summary_row, 0, "合计");
    let _ = worksheet.write_number(summary_row, 9, total_qty);
    let _ = worksheet.write_number(summary_row, 11, total_amt);
    let _ = worksheet.write_number(summary_row, 13, total_cost);
    let _ = worksheet.write_number(summary_row, 14, total_profit);
    let _ = worksheet.write_number(summary_row, 17, total_comm);

    // 列宽：A=18, B=12, C-D=10, E=15, F=20, G-I=12, J-R=10
    let _ = worksheet.set_column_width(0, 18);
    let _ = worksheet.set_column_width(1, 12);
    let _ = worksheet.set_column_width(2, 10);
    let _ = worksheet.set_column_width(3, 10);
    let _ = worksheet.set_column_width(4, 15);
    let _ = worksheet.set_column_width(5, 20);
    let _ = worksheet.set_column_width(6, 12);
    let _ = worksheet.set_column_width(7, 12);
    let _ = worksheet.set_column_width(8, 12);
    for col in 9..=17 {
        let _ = worksheet.set_column_width(col, 10);
    }

    let now = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let filename = format!("commission_details_{}.xlsx", now);
    build_xlsx_response(workbook, &filename)
}

// ---------------------------------------------------------------------
// 导出 5 & 6：商品规则 / 品牌规则导出
// 对齐 88 项目 ExportProductCommissionRules / ExportBrandCommissionRules
// 参数：template_id（提成模板 ID）
// ---------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ExportRulesParams {
    pub template_id: String,
}

/// POST /api/commission-template/export-products
/// 商品规则导出：商品编码/商品名称/提成比例%
pub async fn export_product_rules(
    State(_config): State<Config>,
    Json(params): Json<ExportRulesParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    // 从 tSys_Parameters 读取模板 JSON
    let sql = "SELECT PName, PValue FROM tSys_Parameters WHERE ParametersID = @p1";
    let stream = match conn.query(sql, &[&params.template_id as &dyn ToSql]).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询模板失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取模板失败: {}", e)),
    };

    if rows.is_empty() {
        return json_error_response("提成模板不存在");
    }

    let template_name = rows[0].get::<&str, _>("PName").unwrap_or("").to_string();
    let pvalue = rows[0].get::<&str, _>("PValue").unwrap_or("").to_string();

    // 解析 JSON 获取商品规则列表
    let template_json: serde_json::Value = serde_json::from_str(&pvalue).unwrap_or_default();
    let product_rules = template_json
        .get("product_rules")
        .or_else(|| template_json.get("productRules"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // 收集所有 product_id 用于批量查询商品信息
    let product_ids: Vec<String> = product_rules
        .iter()
        .filter_map(|r| {
            r.get("product_id")
                .or_else(|| r.get("productId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    // 批量查询商品编码和名称
    let mut product_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    if !product_ids.is_empty() {
        let placeholders: Vec<String> = (1..=product_ids.len())
            .map(|i| format!("@p{}", i))
            .collect();
        let in_clause = placeholders.join(", ");
        let goods_sql = format!(
            "SELECT CONVERT(varchar(40), GDSID) AS GDSID, GDSNO, GDSDesc FROM tBas_Goods WHERE GDSID IN ({})",
            in_clause
        );
        let param_refs: Vec<&dyn ToSql> = product_ids.iter().map(|s| s as &dyn ToSql).collect();
        if let Ok(stream) = conn.query(&goods_sql, &param_refs).await {
            if let Ok(goods_rows) = stream.into_first_result().await {
                for row in &goods_rows {
                    let gds_id = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
                    let gds_no = row.get::<&str, _>("GDSNO").unwrap_or("").to_string();
                    let gds_desc = row.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
                    product_map.insert(gds_id, (gds_no, gds_desc));
                }
            }
        }
    }

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let _ = worksheet.set_name("商品提成规则");

    let hfmt = header_format();
    let headers = ["商品编码", "商品名称", "提成比例(%)"];
    for (col, h) in headers.iter().enumerate() {
        let _ = worksheet.write_string_with_format(0, col as u16, *h, &hfmt);
    }

    let mut row_idx = 1u32;
    for rule in &product_rules {
        let pid = rule
            .get("product_id")
            .or_else(|| rule.get("productId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let commission = rule
            .get("commission")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        if let Some((gds_no, gds_desc)) = product_map.get(pid) {
            let _ = worksheet.write_string(row_idx, 0, gds_no);
            let _ = worksheet.write_string(row_idx, 1, gds_desc);
            // 提成比例存储为小数（如 0.05 = 5%），导出为百分比
            let _ = worksheet.write_number(row_idx, 2, commission * 100.0);
            row_idx += 1;
        }
    }

    let _ = worksheet.set_column_width(0, 15);
    let _ = worksheet.set_column_width(1, 30);
    let _ = worksheet.set_column_width(2, 15);

    let now = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let filename = format!("商品提成规则_{}_{}.xlsx", template_name, now);
    build_xlsx_response(workbook, &filename)
}

/// POST /api/commission-template/export-brands
/// 品牌规则导出：品牌名称/提成比例%
pub async fn export_brand_rules(
    State(_config): State<Config>,
    Json(params): Json<ExportRulesParams>,
) -> Response {
    let mut conn = match get_pool().get().await {
        Ok(c) => c,
        Err(e) => return json_error_response(&format!("数据库连接失败: {}", e)),
    };

    let sql = "SELECT PName, PValue FROM tSys_Parameters WHERE ParametersID = @p1";
    let stream = match conn.query(sql, &[&params.template_id as &dyn ToSql]).await {
        Ok(s) => s,
        Err(e) => return json_error_response(&format!("查询模板失败: {}", e)),
    };
    let rows: Vec<Row> = match stream.into_first_result().await {
        Ok(r) => r,
        Err(e) => return json_error_response(&format!("读取模板失败: {}", e)),
    };

    if rows.is_empty() {
        return json_error_response("提成模板不存在");
    }

    let template_name = rows[0].get::<&str, _>("PName").unwrap_or("").to_string();
    let pvalue = rows[0].get::<&str, _>("PValue").unwrap_or("").to_string();

    let template_json: serde_json::Value = serde_json::from_str(&pvalue).unwrap_or_default();
    let brand_rules = template_json
        .get("brand_rules")
        .or_else(|| template_json.get("brandRules"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let brand_ids: Vec<String> = brand_rules
        .iter()
        .filter_map(|r| {
            r.get("brand_id")
                .or_else(|| r.get("brandId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let mut brand_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !brand_ids.is_empty() {
        let placeholders: Vec<String> = (1..=brand_ids.len()).map(|i| format!("@p{}", i)).collect();
        let in_clause = placeholders.join(", ");
        let brand_sql = format!(
            "SELECT CONVERT(varchar(40), BrandID) AS BrandID, BrandName FROM tBas_Brand WHERE BrandID IN ({})",
            in_clause
        );
        let param_refs: Vec<&dyn ToSql> = brand_ids.iter().map(|s| s as &dyn ToSql).collect();
        if let Ok(stream) = conn.query(&brand_sql, &param_refs).await {
            if let Ok(brand_rows) = stream.into_first_result().await {
                for row in &brand_rows {
                    let bid = row.get::<&str, _>("BrandID").unwrap_or("").to_string();
                    let bname = row.get::<&str, _>("BrandName").unwrap_or("").to_string();
                    brand_map.insert(bid, bname);
                }
            }
        }
    }

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let _ = worksheet.set_name("品牌提成规则");

    let hfmt = header_format();
    let headers = ["品牌名称", "提成比例(%)"];
    for (col, h) in headers.iter().enumerate() {
        let _ = worksheet.write_string_with_format(0, col as u16, *h, &hfmt);
    }

    let mut row_idx = 1u32;
    for rule in &brand_rules {
        let bid = rule
            .get("brand_id")
            .or_else(|| rule.get("brandId"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let commission = rule
            .get("commission")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        if let Some(bname) = brand_map.get(bid) {
            let _ = worksheet.write_string(row_idx, 0, bname);
            let _ = worksheet.write_number(row_idx, 1, commission * 100.0);
            row_idx += 1;
        }
    }

    let _ = worksheet.set_column_width(0, 20);
    let _ = worksheet.set_column_width(1, 18);

    let now = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let filename = format!("品牌提成规则_{}_{}.xlsx", template_name, now);
    build_xlsx_response(workbook, &filename)
}

/// 构建 JSON 错误响应（用于导出函数中的错误处理）
fn json_error_response(msg: &str) -> Response {
    let body = serde_json::json!({"success":false,"message":msg}).to_string();
    axum::response::Response::builder()
        .status(500)
        .header("Content-Type", "application/json")
        .body(Body::from(body))
        .unwrap()
}
