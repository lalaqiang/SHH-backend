use axum::{
    extract::State,
    Extension,
    Json,
};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort, row_get_f64};
use crate::handlers::base_data::row_to_json;
use crate::middleware::auth::Claims;

// ============================================================
// Online Products (商品池管理)
// ============================================================

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetOnlineProductsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sale_type: Option<String>,
    pub status: Option<i32>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_online_products(
    State(_config): State<Config>,
    Json(params): Json<GetOnlineProductsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], s.[StkName]
        FROM [tOnline_Goods] og
        LEFT JOIN [tBas_Goods] g ON og.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON og.[StkID] = s.[StkID]
        WHERE og.[State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSDesc] LIKE @p{} OR g.[GDSNO] LIKE @p{} OR g.[GDSSpec] LIKE @p{})", pidx, pidx + 1, pidx + 2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if let Some(st) = &params.sale_type {
        if !st.is_empty() {
            base_query.push_str(&format!(" AND og.[SaleType] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(st.clone()));
        }
    }

    if let Some(status) = params.status {
        base_query.push_str(&format!(" AND og.[Status] = @p{}", pidx));
        query_params.push(Some(status.to_string()));
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetOnlineProductParams {
    pub id: String,
}

pub async fn get_online_product(
    State(_config): State<Config>,
    Json(params): Json<GetOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], s.[StkName]
        FROM [tOnline_Goods] og
        LEFT JOIN [tBas_Goods] g ON og.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON og.[StkID] = s.[StkID]
        WHERE og.[OnlineGDSID] = @p1 AND og.[State] <> 'D'"#;
    let stream = conn.query(sql, &[&params.id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("商品不存在")))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateOnlineProductParams {
    pub gds_id: Option<String>,
    pub sale_type: Option<String>,
    pub clearance_price: Option<f64>,
    pub max_order_qty: Option<i32>,
    pub sort: Option<i32>,
    pub status: Option<i32>,
    pub stk_id: Option<String>,
}

pub async fn create_online_product(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let gds_id = body.gds_id.as_deref().unwrap_or("");
    let sale_type = body.sale_type.as_deref().unwrap_or("normal");
    let clearance_price = body.clearance_price.unwrap_or(0.0);
    let max_order_qty = body.max_order_qty.unwrap_or(0);
    let sort = body.sort.unwrap_or(0);
    let status = body.status.unwrap_or(1);
    let stk_id = body.stk_id.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tOnline_Goods] ([OnlineGDSID], [GDSID], [SaleType], [ClearancePrice], [MaxOrderQty], [Sort], [Status], [StkID], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, 'A', @p8, @p9)"#;

    conn.execute(sql, &[
        &gds_id,
        &sale_type,
        &clearance_price,
        &max_order_qty,
        &sort,
        &status,
        &stk_id,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("线上商品创建成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateOnlineProductParams {
    pub online_gds_id: String,
    pub gds_id: Option<String>,
    pub sale_type: Option<String>,
    pub clearance_price: Option<f64>,
    pub max_order_qty: Option<i32>,
    pub sort: Option<i32>,
    pub status: Option<i32>,
    pub stk_id: Option<String>,
}

pub async fn update_online_product(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let gds_id = body.gds_id.as_deref().unwrap_or("");
    let sale_type = body.sale_type.as_deref().unwrap_or("normal");
    let clearance_price = body.clearance_price.unwrap_or(0.0);
    let max_order_qty = body.max_order_qty.unwrap_or(0);
    let sort = body.sort.unwrap_or(0);
    let status = body.status.unwrap_or(1);
    let stk_id = body.stk_id.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_Goods] SET
        [GDSID] = @p1, [SaleType] = @p2, [ClearancePrice] = @p3,
        [MaxOrderQty] = @p4, [Sort] = @p5, [Status] = @p6,
        [StkID] = @p7, [EDate] = @p8, [EUser] = @p9
        WHERE [OnlineGDSID] = @p10 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &gds_id,
        &sale_type,
        &clearance_price,
        &max_order_qty,
        &sort,
        &status,
        &stk_id,
        &now,
        &claims.user_code.as_str(),
        &body.online_gds_id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("线上商品更新成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeleteOnlineProductParams {
    pub ids: Vec<String>,
}

pub async fn delete_online_product(
    State(_config): State<Config>,
    Json(body): Json<DeleteOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的商品")));
    }

    for id in &body.ids {
        let sql = "UPDATE [tOnline_Goods] SET [State] = 'D' WHERE [OnlineGDSID] = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个线上商品", body.ids.len()))))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BrowseOnlineProductsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sale_type: Option<String>,
    pub stk_id: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn browse_online_products(
    State(_config): State<Config>,
    Json(params): Json<BrowseOnlineProductsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[BarCode], g.[SPrice], s.[StkName],
        ISNULL(sq.[Qty],0) AS [StockQty]
        FROM [tOnline_Goods] og
        LEFT JOIN [tBas_Goods] g ON og.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON og.[StkID] = s.[StkID]
        LEFT JOIN [tStk_Qty] sq ON og.[GDSID] = sq.[GDSID] AND og.[StkID] = sq.[StkID]
        WHERE og.[State] <> 'D' AND og.[Status] = 1"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (g.[GDSDesc] LIKE @p{} OR g.[GDSNO] LIKE @p{} OR g.[BarCode] LIKE @p{})", pidx, pidx + 1, pidx + 2));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if let Some(st) = &params.sale_type {
        if !st.is_empty() {
            base_query.push_str(&format!(" AND og.[SaleType] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(st.clone()));
        }
    }

    if let Some(sid) = &params.stk_id {
        if !sid.is_empty() {
            base_query.push_str(&format!(" AND og.[StkID] = @p{}", pidx));
            query_params.push(Some(sid.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BrowseOnlineProductParams {
    pub id: String,
}

pub async fn browse_online_product(
    State(_config): State<Config>,
    Json(params): Json<BrowseOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[BarCode], g.[SPrice], s.[StkName],
        ISNULL(sq.[Qty],0) AS [StockQty]
        FROM [tOnline_Goods] og
        LEFT JOIN [tBas_Goods] g ON og.[GDSID] = g.[GDSID]
        LEFT JOIN [tBas_Stock] s ON og.[StkID] = s.[StkID]
        LEFT JOIN [tStk_Qty] sq ON og.[GDSID] = sq.[GDSID] AND og.[StkID] = sq.[StkID]
        WHERE og.[OnlineGDSID] = @p1 AND og.[State] <> 'D' AND og.[Status] = 1"#;
    let stream = conn.query(sql, &[&params.id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("商品不存在或已下架")))
    }
}

// ============================================================
// Online Orders (订单管理)
// ============================================================

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OrderItemInput {
    pub online_product_id: String,
    pub quantity: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlaceOnlineOrderParams {
    pub items: Vec<OrderItemInput>,
    pub contact_name: Option<String>,
    pub contact_phone: Option<String>,
    pub address: Option<String>,
    pub payment_method: Option<String>,
    pub remark: Option<String>,
}

pub async fn place_online_order(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<PlaceOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let today = chrono::Local::now().format("%Y%m%d").to_string();

    if body.items.is_empty() {
        return Ok(Json(ApiResponse::err("订单商品不能为空")));
    }

    let count_sql = "SELECT COUNT(*) as cnt FROM [tOnline_Order] WHERE [OrderNo] LIKE @p1";
    let prefix = format!("OL{}%", today);
    let stream = conn.query(count_sql, &[&prefix.as_str()]).await?;
    let mut seq: i32 = 1;
    if let Some(row) = stream.into_row().await? {
        seq = row.get::<i32, _>("cnt").unwrap_or(0) + 1;
    }

    let order_no = format!("OL{}{:04}", today, seq);

    // Look up product info for each item from tOnline_Goods + tBas_Goods
    let mut total_amt: f64 = 0.0;
    let mut detail_rows: Vec<(String, String, String, i32, f64, f64, String)> = Vec::new();
    for item in &body.items {
        let prod_sql = r#"SELECT og.[GDSID], og.[SaleType], og.[ClearancePrice], g.[GDSDesc], g.[GDSNO], g.[SPrice]
            FROM [tOnline_Goods] og
            LEFT JOIN [tBas_Goods] g ON og.[GDSID] = g.[GDSID]
            WHERE og.[OnlineGDSID] = @p1 AND og.[State] <> 'D' AND og.[Status] = 1"#;
        let prod_stream = conn.query(prod_sql, &[&item.online_product_id.as_str()]).await?;
        if let Some(row) = prod_stream.into_row().await? {
            let gds_id: String = row.get::<&str, _>("GDSID").unwrap_or("").to_string();
            let gds_desc: String = row.get::<&str, _>("GDSDesc").unwrap_or("").to_string();
            let gds_no: String = row.get::<&str, _>("GDSNO").unwrap_or("").to_string();
            let sale_type: String = row.get::<&str, _>("SaleType").unwrap_or("normal").to_string();
            let clearance_price: f64 = row_get_f64(&row, "ClearancePrice");
            let s_price: f64 = row_get_f64(&row, "SPrice");
            let price = if sale_type == "clearance" && clearance_price > 0.0 { clearance_price } else { s_price };
            let qty = item.quantity;
            let line_amt = price * qty as f64;
            total_amt += line_amt;
            detail_rows.push((gds_id, gds_no, gds_desc, qty, price, line_amt, sale_type));
        }
    }

    if detail_rows.is_empty() {
        return Ok(Json(ApiResponse::err("商品信息查询失败")));
    }

    let contact_name = body.contact_name.as_deref().unwrap_or("");
    let contact_phone = body.contact_phone.as_deref().unwrap_or("");
    let address = body.address.as_deref().unwrap_or("");
    let remark = body.remark.as_deref().unwrap_or("");
    let payment_method = body.payment_method.as_deref().unwrap_or("cod");

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        "".to_string()
    };

    let order_sql = r#"INSERT INTO [tOnline_Order] ([OnlineOrderID], [OrderNo], [EmpID], [ContactName], [ContactPhone], [Address], [TotalAmt], [Status], [PaymentStatus], [PaymentMethod], [Remark], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, 'pending', 'unpaid', @p7, @p8, 'A', @p9, @p10)"#;

    conn.execute(order_sql, &[
        &order_no.as_str(),
        &emp_id.as_str(),
        &contact_name,
        &contact_phone,
        &address,
        &total_amt,
        &payment_method,
        &remark,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    let order_id_sql = "SELECT [OnlineOrderID] FROM [tOnline_Order] WHERE [OrderNo] = @p1";
    let oid_stream = conn.query(order_id_sql, &[&order_no.as_str()]).await?;
    let order_id = if let Some(row) = oid_stream.into_row().await? {
        row.get::<&str, _>("OnlineOrderID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("订单创建失败")));
    };

    for (gds_id, gds_no, gds_desc, qty, price, line_amt, sale_type) in &detail_rows {
        let detail_sql = r#"INSERT INTO [tOnline_OrderDetail] ([OnlineOrderDtlID], [OnlineOrderID], [GDSID], [GDSNO], [GDSDesc], [Qty], [Price], [Amt], [SaleType], [CostPrice], [State])
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, 0, 'A')"#;
        conn.execute(detail_sql, &[
            &order_id.as_str(),
            &gds_id.as_str(),
            &gds_no.as_str(),
            &gds_desc.as_str(),
            qty,
            price,
            line_amt,
            &sale_type.as_str(),
        ]).await?;
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({ "OrderNo": order_no, "OrderID": order_id, "TotalAmt": total_amt }))))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetOnlineOrdersParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub status: Option<String>,
    pub payment_status: Option<String>,
    pub ship_status: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_online_orders(
    State(_config): State<Config>,
    Json(params): Json<GetOnlineOrdersParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = r#"SELECT o.*, e.[EmpName]
        FROM [tOnline_Order] o
        LEFT JOIN [tBas_Emp] e ON o.[EmpID] = e.[EmpID]
        WHERE o.[State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND (o.[OrderNo] LIKE @p{} OR e.[EmpName] LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    if let Some(st) = &params.status {
        if !st.is_empty() {
            base_query.push_str(&format!(" AND o.[Status] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(st.clone()));
        }
    }

    if let Some(ps) = &params.payment_status {
        if !ps.is_empty() {
            base_query.push_str(&format!(" AND o.[PaymentStatus] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ps.clone()));
        }
    }

    if let Some(ss) = &params.ship_status {
        if !ss.is_empty() {
            base_query.push_str(&format!(" AND o.[ShipStatus] = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(ss.clone()));
        }
    }

    if let Some(sd) = &params.start_date {
        if !sd.is_empty() {
            base_query.push_str(&format!(" AND CONVERT(varchar(10), o.[EDate], 120) >= @p{}", pidx));
            pidx += 1;
            query_params.push(Some(sd.clone()));
        }
    }

    if let Some(ed) = &params.end_date {
        if !ed.is_empty() {
            base_query.push_str(&format!(" AND CONVERT(varchar(10), o.[EDate], 120) <= @p{}", pidx));
            query_params.push(Some(ed.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetMyOnlineOrdersParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub status: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_my_online_orders(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<GetMyOnlineOrdersParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::ok_paginated(vec![], 0, page, page_size)));
    };

    let mut base_query = r#"SELECT o.*, e.[EmpName]
        FROM [tOnline_Order] o
        LEFT JOIN [tBas_Emp] e ON o.[EmpID] = e.[EmpID]
        WHERE o.[State] <> 'D' AND o.[EmpID] = @p1"#.to_string();
    let mut query_params: Vec<Option<String>> = vec![Some(emp_id.clone())];
    let pidx = 2;

    if let Some(st) = &params.status {
        if !st.is_empty() {
            base_query.push_str(&format!(" AND o.[Status] = @p{}", pidx));
            query_params.push(Some(st.clone()));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetOnlineOrderParams {
    pub id: String,
}

pub async fn get_online_order(
    State(_config): State<Config>,
    Json(params): Json<GetOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let order_sql = r#"SELECT o.*, e.[EmpName]
        FROM [tOnline_Order] o
        LEFT JOIN [tBas_Emp] e ON o.[EmpID] = e.[EmpID]
        WHERE o.[OnlineOrderID] = @p1 AND o.[State] <> 'D'"#;
    let order_stream = conn.query(order_sql, &[&params.id.as_str()]).await?;

    let order_row = match order_stream.into_row().await? {
        Some(row) => row,
        None => return Ok(Json(ApiResponse::err("订单不存在"))),
    };
    let mut order_data = row_to_json(&order_row);

    let detail_sql = r#"SELECT od.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[BarCode]
        FROM [tOnline_OrderDetail] od
        LEFT JOIN [tBas_Goods] g ON od.[GDSID] = g.[GDSID]
        WHERE od.[OnlineOrderID] = @p1 AND od.[State] <> 'D'"#;
    let detail_stream = conn.query(detail_sql, &[&params.id.as_str()]).await?;
    let detail_rows: Vec<Row> = detail_stream.into_first_result().await?;
    let details: Vec<serde_json::Value> = detail_rows.iter().map(row_to_json).collect();

    if let Some(obj) = order_data.as_object_mut() {
        obj.insert("items".to_string(), serde_json::Value::Array(details));
    }

    Ok(Json(ApiResponse::ok(order_data)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmOnlineOrderParams {
    pub id: String,
}

pub async fn confirm_online_order(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<ConfirmOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let sql = r#"UPDATE [tOnline_Order] SET [Status] = 'confirmed', [EDate] = @p1, [EUser] = @p2
        WHERE [OnlineOrderID] = @p3 AND [State] <> 'D' AND [Status] = 'pending'"#;
    let result = conn.execute(sql, &[
        &now,
        &claims.user_code.as_str(),
        &body.id.as_str(),
    ]).await?;

    if result.total() == 0 {
        return Ok(Json(ApiResponse::err("订单确认失败，可能状态不是待确认")));
    }

    Ok(Json(ApiResponse::msg("订单确认成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CancelOnlineOrderParams {
    pub id: String,
    pub reason: Option<String>,
}

pub async fn cancel_online_order(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CancelOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let reason = body.reason.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_Order] SET [Status] = 'cancelled', [Remark] = ISNULL([Remark],'') + @p1, [EDate] = @p2, [EUser] = @p3
        WHERE [OnlineOrderID] = @p4 AND [State] <> 'D' AND [Status] IN ('pending', 'confirmed')"#;
    let cancel_remark = if reason.is_empty() { "".to_string() } else { format!(" [取消原因: {}]", reason) };
    let result = conn.execute(sql, &[
        &cancel_remark.as_str(),
        &now,
        &claims.user_code.as_str(),
        &body.id.as_str(),
    ]).await?;

    if result.total() == 0 {
        return Ok(Json(ApiResponse::err("订单取消失败，可能状态不允许取消")));
    }

    Ok(Json(ApiResponse::msg("订单取消成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateOnlineOrderShipInfoParams {
    pub id: String,
    pub ship_company: Option<String>,
    pub ship_no: Option<String>,
    pub ship_status: Option<String>,
}

pub async fn update_online_order_ship_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateOnlineOrderShipInfoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let ship_company = body.ship_company.as_deref().unwrap_or("");
    let ship_no = body.ship_no.as_deref().unwrap_or("");
    let ship_status = body.ship_status.as_deref().unwrap_or("unshipped");

    let sql = r#"UPDATE [tOnline_Order] SET [TrackingCompany] = @p1, [TrackingNo] = @p2, [ShipStatus] = @p3, [EDate] = @p4, [EUser] = @p5
        WHERE [OnlineOrderID] = @p6 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &ship_company,
        &ship_no,
        &ship_status,
        &now,
        &claims.user_code.as_str(),
        &body.id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("物流信息更新成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BatchUpdateShipInfoItem {
    pub id: String,
    pub ship_company: Option<String>,
    pub ship_no: Option<String>,
    pub ship_status: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BatchUpdateOnlineOrderShipInfoParams {
    pub items: Vec<BatchUpdateShipInfoItem>,
}

pub async fn batch_update_online_order_ship_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<BatchUpdateOnlineOrderShipInfoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    if body.items.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要更新的订单")));
    }

    let mut updated = 0u32;
    for item in &body.items {
        let ship_company = item.ship_company.as_deref().unwrap_or("");
        let ship_no = item.ship_no.as_deref().unwrap_or("");
        let ship_status = item.ship_status.as_deref().unwrap_or("unshipped");

        let sql = r#"UPDATE [tOnline_Order] SET [TrackingCompany] = @p1, [TrackingNo] = @p2, [ShipStatus] = @p3, [EDate] = @p4, [EUser] = @p5
            WHERE [OnlineOrderID] = @p6 AND [State] <> 'D'"#;

        let result = conn.execute(sql, &[
            &ship_company,
            &ship_no,
            &ship_status,
            &now,
            &claims.user_code.as_str(),
            &item.id.as_str(),
        ]).await?;

        updated += result.total() as u32;
    }

    Ok(Json(ApiResponse::msg(&format!("成功更新{}个订单的物流信息", updated))))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BatchGenerateSalesOrdersParams {
    pub order_ids: Vec<String>,
}

pub async fn batch_generate_sales_orders(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<BatchGenerateSalesOrdersParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    if body.order_ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要生成销售单的订单")));
    }

    let mut generated = 0u32;
    let mut results: Vec<serde_json::Value> = Vec::new();

    for order_id in &body.order_ids {
        let order_sql = r#"SELECT o.[OnlineOrderID], o.[OrderNo], o.[EmpID], o.[TotalAmt]
            FROM [tOnline_Order] o
            WHERE o.[OnlineOrderID] = @p1 AND o.[State] <> 'D' AND o.[Status] = 'confirmed'"#;
        let order_stream = conn.query(order_sql, &[&order_id.as_str()]).await?;

        let order_row = match order_stream.into_row().await? {
            Some(row) => row,
            None => continue,
        };

        let order_no: &str = order_row.get::<&str, _>("OrderNo").unwrap_or("");
        let emp_id: &str = order_row.get::<&str, _>("EmpID").unwrap_or("");
        let total_amt: f64 = row_get_f64(&order_row, "TotalAmt");

        let sal_no = format!("SOL{}", &order_no[2..]);
        let si_id = format!("{}", uuid::Uuid::new_v4());

        let sal_sql = r#"INSERT INTO [tSal_Inv] ([SIID], [SINo], [SIDate], [CustID], [SumAmt], [State], [EDate], [EUser], [LUTime])
            VALUES (@p1, @p2, @p3, @p4, @p5, 'N', @p3, @p6, @p3)"#;
        let zero_uuid = "00000000-0000-0000-0000-000000000000";
        conn.execute(sal_sql, &[
            &si_id.as_str(),
            &sal_no.as_str(),
            &now,
            &emp_id,
            &total_amt,
            &zero_uuid,
        ]).await?;

        let detail_sql = r#"SELECT [GDSID], [Qty], [Price], [LineAmt]
            FROM [tOnline_OrderDetail]
            WHERE [OrderID] = @p1 AND [State] <> 'D'"#;
        let detail_stream = conn.query(detail_sql, &[&order_id.as_str()]).await?;
        let detail_rows: Vec<Row> = detail_stream.into_first_result().await?;

        let sal_inv_id = si_id.clone();

        for (i, dr) in detail_rows.iter().enumerate() {
            let gds_id: &str = dr.get::<&str, _>("GDSID").unwrap_or("");
            let qty: i32 = dr.get::<i32, _>("Qty").unwrap_or(0);
            let price: f64 = row_get_f64(&dr, "Price");
            let line_amt: f64 = row_get_f64(&dr, "LineAmt");
            let row_no = format!("{:03}", i + 1);

            let sal_detail_sql = r#"INSERT INTO [tSal_InvDetail] ([SIID], [SIDetailID], [RowNO], [GDSID], [Qty], [Price], [Amt])
                VALUES (@p1, NEWID(), @p2, @p3, @p4, @p5, @p6)"#;
            conn.execute(sal_detail_sql, &[
                &sal_inv_id.as_str(),
                &row_no,
                &gds_id,
                &qty,
                &price,
                &line_amt,
            ]).await?;
        }

        let update_order_sql = r#"UPDATE [tOnline_Order] SET [Status] = 'processed', [EDate] = @p1, [EUser] = @p2
            WHERE [OnlineOrderID] = @p3 AND [State] <> 'D'"#;
        conn.execute(update_order_sql, &[
            &now,
            &claims.user_code.as_str(),
            &order_id.as_str(),
        ]).await?;

        // ★ 线上订单生成销售单后自动重算提成（对齐 88 项目，不依赖前端调用）
        // 提成计算失败不影响订单处理主流程，仅记录 warn 日志
        if let Err(e) = crate::services::commission_service::recalc_invoice_commission(&mut conn, &si_id).await {
            tracing::warn!("[batch_generate_sales_orders] 线上订单 {} 生成销售单 {} 提成重算失败: {}", order_no, si_id, e);
        }

        generated += 1;
        results.push(serde_json::json!({ "OrderNo": order_no, "SalInvNO": sal_no }));
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "generated": generated,
        "results": results
    }))))
}

// ============================================================
// Payment (支付)
// ============================================================

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetPaymentConfigsParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_payment_configs(
    State(_config): State<Config>,
    Json(params): Json<GetPaymentConfigsParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let mut base_query = r#"SELECT * FROM [tOnline_PaymentConfig] WHERE [State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND ([PName] LIKE @p{} OR [PCode] LIKE @p{})", pidx, pidx + 1));
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
        }
    }

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(&base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());
    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetPaymentConfigParams {
    pub id: String,
}

pub async fn get_payment_config(
    State(_config): State<Config>,
    Json(params): Json<GetPaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT * FROM [tOnline_PaymentConfig] WHERE [PaymentConfigID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(sql, &[&params.id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("支付配置不存在")))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreatePaymentConfigParams {
    pub code: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub help: Option<String>,
    pub sort: Option<i32>,
    pub enabled: Option<i32>,
}

pub async fn create_payment_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let code = body.code.as_deref().unwrap_or("");
    let name = body.name.as_deref().unwrap_or("");
    let kind = body.kind.as_deref().unwrap_or("payment");
    let help = body.help.as_deref().unwrap_or("");
    let sort = body.sort.unwrap_or(0);
    let enabled = body.enabled.unwrap_or(1);

    let sql = r#"INSERT INTO [tOnline_PaymentConfig] ([PaymentConfigID], [PCode], [PName], [PKind], [PHelp], [Enabled], [Sort], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, 'A', @p7, @p8)"#;

    conn.execute(sql, &[
        &code,
        &name,
        &kind,
        &help,
        &enabled,
        &sort,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付配置创建成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdatePaymentConfigParams {
    pub id: String,
    pub code: Option<String>,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub help: Option<String>,
    pub sort: Option<i32>,
    pub enabled: Option<i32>,
}

pub async fn update_payment_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdatePaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let code = body.code.as_deref().unwrap_or("");
    let name = body.name.as_deref().unwrap_or("");
    let kind = body.kind.as_deref().unwrap_or("payment");
    let help = body.help.as_deref().unwrap_or("");
    let sort = body.sort.unwrap_or(0);
    let enabled = body.enabled.unwrap_or(1);

    let sql = r#"UPDATE [tOnline_PaymentConfig] SET
        [PCode] = @p1, [PName] = @p2, [PKind] = @p3, [PHelp] = @p4,
        [Enabled] = @p5, [Sort] = @p6, [EDate] = @p7, [EUser] = @p8
        WHERE [PaymentConfigID] = @p9 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &code,
        &name,
        &kind,
        &help,
        &enabled,
        &sort,
        &now,
        &claims.user_code.as_str(),
        &body.id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付配置更新成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeletePaymentConfigParams {
    pub ids: Vec<String>,
}

pub async fn delete_payment_config(
    State(_config): State<Config>,
    Json(body): Json<DeletePaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的支付配置")));
    }

    for id in &body.ids {
        let sql = "UPDATE [tOnline_PaymentConfig] SET [State] = 'D' WHERE [PaymentConfigID] = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个支付配置", body.ids.len()))))
}

pub async fn get_available_payment_methods(
    State(_config): State<Config>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT [PaymentConfigID], [PCode], [PName], [PKind], [PHelp], [QRCodeUrl], [IsPersonal], [Enabled], [Sort] FROM [tOnline_PaymentConfig] WHERE [State] <> 'D' AND [Enabled] = 1 ORDER BY [Sort]";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateOnlineOrderPaymentParams {
    pub order_id: String,
    pub payment_method: Option<String>,
}

pub async fn create_online_order_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateOnlineOrderPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let pay_method = body.payment_method.as_deref().unwrap_or("");

    let check_sql = "SELECT [OnlineOrderID], [Status], [PaymentStatus] FROM [tOnline_Order] WHERE [OnlineOrderID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(check_sql, &[&body.order_id.as_str()]).await?;

    let order_row = match stream.into_row().await? {
        Some(row) => row,
        None => return Ok(Json(ApiResponse::err("订单不存在"))),
    };

    let payment_status: &str = order_row.get::<&str, _>("PaymentStatus").unwrap_or("");
    if payment_status == "paid" {
        return Ok(Json(ApiResponse::err("订单已支付")));
    }

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'pending', [PaymentMethod] = @p1, [EDate] = @p2, [EUser] = @p3
        WHERE [OnlineOrderID] = @p4 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &pay_method,
        &now,
        &claims.user_code.as_str(),
        &body.order_id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "OrderID": body.order_id,
        "PaymentStatus": "pending"
    }))))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QueryPaymentStatusParams {
    pub order_id: String,
}

pub async fn query_payment_status(
    State(_config): State<Config>,
    Json(params): Json<QueryPaymentStatusParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT [OnlineOrderID], [OrderNo], [PaymentStatus], [PaymentMethod], [PaymentProof] FROM [tOnline_Order] WHERE [OnlineOrderID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(sql, &[&params.order_id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("订单不存在")))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UploadPaymentProofParams {
    pub order_id: String,
    pub payment_proof: Option<String>,
}

pub async fn upload_payment_proof(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UploadPaymentProofParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let payment_proof = body.payment_proof.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentProof] = @p1, [PaymentStatus] = 'proof_uploaded', [EDate] = @p2, [EUser] = @p3
        WHERE [OnlineOrderID] = @p4 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &payment_proof,
        &now,
        &claims.user_code.as_str(),
        &body.order_id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付凭证上传成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct VerifyPaymentParams {
    pub order_id: String,
    pub verified: Option<bool>,
}

pub async fn verify_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<VerifyPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let new_status = if body.verified.unwrap_or(true) { "paid" } else { "verify_failed" };

    let sql = if new_status == "paid" {
        r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'paid', [LUTime] = @p1, [EDate] = @p1, [EUser] = @p2
            WHERE [OnlineOrderID] = @p3 AND [State] <> 'D'"#
    } else {
        r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'verify_failed', [EDate] = @p1, [EUser] = @p2
            WHERE [OnlineOrderID] = @p3 AND [State] <> 'D'"#
    };

    conn.execute(sql, &[
        &now,
        &claims.user_code.as_str(),
        &body.order_id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg(if new_status == "paid" { "支付验证通过" } else { "支付验证未通过" })))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClaimPersonalPaymentParams {
    pub order_id: String,
}

pub async fn claim_personal_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<ClaimPersonalPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("用户信息获取失败")));
    };

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'paid', [LUTime] = @p1, [PaymentMethod] = 'personal', [EDate] = @p1, [EUser] = @p2
        WHERE [OnlineOrderID] = @p3 AND [EmpID] = @p4 AND [State] <> 'D' AND [PaymentStatus] IN ('unpaid', 'pending', 'proof_uploaded')"#;

    let result = conn.execute(sql, &[
        &now,
        &claims.user_code.as_str(),
        &body.order_id.as_str(),
        &emp_id.as_str(),
    ]).await?;

    if result.total() == 0 {
        return Ok(Json(ApiResponse::err("认领支付失败，可能订单状态不允许")));
    }

    Ok(Json(ApiResponse::msg("个人支付认领成功")))
}

// ============================================================
// Address (地址簿)
// ============================================================

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetAddressesParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

pub async fn get_addresses(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(params): Json<GetAddressesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::ok_paginated(vec![], 0, page, page_size)));
    };

    let base_query = format!(r#"SELECT * FROM [tOnline_Address] WHERE [EmpID] = '{}' AND [State] <> 'D' ORDER BY [IsDefault] DESC, [EDate] DESC"#, emp_id);

    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let offset = (page - 1) * page_size;
    let top = offset + page_size;
    let paginated_sql = format!(
        "SELECT * FROM (SELECT TOP ({}) ROW_NUMBER() OVER (ORDER BY [IsDefault] DESC, [EDate] DESC) as _rn, * FROM [tOnline_Address] WHERE [EmpID] = @p1 AND [State] <> 'D') p WHERE _rn > {}",
        top, offset
    );

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &[]).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &[&emp_id.as_str()]).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateAddressParams {
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub is_default: Option<i32>,
}

pub async fn create_address(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("用户信息获取失败")));
    };

    let contact_name = body.contact_name.as_deref().unwrap_or("");
    let phone = body.phone.as_deref().unwrap_or("");
    let province = body.province.as_deref().unwrap_or("");
    let city = body.city.as_deref().unwrap_or("");
    let district = body.district.as_deref().unwrap_or("");
    let address = body.address.as_deref().unwrap_or("");
    let is_default_int: i32 = if body.is_default.unwrap_or(0) == 1 { 1 } else { 0 };

    if is_default_int == 1 {
        let clear_default_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 0 WHERE [EmpID] = @p1 AND [State] <> 'D'";
        conn.execute(clear_default_sql, &[&emp_id.as_str()]).await?;
    }

    let sql = r#"INSERT INTO [tOnline_Address] ([AddressID], [EmpID], [ContactName], [Phone], [Province], [City], [District], [Address], [IsDefault], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, 'A', @p9, @p10)"#;

    conn.execute(sql, &[
        &emp_id.as_str(),
        &contact_name,
        &phone,
        &province,
        &city,
        &district,
        &address,
        &is_default_int,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("地址创建成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UpdateAddressParams {
    pub id: String,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub province: Option<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub address: Option<String>,
    pub is_default: Option<i32>,
}

pub async fn update_address(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let contact_name = body.contact_name.as_deref().unwrap_or("");
    let phone = body.phone.as_deref().unwrap_or("");
    let province = body.province.as_deref().unwrap_or("");
    let city = body.city.as_deref().unwrap_or("");
    let district = body.district.as_deref().unwrap_or("");
    let address = body.address.as_deref().unwrap_or("");
    let is_default_int: i32 = if body.is_default.unwrap_or(0) == 1 { 1 } else { 0 };

    if is_default_int == 1 {
        let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
        let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
        if let Some(row) = emp_stream.into_row().await? {
            let emp_id = row.get::<&str, _>("EmpID").unwrap_or("").to_string();
            let clear_default_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 0 WHERE [EmpID] = @p1 AND [State] <> 'D'";
            conn.execute(clear_default_sql, &[&emp_id.as_str()]).await?;
        }
    }

    let sql = r#"UPDATE [tOnline_Address] SET
        [ContactName] = @p1, [Phone] = @p2, [Province] = @p3, [City] = @p4,
        [District] = @p5, [Address] = @p6, [IsDefault] = @p7, [EDate] = @p8, [EUser] = @p9
        WHERE [AddressID] = @p10 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &contact_name,
        &phone,
        &province,
        &city,
        &district,
        &address,
        &is_default_int,
        &now,
        &claims.user_code.as_str(),
        &body.id.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("地址更新成功")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeleteAddressParams {
    pub ids: Vec<String>,
}

pub async fn delete_address(
    State(_config): State<Config>,
    Json(body): Json<DeleteAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的地址")));
    }

    for id in &body.ids {
        let sql = "UPDATE [tOnline_Address] SET [State] = 'D' WHERE [AddressID] = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个地址", body.ids.len()))))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SetDefaultAddressParams {
    pub id: String,
}

pub async fn set_default_address(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SetDefaultAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    if let Some(row) = emp_stream.into_row().await? {
        let emp_id = row.get::<&str, _>("EmpID").unwrap_or("").to_string();
        let clear_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 0 WHERE [EmpID] = @p1 AND [State] <> 'D'";
        conn.execute(clear_sql, &[&emp_id.as_str()]).await?;
    }

    let set_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 1 WHERE [AddressID] = @p1 AND [State] <> 'D'";
    conn.execute(set_sql, &[&body.id.as_str()]).await?;

    Ok(Json(ApiResponse::msg("默认地址设置成功")))
}

// ============================================================
// Regions (省市区)
// ============================================================

pub async fn get_regions(
    State(_config): State<Config>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    Ok(Json(ApiResponse::ok(vec![])))
}
