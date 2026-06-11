use axum::extract::{State, Json, Extension};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::try_get_value;
use crate::middleware::auth::Claims;

#[derive(Deserialize)]
pub struct WorkspaceParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

#[derive(Deserialize)]
pub struct CommonMenusParams {
    pub menus: Option<String>,
}

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

/// POST /api/workspace/todo - 获取待审批单据列表
/// 查询 tPur_Order, tSal_Order, tStk_IO, tStk_Move, tSal_Inv 中 State='N' 的单据
pub async fn get_todo_list(
    State(_config): State<Config>,
    Extension(_claims): Extension<Claims>,
    Json(params): Json<WorkspaceParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    // 使用 UNION ALL 合并各表的新建单据，统一返回 docType, docNo, docDate, amount, state
    let base_query = r#"
        SELECT '采购订单' AS docType, PoNo AS docNo, PoDate AS docDate, TotalAmt AS amount, State AS state FROM tPur_Order WHERE State = 'N'
        UNION ALL
        SELECT '销售订单' AS docType, SoNo AS docNo, SoDate AS docDate, TotalAmt AS amount, State AS state FROM tSal_Order WHERE State = 'N'
        UNION ALL
        SELECT '入出库单' AS docType, IONo AS docNo, IODate AS docDate, TotalAmt AS amount, State AS state FROM tStk_IO WHERE State = 'N'
        UNION ALL
        SELECT '调拨单' AS docType, MoveNo AS docNo, MoveDate AS docDate, TotalAmt AS amount, State AS state FROM tStk_Move WHERE State = 'N'
        UNION ALL
        SELECT '销售发票' AS docType, InvNo AS docNo, InvDate AS docDate, TotalAmt AS amount, State AS state FROM tSal_Inv WHERE State = 'N'
    "#.trim();

    let mut query_params: Vec<Option<String>> = Vec::new();

    // 如果有关键词搜索，需要在外层包装 WHERE 条件
    let filtered_query = if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            format!(
                "SELECT * FROM ({}) t WHERE t.docNo LIKE @p1 OR t.docType LIKE @p2",
                base_query
            )
        } else {
            base_query.to_string()
        }
    } else {
        base_query.to_string()
    };

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", filtered_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &filtered_query,
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
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

/// POST /api/workspace/doing - 获取当前用户正在处理的单据列表
/// 查询各表中 State='E' 且 EUser 为当前用户的单据
pub async fn get_doing_list(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<WorkspaceParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);
    let user_code = claims.user_code.clone();

    let base_query = r#"
        SELECT '采购订单' AS docType, PoNo AS docNo, PoDate AS docDate, TotalAmt AS amount, State AS state FROM tPur_Order WHERE State = 'E' AND EUser = @p1
        UNION ALL
        SELECT '销售订单' AS docType, SoNo AS docNo, SoDate AS docDate, TotalAmt AS amount, State AS state FROM tSal_Order WHERE State = 'E' AND EUser = @p1
        UNION ALL
        SELECT '入出库单' AS docType, IONo AS docNo, IODate AS docDate, TotalAmt AS amount, State AS state FROM tStk_IO WHERE State = 'E' AND EUser = @p1
        UNION ALL
        SELECT '调拨单' AS docType, MoveNo AS docNo, MoveDate AS docDate, TotalAmt AS amount, State AS state FROM tStk_Move WHERE State = 'E' AND EUser = @p1
        UNION ALL
        SELECT '销售发票' AS docType, InvNo AS docNo, InvDate AS docDate, TotalAmt AS amount, State AS state FROM tSal_Inv WHERE State = 'E' AND EUser = @p1
    "#.trim();

    let mut query_params: Vec<Option<String>> = vec![Some(user_code)];
    let mut pidx = 2;

    // 如果有关键词搜索，需要在外层包装 WHERE 条件
    let filtered_query = if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            query_params.push(Some(format!("%{}%", kw)));
            query_params.push(Some(format!("%{}%", kw)));
            let result = format!(
                "SELECT * FROM ({}) t WHERE t.docNo LIKE @p{} OR t.docType LIKE @p{}",
                base_query, pidx, pidx + 1
            );
            pidx += 2;
            result
        } else {
            base_query.to_string()
        }
    } else {
        base_query.to_string()
    };

    let count_sql = format!("SELECT COUNT(*) AS cnt FROM ({}) t", filtered_query);
    let paginated_sql = build_pagination_sql_with_sort(
        &filtered_query,
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
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

/// POST /api/workspace/common-menus - 获取/保存常用菜单
/// 如果请求体中包含 menus 字段，则保存；否则返回常用菜单列表
pub async fn get_common_menus(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<CommonMenusParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let user_code = claims.user_code.clone();
    let now = chrono::Local::now().naive_local();

    // 如果传了 menus，则保存/更新
    if let Some(menus) = &params.menus {
        // 先检查是否已有记录
        let check_sql = "SELECT COUNT(*) AS cnt FROM tSys_Parameters WHERE PKind = 'common_menus' AND EUser = @p1";
        let check_params: Vec<&dyn tiberius::ToSql> = vec![&user_code];
        let check_stream = conn.query(check_sql, &check_params).await?;
        let mut count: i32 = 0;
        if let Some(row) = check_stream.into_row().await? {
            count = row.get::<i32, _>("cnt").unwrap_or(0);
        }

        if count > 0 {
            // 更新
            let update_sql = "UPDATE tSys_Parameters SET PValue = @p1, EDate = @p2 WHERE PKind = 'common_menus' AND EUser = @p3";
            let update_params: Vec<&dyn tiberius::ToSql> = vec![menus, &now, &user_code];
            conn.execute(update_sql, &update_params).await?;
        } else {
            // 新增
            let pkey = format!("common_menus_{}", user_code);
            let insert_sql = r#"INSERT INTO tSys_Parameters (PKind, PKey, PValue, PDesc, EDate, EUser)
                                VALUES (@p1, @p2, @p3, @p4, @p5, @p6)"#;
            let pkind = "common_menus";
            let pdesc = "用户常用菜单";
            let insert_params: Vec<&dyn tiberius::ToSql> = vec![
                &pkind,
                &pkey,
                menus,
                &pdesc,
                &now,
                &user_code,
            ];
            conn.execute(insert_sql, &insert_params).await?;
        }

        return Ok(Json(ApiResponse::msg("常用菜单保存成功")));
    }

    // 查询常用菜单
    let query_sql = "SELECT PValue FROM tSys_Parameters WHERE PKind = 'common_menus' AND EUser = @p1";
    let query_params: Vec<&dyn tiberius::ToSql> = vec![&user_code];
    let stream = conn.query(query_sql, &query_params).await?;

    if let Some(row) = stream.into_row().await? {
        let pvalue: String = row.get::<&str, _>("PValue").unwrap_or("").to_string();
        if !pvalue.is_empty() {
            // 尝试解析为 JSON 数组
            if let Ok(arr) = serde_json::from_str::<serde_json::Value>(&pvalue) {
                return Ok(Json(ApiResponse::ok(arr)));
            }
        }
    }

    // 没有记录则返回默认菜单
    let default_menus = serde_json::json!([
        { "name": "采购订单", "path": "/purchase/order", "icon": "ShoppingCart" },
        { "name": "销售订单", "path": "/sales/order", "icon": "Sell" },
        { "name": "入出库单", "path": "/inventory/io", "icon": "Box" },
        { "name": "库存查询", "path": "/inventory/stock", "icon": "Search" },
        { "name": "商品资料", "path": "/base/goods", "icon": "Goods" },
        { "name": "客户资料", "path": "/base/customer", "icon": "User" },
        { "name": "供应商资料", "path": "/base/supplier", "icon": "OfficeBuilding" },
        { "name": "报表中心", "path": "/report/index", "icon": "DataAnalysis" },
    ]);
    Ok(Json(ApiResponse::ok(default_menus)))
}
