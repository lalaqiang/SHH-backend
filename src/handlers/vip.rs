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
use super::base_data::row_to_json;

const ZERO_UUID: &str = "00000000-0000-0000-0000-000000000000";

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
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = "SELECT * FROM tSal_VIP WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            // 对齐 tSal_VIP 实际字段：VIPCode/VIPName/Tel
            base_query.push_str(&format!(
                " AND (VIPCode LIKE @p{} OR VIPName LIKE @p{} OR Tel LIKE @p{})",
                pidx, pidx + 1, pidx + 2
            ));
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

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    // 对齐 tSal_VIP 实际字段：VIPID/VIPCode/VIPName/Tel/VIPLevel/SumAmt/SumInt/OutInt/StartDate/State/VIPTypeID/vipsd
    // 前端字段映射：VIPNo→VIPCode, Phone→Tel, Points→SumInt, Level→VIPLevel
    let vip_code = json_str(&body, "VIPNo");
    let vip_code = if vip_code.is_empty() {
        format!("VIP{}", chrono::Local::now().format("%Y%m%d%H%M%S"))
    } else {
        vip_code
    };
    let vip_name = json_str(&body, "VIPName");
    let tel = json_str(&body, "Phone");
    let vip_level = json_i32(&body, "Level");
    let sum_int = json_f64(&body, "Points"); // 前端 Points → 数据库 SumInt（累计积分）

    // 必填字段：VIPID(主键), vipsd, OutInt, State
    let sql = r#"INSERT INTO tSal_VIP (VIPID, VIPCode, VIPName, Tel, VIPLevel, SumAmt, SumInt, OutInt,
        StartDate, State, VIPTypeID, vipsd, StkID, EmpID)
        VALUES (NEWID(), @p1, @p2, @p3, @p4, 0, @p5, 0, @p6, @p7, @p8, 0, @p9, @p10)"#;

    conn.execute(sql, &[
        &vip_code.as_str(),
        &vip_name.as_str(),
        &tel.as_str(),
        &vip_level,
        &sum_int,
        &now,
        &"S",
        &ZERO_UUID, // VIPTypeID 默认
        &ZERO_UUID, // StkID
        &ZERO_UUID, // EmpID
    ]).await?;

    Ok(Json(ApiResponse::msg("会员创建成功")))
}

pub async fn update_vip(
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    // 对齐 tSal_VIP 实际字段：主键为 VIPID（uniqueidentifier），非 ID
    let vip_id = json_str(&body, "VIPID");
    if vip_id.is_empty() {
        // 兼容前端可能传的 ID 字段
        let id_alt = json_str(&body, "ID");
        if id_alt.is_empty() {
            return Ok(Json(ApiResponse::err("VIPID 不能为空")));
        }
        // 如果 ID 不是 UUID 格式，返回错误
        if id_alt.len() != 36 {
            return Ok(Json(ApiResponse::err("VIPID 格式错误（需 UUID 格式）")));
        }
        update_vip_by_id(&mut conn, &id_alt, &body).await
    } else {
        update_vip_by_id(&mut conn, &vip_id, &body).await
    }
}

async fn update_vip_by_id(
    conn: &mut crate::handlers::approval::Conn,
    vip_id: &str,
    body: &serde_json::Value,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    // 对齐 tSal_VIP 实际字段：VIPCode/VIPName/Tel/VIPLevel/SumInt
    // 前端字段映射：VIPNo→VIPCode, Phone→Tel, Points→SumInt, Level→VIPLevel
    let vip_code = json_str(body, "VIPNo");
    let vip_name = json_str(body, "VIPName");
    let tel = json_str(body, "Phone");
    let vip_level = json_i32(body, "Level");
    let sum_int = json_f64(body, "Points");

    let sql = r#"UPDATE tSal_VIP SET VIPCode=@p1, VIPName=@p2, Tel=@p3, VIPLevel=@p4,
        SumInt=@p5 WHERE VIPID=@p6"#;

    conn.execute(sql, &[
        &vip_code.as_str(),
        &vip_name.as_str(),
        &tel.as_str(),
        &vip_level,
        &sum_int,
        &vip_id,
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

    // 对齐 tSal_VIP 实际字段：主键为 VIPID，无 EDate/EUser 字段
    for id in &body.ids {
        let sql = "UPDATE tSal_VIP SET State = 'D' WHERE VIPID = @p1";
        let id_str = id.as_str();
        conn.execute(sql, &[&id_str]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}条会员记录", body.ids.len()))))
}
