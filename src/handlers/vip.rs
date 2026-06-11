use axum::{
    extract::State,
    Json,
};
use serde::Deserialize;
use tiberius::Row;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use super::base_data::{try_get_value, row_to_json};

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn list_vip(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 100);

    let mut base_query = "SELECT * FROM tSal_VIP WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (VIPNo LIKE @p{} OR VIPName LIKE @p{} OR Phone LIKE @p{})",
                pidx, pidx + 1, pidx + 2
            ));
            pidx += 3;
            query_params.push(Some(format!("%{}%", kw)));
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

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key)
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn json_opt_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn json_i32(v: &serde_json::Value, key: &str) -> i32 {
    v.get(key)
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32
}

pub async fn create_vip(
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let now = chrono::Local::now().naive_local();
    let vip_no = json_str(&body, "VIPNo");
    let vip_no = if vip_no.is_empty() {
        format!("VIP{}", chrono::Local::now().format("%Y%m%d%H%M%S"))
    } else {
        vip_no
    };
    let vip_name = json_str(&body, "VIPName");
    let phone = json_str(&body, "Phone");
    let balance = json_f64(&body, "Balance");
    let points = json_i32(&body, "Points");
    let level = json_opt_str(&body, "Level").unwrap_or_else(|| "普通会员".to_string());

    let sql = r#"INSERT INTO tSal_VIP (VIPNo, VIPName, Phone, Balance, Points, Level, State, EDate, EUser)
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9)"#;

    conn.execute(sql, &[
        &vip_no.as_str(),
        &vip_name.as_str(),
        &phone.as_str(),
        &balance,
        &points,
        &level.as_str(),
        &"S",
        &now,
        &"system",
    ]).await?;

    Ok(Json(ApiResponse::msg("会员创建成功")))
}

pub async fn update_vip(
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let id = json_i32(&body, "ID");
    if id == 0 {
        return Ok(Json(ApiResponse::err("记录ID不能为空")));
    }

    let now = chrono::Local::now().naive_local();
    let vip_no = json_str(&body, "VIPNo");
    let vip_name = json_str(&body, "VIPName");
    let phone = json_str(&body, "Phone");
    let balance = json_f64(&body, "Balance");
    let points = json_i32(&body, "Points");
    let level = json_opt_str(&body, "Level").unwrap_or_else(|| "普通会员".to_string());

    let sql = r#"UPDATE tSal_VIP SET VIPNo=@p1, VIPName=@p2, Phone=@p3, Balance=@p4,
        Points=@p5, Level=@p6, EDate=@p7, EUser=@p8 WHERE ID=@p9"#;

    conn.execute(sql, &[
        &vip_no.as_str(),
        &vip_name.as_str(),
        &phone.as_str(),
        &balance,
        &points,
        &level.as_str(),
        &now,
        &"system",
        &id,
    ]).await?;

    Ok(Json(ApiResponse::msg("会员信息更新成功")))
}

#[derive(Deserialize)]
pub struct DeleteRequest {
    pub ids: Vec<String>,
}

pub async fn delete_vip(
    State(_config): State<Config>,
    Json(body): Json<DeleteRequest>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的记录")));
    }

    for id in &body.ids {
        let sql = "UPDATE tSal_VIP SET State = 'D', EDate = @p1, EUser = @p2 WHERE ID = @p3";
        let now = chrono::Local::now().naive_local();
        let id_str = id.as_str();
        conn.execute(sql, &[&now, &"system", &id_str]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}条会员记录", body.ids.len()))))
}
