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
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

fn row_to_json(row: &Row) -> serde_json::Value {
    let columns = row.columns();
    let mut map = serde_json::Map::new();
    for col in columns {
        let name = col.name().to_string();
        if name == "_rn" {
            continue;
        }
        let val = try_get_value(row, &name);
        map.insert(name, val);
    }
    serde_json::Value::Object(map)
}

// ============================================================
// Online Products (商品池管理)
// ============================================================

#[derive(Deserialize)]
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

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
        pidx += 1;
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
pub struct CreateOnlineProductParams {
    pub GDSID: Option<String>,
    pub SaleType: Option<String>,
    pub ClearancePrice: Option<f64>,
    pub MaxOrderQty: Option<i32>,
    pub Sort: Option<i32>,
    pub Status: Option<i32>,
    pub StkID: Option<String>,
}

pub async fn create_online_product(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let gds_id = body.GDSID.as_deref().unwrap_or("");
    let sale_type = body.SaleType.as_deref().unwrap_or("normal");
    let clearance_price = body.ClearancePrice.unwrap_or(0.0);
    let max_order_qty = body.MaxOrderQty.unwrap_or(0);
    let sort = body.Sort.unwrap_or(0);
    let status = body.Status.unwrap_or(1);
    let stk_id = body.StkID.as_deref().unwrap_or("");

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
pub struct UpdateOnlineProductParams {
    pub OnlineGDSID: String,
    pub GDSID: Option<String>,
    pub SaleType: Option<String>,
    pub ClearancePrice: Option<f64>,
    pub MaxOrderQty: Option<i32>,
    pub Sort: Option<i32>,
    pub Status: Option<i32>,
    pub StkID: Option<String>,
}

pub async fn update_online_product(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let gds_id = body.GDSID.as_deref().unwrap_or("");
    let sale_type = body.SaleType.as_deref().unwrap_or("normal");
    let clearance_price = body.ClearancePrice.unwrap_or(0.0);
    let max_order_qty = body.MaxOrderQty.unwrap_or(0);
    let sort = body.Sort.unwrap_or(0);
    let status = body.Status.unwrap_or(1);
    let stk_id = body.StkID.as_deref().unwrap_or("");

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
        &body.OnlineGDSID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("线上商品更新成功")))
}

#[derive(Deserialize)]
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

    let mut base_query = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[GDSBarCode], s.[StkName],
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
            base_query.push_str(&format!(" AND (g.[GDSDesc] LIKE @p{} OR g.[GDSNO] LIKE @p{} OR g.[GDSBarCode] LIKE @p{})", pidx, pidx + 1, pidx + 2));
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
            pidx += 1;
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
pub struct BrowseOnlineProductParams {
    pub id: String,
}

pub async fn browse_online_product(
    State(_config): State<Config>,
    Json(params): Json<BrowseOnlineProductParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = r#"SELECT og.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[GDSBarCode], s.[StkName],
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
pub struct OrderItemInput {
    pub GDSID: String,
    pub Qty: i32,
    pub Price: f64,
}

#[derive(Deserialize)]
pub struct PlaceOnlineOrderParams {
    pub items: Vec<OrderItemInput>,
    pub AddressID: Option<String>,
    pub Remark: Option<String>,
    pub PaymentMethod: Option<String>,
}

pub async fn place_online_order(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<PlaceOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();
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

    let total_amt: f64 = body.items.iter().map(|item| item.Price * item.Qty as f64).sum();

    let address_id = body.AddressID.as_deref().unwrap_or("");
    let remark = body.Remark.as_deref().unwrap_or("");
    let payment_method = body.PaymentMethod.as_deref().unwrap_or("");

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        "".to_string()
    };

    let order_sql = r#"INSERT INTO [tOnline_Order] ([OrderID], [OrderNo], [EmpID], [AddressID], [TotalAmt], [Status], [PaymentStatus], [PaymentMethod], [Remark], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, 'pending', 'unpaid', @p5, @p6, 'A', @p7, @p8)"#;

    conn.execute(order_sql, &[
        &order_no.as_str(),
        &emp_id.as_str(),
        &address_id,
        &total_amt,
        &payment_method,
        &remark,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    let order_id_sql = "SELECT [OrderID] FROM [tOnline_Order] WHERE [OrderNo] = @p1";
    let oid_stream = conn.query(order_id_sql, &[&order_no.as_str()]).await?;
    let order_id = if let Some(row) = oid_stream.into_row().await? {
        row.get::<&str, _>("OrderID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("订单创建失败")));
    };

    for item in &body.items {
        let line_amt = item.Price * item.Qty as f64;
        let detail_sql = r#"INSERT INTO [tOnline_OrderDetail] ([DetailID], [OrderID], [GDSID], [Qty], [Price], [LineAmt], [State])
            VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, 'A')"#;
        conn.execute(detail_sql, &[
            &order_id.as_str(),
            &item.GDSID.as_str(),
            &item.Qty,
            &item.Price,
            &line_amt,
        ]).await?;
    }

    Ok(Json(ApiResponse::ok(serde_json::json!({ "OrderNo": order_no, "OrderID": order_id }))))
}

#[derive(Deserialize)]
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

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
            pidx += 1;
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

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
    let mut pidx = 2;

    if let Some(st) = &params.status {
        if !st.is_empty() {
            base_query.push_str(&format!(" AND o.[Status] = @p{}", pidx));
            pidx += 1;
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
        WHERE o.[OrderID] = @p1 AND o.[State] <> 'D'"#;
    let order_stream = conn.query(order_sql, &[&params.id.as_str()]).await?;

    let order_row = match order_stream.into_row().await? {
        Some(row) => row,
        None => return Ok(Json(ApiResponse::err("订单不存在"))),
    };
    let mut order_data = row_to_json(&order_row);

    let detail_sql = r#"SELECT od.*, g.[GDSDesc], g.[GDSNO], g.[GDSSpec], g.[GDSBarCode]
        FROM [tOnline_OrderDetail] od
        LEFT JOIN [tBas_Goods] g ON od.[GDSID] = g.[GDSID]
        WHERE od.[OrderID] = @p1 AND od.[State] <> 'D'"#;
    let detail_stream = conn.query(detail_sql, &[&params.id.as_str()]).await?;
    let detail_rows: Vec<Row> = detail_stream.into_first_result().await?;
    let details: Vec<serde_json::Value> = detail_rows.iter().map(row_to_json).collect();

    if let Some(obj) = order_data.as_object_mut() {
        obj.insert("items".to_string(), serde_json::Value::Array(details));
    }

    Ok(Json(ApiResponse::ok(order_data)))
}

#[derive(Deserialize)]
pub struct ConfirmOnlineOrderParams {
    pub id: String,
}

pub async fn confirm_online_order(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<ConfirmOnlineOrderParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let sql = r#"UPDATE [tOnline_Order] SET [Status] = 'confirmed', [EDate] = @p1, [EUser] = @p2
        WHERE [OrderID] = @p3 AND [State] <> 'D' AND [Status] = 'pending'"#;
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
    let now = chrono::Local::now().naive_local();

    let reason = body.reason.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_Order] SET [Status] = 'cancelled', [Remark] = ISNULL([Remark],'') + @p1, [EDate] = @p2, [EUser] = @p3
        WHERE [OrderID] = @p4 AND [State] <> 'D' AND [Status] IN ('pending', 'confirmed')"#;
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
pub struct UpdateOnlineOrderShipInfoParams {
    pub id: String,
    pub ShipCompany: Option<String>,
    pub ShipNo: Option<String>,
    pub ShipStatus: Option<String>,
}

pub async fn update_online_order_ship_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateOnlineOrderShipInfoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let ship_company = body.ShipCompany.as_deref().unwrap_or("");
    let ship_no = body.ShipNo.as_deref().unwrap_or("");
    let ship_status = body.ShipStatus.as_deref().unwrap_or("unshipped");

    let sql = r#"UPDATE [tOnline_Order] SET [ShipCompany] = @p1, [ShipNo] = @p2, [ShipStatus] = @p3, [EDate] = @p4, [EUser] = @p5
        WHERE [OrderID] = @p6 AND [State] <> 'D'"#;

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
pub struct BatchUpdateShipInfoItem {
    pub id: String,
    pub ShipCompany: Option<String>,
    pub ShipNo: Option<String>,
    pub ShipStatus: Option<String>,
}

#[derive(Deserialize)]
pub struct BatchUpdateOnlineOrderShipInfoParams {
    pub items: Vec<BatchUpdateShipInfoItem>,
}

pub async fn batch_update_online_order_ship_info(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<BatchUpdateOnlineOrderShipInfoParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    if body.items.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要更新的订单")));
    }

    let mut updated = 0u32;
    for item in &body.items {
        let ship_company = item.ShipCompany.as_deref().unwrap_or("");
        let ship_no = item.ShipNo.as_deref().unwrap_or("");
        let ship_status = item.ShipStatus.as_deref().unwrap_or("unshipped");

        let sql = r#"UPDATE [tOnline_Order] SET [ShipCompany] = @p1, [ShipNo] = @p2, [ShipStatus] = @p3, [EDate] = @p4, [EUser] = @p5
            WHERE [OrderID] = @p6 AND [State] <> 'D'"#;

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
pub struct BatchGenerateSalesOrdersParams {
    pub order_ids: Vec<String>,
}

pub async fn batch_generate_sales_orders(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<BatchGenerateSalesOrdersParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    if body.order_ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要生成销售单的订单")));
    }

    let mut generated = 0u32;
    let mut results: Vec<serde_json::Value> = Vec::new();

    for order_id in &body.order_ids {
        let order_sql = r#"SELECT o.[OrderID], o.[OrderNo], o.[EmpID], o.[TotalAmt]
            FROM [tOnline_Order] o
            WHERE o.[OrderID] = @p1 AND o.[State] <> 'D' AND o.[Status] = 'confirmed'"#;
        let order_stream = conn.query(order_sql, &[&order_id.as_str()]).await?;

        let order_row = match order_stream.into_row().await? {
            Some(row) => row,
            None => continue,
        };

        let order_no: &str = order_row.get::<&str, _>("OrderNo").unwrap_or("");
        let emp_id: &str = order_row.get::<&str, _>("EmpID").unwrap_or("");
        let total_amt: f64 = row_get_f64(&order_row, "TotalAmt");

        let sal_no = format!("SOL{}", &order_no[2..]);

        let sal_sql = r#"INSERT INTO [tSal_Inv] ([InvID], [InvNO], [CustID], [TotalAmt], [Kind], [State], [EDate], [EUser])
            VALUES (NEWID(), @p1, @p2, @p3, 'POS', 'N', @p4, @p5)"#;
        conn.execute(sal_sql, &[
            &sal_no.as_str(),
            &emp_id,
            &total_amt,
            &now,
            &claims.user_code.as_str(),
        ]).await?;

        let detail_sql = r#"SELECT [GDSID], [Qty], [Price], [LineAmt]
            FROM [tOnline_OrderDetail]
            WHERE [OrderID] = @p1 AND [State] <> 'D'"#;
        let detail_stream = conn.query(detail_sql, &[&order_id.as_str()]).await?;
        let detail_rows: Vec<Row> = detail_stream.into_first_result().await?;

        let sal_inv_id_sql = "SELECT [InvID] FROM [tSal_Inv] WHERE [InvNO] = @p1";
        let sal_inv_stream = conn.query(sal_inv_id_sql, &[&sal_no.as_str()]).await?;
        let sal_inv_id = if let Some(row) = sal_inv_stream.into_row().await? {
            row.get::<&str, _>("InvID").unwrap_or("").to_string()
        } else {
            continue;
        };

        for dr in &detail_rows {
            let gds_id: &str = dr.get::<&str, _>("GDSID").unwrap_or("");
            let qty: i32 = dr.get::<i32, _>("Qty").unwrap_or(0);
            let price: f64 = row_get_f64(&dr, "Price");
            let line_amt: f64 = row_get_f64(&dr, "LineAmt");

            let sal_detail_sql = r#"INSERT INTO [tSal_InvDetail] ([DetailID], [InvID], [GDSID], [Qty], [Price], [LineAmt], [State])
                VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, 'A')"#;
            conn.execute(sal_detail_sql, &[
                &sal_inv_id.as_str(),
                &gds_id,
                &qty,
                &price,
                &line_amt,
            ]).await?;
        }

        let update_order_sql = r#"UPDATE [tOnline_Order] SET [Status] = 'processed', [EDate] = @p1, [EUser] = @p2
            WHERE [OrderID] = @p3 AND [State] <> 'D'"#;
        conn.execute(update_order_sql, &[
            &now,
            &claims.user_code.as_str(),
            &order_id.as_str(),
        ]).await?;

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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

    let mut base_query = r#"SELECT * FROM [tOnline_PaymentConfig] WHERE [State] <> 'D'"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(" AND ([PayName] LIKE @p{} OR [PayCode] LIKE @p{})", pidx, pidx + 1));
            pidx += 2;
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
pub struct GetPaymentConfigParams {
    pub id: String,
}

pub async fn get_payment_config(
    State(_config): State<Config>,
    Json(params): Json<GetPaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT * FROM [tOnline_PaymentConfig] WHERE [PayConfigID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(sql, &[&params.id.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("支付配置不存在")))
    }
}

#[derive(Deserialize)]
pub struct CreatePaymentConfigParams {
    pub PayCode: Option<String>,
    pub PayName: Option<String>,
    pub PayType: Option<String>,
    pub PayDesc: Option<String>,
    pub Sort: Option<i32>,
    pub Enabled: Option<String>,
    pub Config: Option<String>,
}

pub async fn create_payment_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let pay_code = body.PayCode.as_deref().unwrap_or("");
    let pay_name = body.PayName.as_deref().unwrap_or("");
    let pay_type = body.PayType.as_deref().unwrap_or("");
    let pay_desc = body.PayDesc.as_deref().unwrap_or("");
    let sort = body.Sort.unwrap_or(0);
    let enabled = body.Enabled.as_deref().unwrap_or("Y");
    let config = body.Config.as_deref().unwrap_or("");

    let sql = r#"INSERT INTO [tOnline_PaymentConfig] ([PayConfigID], [PayCode], [PayName], [PayType], [PayDesc], [Sort], [Enabled], [Config], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, 'A', @p8, @p9)"#;

    conn.execute(sql, &[
        &pay_code,
        &pay_name,
        &pay_type,
        &pay_desc,
        &sort,
        &enabled,
        &config,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付配置创建成功")))
}

#[derive(Deserialize)]
pub struct UpdatePaymentConfigParams {
    pub PayConfigID: String,
    pub PayCode: Option<String>,
    pub PayName: Option<String>,
    pub PayType: Option<String>,
    pub PayDesc: Option<String>,
    pub Sort: Option<i32>,
    pub Enabled: Option<String>,
    pub Config: Option<String>,
}

pub async fn update_payment_config(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdatePaymentConfigParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let pay_code = body.PayCode.as_deref().unwrap_or("");
    let pay_name = body.PayName.as_deref().unwrap_or("");
    let pay_type = body.PayType.as_deref().unwrap_or("");
    let pay_desc = body.PayDesc.as_deref().unwrap_or("");
    let sort = body.Sort.unwrap_or(0);
    let enabled = body.Enabled.as_deref().unwrap_or("Y");
    let config = body.Config.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_PaymentConfig] SET
        [PayCode] = @p1, [PayName] = @p2, [PayType] = @p3, [PayDesc] = @p4,
        [Sort] = @p5, [Enabled] = @p6, [Config] = @p7, [EDate] = @p8, [EUser] = @p9
        WHERE [PayConfigID] = @p10 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &pay_code,
        &pay_name,
        &pay_type,
        &pay_desc,
        &sort,
        &enabled,
        &config,
        &now,
        &claims.user_code.as_str(),
        &body.PayConfigID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付配置更新成功")))
}

#[derive(Deserialize)]
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
        let sql = "UPDATE [tOnline_PaymentConfig] SET [State] = 'D' WHERE [PayConfigID] = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个支付配置", body.ids.len()))))
}

pub async fn get_available_payment_methods(
    State(_config): State<Config>,
    Json(_params): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT [PayConfigID], [PayCode], [PayName], [PayType], [PayDesc], [Sort] FROM [tOnline_PaymentConfig] WHERE [State] <> 'D' AND [Enabled] = 'Y' ORDER BY [Sort]";
    let stream = conn.query(sql, &[]).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct CreateOnlineOrderPaymentParams {
    pub OrderID: String,
    pub PayConfigID: Option<String>,
    pub PayMethod: Option<String>,
}

pub async fn create_online_order_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateOnlineOrderPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let pay_config_id = body.PayConfigID.as_deref().unwrap_or("");
    let pay_method = body.PayMethod.as_deref().unwrap_or("");

    let check_sql = "SELECT [OrderID], [Status], [PaymentStatus] FROM [tOnline_Order] WHERE [OrderID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(check_sql, &[&body.OrderID.as_str()]).await?;

    let order_row = match stream.into_row().await? {
        Some(row) => row,
        None => return Ok(Json(ApiResponse::err("订单不存在"))),
    };

    let payment_status: &str = order_row.get::<&str, _>("PaymentStatus").unwrap_or("");
    if payment_status == "paid" {
        return Ok(Json(ApiResponse::err("订单已支付")));
    }

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'pending', [PaymentMethod] = @p1, [PayConfigID] = @p2, [EDate] = @p3, [EUser] = @p4
        WHERE [OrderID] = @p5 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &pay_method,
        &pay_config_id,
        &now,
        &claims.user_code.as_str(),
        &body.OrderID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::ok(serde_json::json!({
        "OrderID": body.OrderID,
        "PaymentStatus": "pending"
    }))))
}

#[derive(Deserialize)]
pub struct QueryPaymentStatusParams {
    pub OrderID: String,
}

pub async fn query_payment_status(
    State(_config): State<Config>,
    Json(params): Json<QueryPaymentStatusParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let sql = "SELECT [OrderID], [OrderNo], [PaymentStatus], [PaymentMethod], [PaymentProof], [PayTime] FROM [tOnline_Order] WHERE [OrderID] = @p1 AND [State] <> 'D'";
    let stream = conn.query(sql, &[&params.OrderID.as_str()]).await?;

    if let Some(row) = stream.into_row().await? {
        Ok(Json(ApiResponse::ok(row_to_json(&row))))
    } else {
        Ok(Json(ApiResponse::err("订单不存在")))
    }
}

#[derive(Deserialize)]
pub struct UploadPaymentProofParams {
    pub OrderID: String,
    pub PaymentProof: Option<String>,
}

pub async fn upload_payment_proof(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UploadPaymentProofParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let payment_proof = body.PaymentProof.as_deref().unwrap_or("");

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentProof] = @p1, [PaymentStatus] = 'proof_uploaded', [EDate] = @p2, [EUser] = @p3
        WHERE [OrderID] = @p4 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &payment_proof,
        &now,
        &claims.user_code.as_str(),
        &body.OrderID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("支付凭证上传成功")))
}

#[derive(Deserialize)]
pub struct VerifyPaymentParams {
    pub OrderID: String,
    pub verified: Option<bool>,
}

pub async fn verify_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<VerifyPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let new_status = if body.verified.unwrap_or(true) { "paid" } else { "verify_failed" };

    let sql = if new_status == "paid" {
        r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'paid', [PayTime] = @p1, [EDate] = @p1, [EUser] = @p2
            WHERE [OrderID] = @p3 AND [State] <> 'D'"#
    } else {
        r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'verify_failed', [EDate] = @p1, [EUser] = @p2
            WHERE [OrderID] = @p3 AND [State] <> 'D'"#
    };

    conn.execute(sql, &[
        &now,
        &claims.user_code.as_str(),
        &body.OrderID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg(if new_status == "paid" { "支付验证通过" } else { "支付验证未通过" })))
}

#[derive(Deserialize)]
pub struct ClaimPersonalPaymentParams {
    pub OrderID: String,
}

pub async fn claim_personal_payment(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<ClaimPersonalPaymentParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("用户信息获取失败")));
    };

    let sql = r#"UPDATE [tOnline_Order] SET [PaymentStatus] = 'paid', [PayTime] = @p1, [PaymentMethod] = 'personal', [EDate] = @p1, [EUser] = @p2
        WHERE [OrderID] = @p3 AND [EmpID] = @p4 AND [State] <> 'D' AND [PaymentStatus] IN ('unpaid', 'pending', 'proof_uploaded')"#;

    let result = conn.execute(sql, &[
        &now,
        &claims.user_code.as_str(),
        &body.OrderID.as_str(),
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
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 200);

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
pub struct CreateAddressParams {
    pub ReceiverName: Option<String>,
    pub Phone: Option<String>,
    pub Province: Option<String>,
    pub City: Option<String>,
    pub District: Option<String>,
    pub DetailAddr: Option<String>,
    pub IsDefault: Option<String>,
}

pub async fn create_address(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreateAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
    let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
    let emp_id = if let Some(row) = emp_stream.into_row().await? {
        row.get::<&str, _>("EmpID").unwrap_or("").to_string()
    } else {
        return Ok(Json(ApiResponse::err("用户信息获取失败")));
    };

    let receiver_name = body.ReceiverName.as_deref().unwrap_or("");
    let phone = body.Phone.as_deref().unwrap_or("");
    let province = body.Province.as_deref().unwrap_or("");
    let city = body.City.as_deref().unwrap_or("");
    let district = body.District.as_deref().unwrap_or("");
    let detail_addr = body.DetailAddr.as_deref().unwrap_or("");
    let is_default = body.IsDefault.as_deref().unwrap_or("N");

    if is_default == "Y" {
        let clear_default_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 'N' WHERE [EmpID] = @p1 AND [State] <> 'D'";
        conn.execute(clear_default_sql, &[&emp_id.as_str()]).await?;
    }

    let sql = r#"INSERT INTO [tOnline_Address] ([AddressID], [EmpID], [ReceiverName], [Phone], [Province], [City], [District], [DetailAddr], [IsDefault], [State], [EDate], [EUser])
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, 'A', @p9, @p10)"#;

    conn.execute(sql, &[
        &emp_id.as_str(),
        &receiver_name,
        &phone,
        &province,
        &city,
        &district,
        &detail_addr,
        &is_default,
        &now,
        &claims.user_code.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("地址创建成功")))
}

#[derive(Deserialize)]
pub struct UpdateAddressParams {
    pub AddressID: String,
    pub ReceiverName: Option<String>,
    pub Phone: Option<String>,
    pub Province: Option<String>,
    pub City: Option<String>,
    pub District: Option<String>,
    pub DetailAddr: Option<String>,
    pub IsDefault: Option<String>,
}

pub async fn update_address(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdateAddressParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().naive_local();

    let receiver_name = body.ReceiverName.as_deref().unwrap_or("");
    let phone = body.Phone.as_deref().unwrap_or("");
    let province = body.Province.as_deref().unwrap_or("");
    let city = body.City.as_deref().unwrap_or("");
    let district = body.District.as_deref().unwrap_or("");
    let detail_addr = body.DetailAddr.as_deref().unwrap_or("");
    let is_default = body.IsDefault.as_deref().unwrap_or("N");

    if is_default == "Y" {
        let emp_sql = "SELECT TOP 1 [EmpID] FROM [tBas_Emp] WHERE [EmpNo] = @p1 AND [State] <> 'D'";
        let emp_stream = conn.query(emp_sql, &[&claims.user_code.as_str()]).await?;
        if let Some(row) = emp_stream.into_row().await? {
            let emp_id = row.get::<&str, _>("EmpID").unwrap_or("").to_string();
            let clear_default_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 'N' WHERE [EmpID] = @p1 AND [State] <> 'D'";
            conn.execute(clear_default_sql, &[&emp_id.as_str()]).await?;
        }
    }

    let sql = r#"UPDATE [tOnline_Address] SET
        [ReceiverName] = @p1, [Phone] = @p2, [Province] = @p3, [City] = @p4,
        [District] = @p5, [DetailAddr] = @p6, [IsDefault] = @p7, [EDate] = @p8, [EUser] = @p9
        WHERE [AddressID] = @p10 AND [State] <> 'D'"#;

    conn.execute(sql, &[
        &receiver_name,
        &phone,
        &province,
        &city,
        &district,
        &detail_addr,
        &is_default,
        &now,
        &claims.user_code.as_str(),
        &body.AddressID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("地址更新成功")))
}

#[derive(Deserialize)]
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
        let clear_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 'N' WHERE [EmpID] = @p1 AND [State] <> 'D'";
        conn.execute(clear_sql, &[&emp_id.as_str()]).await?;
    }

    let set_sql = "UPDATE [tOnline_Address] SET [IsDefault] = 'Y' WHERE [AddressID] = @p1 AND [State] <> 'D'";
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
