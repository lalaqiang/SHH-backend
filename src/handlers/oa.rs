use axum::extract::{State, Json};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::try_get_value;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
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

pub async fn get_workflow_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tSys_WorkFlow WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // 实际字段：WorkFlowNO / WorkFlowDesc（不是 FlowNo / FlowName）
            base_query.push_str(&format!(
                " AND (WorkFlowNO LIKE @p{} OR WorkFlowDesc LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
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
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

#[derive(Deserialize)]
pub struct WorkflowApproveRequest {
    pub WorkFlowID: String,  // 实际 PK 是 WorkFlowID，不是 FlowID
    pub Approved: bool,
    pub Remark: Option<String>,
}

pub async fn approve_workflow(
    State(_config): State<Config>,
    Json(params): Json<WorkflowApproveRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // ⚠️ 业务注意：tSys_WorkFlow 是工作流模板，State 表示模板状态
    // 实际审批实例走 tSys_WorkFlowProc / tSys_WorkFlowProcEmp / tSys_WorkFlowLog
    // 此处仅作模板级"启用/停用"，不做审批
    let new_state = if params.Approved { "Y" } else { "N" };
    let remark = params.Remark.as_deref().unwrap_or("");

    let sql = "UPDATE tSys_WorkFlow SET State = @p1, EDate = GETDATE() WHERE WorkFlowID = @p2";
    conn.execute(sql, &[&new_state, &params.WorkFlowID]).await?;

    let _ = remark; // 当前模板表无 Remark 字段，预留

    Ok(Json(ApiResponse::msg(if params.Approved { "工作流启用" } else { "工作流停用" })))
}

pub async fn get_notice_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tOA_InfoDetail WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (Title LIKE @p{} OR InfoType LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
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
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

pub async fn get_email_list(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tOA_Email WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (Subject LIKE @p{} OR Sender LIKE @p{})",
                pidx, pidx + 1
            ));
            pidx += 2;
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
    let count_stream = conn.query(&count_sql, &param_refs).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &param_refs).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}
