use super::base_data::row_to_json;
use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use axum::{Json, extract::State};
use serde::Deserialize;
use tiberius::Row;

#[derive(Deserialize)]
pub struct PaginationParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn list_emp_sales(
    State(_config): State<Config>,
    Json(params): Json<PaginationParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(20), 1000);

    let mut base_query = "SELECT * FROM tSal_EmpSales WHERE State <> 'D'".to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let pidx = 1;

    if let Some(kw) = &params.keyword {
        if !kw.is_empty() {
            base_query.push_str(&format!(
                " AND (EmpNo LIKE @p{} OR EmpName LIKE @p{} OR GDSNO LIKE @p{} OR GDSDesc LIKE @p{})",
                pidx, pidx + 1, pidx + 2, pidx + 3
            ));
            query_params.push(Some(format!("%{}%", kw)));
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

    Ok(Json(ApiResponse::ok_paginated(
        data,
        total as u64,
        page,
        page_size,
    )))
}

fn json_str(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn json_opt_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn json_i32(v: &serde_json::Value, key: &str) -> i32 {
    v.get(key).and_then(|v| v.as_i64()).unwrap_or(0) as i32
}

pub async fn create_emp_sales(
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let emp_no = json_str(&body, "EmpNo");
    let emp_name = json_str(&body, "EmpName");
    let gdsno = json_str(&body, "GDSNO");
    let gdsdesc = json_str(&body, "GDSDesc");
    let qty = json_f64(&body, "Qty");
    let price = json_f64(&body, "Price");
    let amt = json_f64(&body, "Amt");
    let amt = if amt == 0.0 { qty * price } else { amt };
    let sale_date = json_opt_str(&body, "SaleDate")
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let state = json_opt_str(&body, "State").unwrap_or_else(|| "N".to_string());

    let sql = r#"INSERT INTO tSal_EmpSales (EmpNo, EmpName, GDSNO, GDSDesc, Qty, Price, Amt, SaleDate, State, EDate, EUser)
        VALUES (@p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8, @p9, @p10, @p11)"#;

    // P5 修复：tSal_EmpSales.EUser 是 uniqueidentifier 列，传 "system" 字符串会报
    //   "Conversion failed when converting from a character string to uniqueidentifier"
    //   该函数无 Extension<Claims> 注入，使用 ZERO_UUID 作为审计占位（列可空）
    const EUSER_PLACEHOLDER: &str = "00000000-0000-0000-0000-000000000000";
    conn.execute(
        sql,
        &[
            &emp_no.as_str(),
            &emp_name.as_str(),
            &gdsno.as_str(),
            &gdsdesc.as_str(),
            &qty,
            &price,
            &amt,
            &sale_date.as_str(),
            &state.as_str(),
            &now,
            &EUSER_PLACEHOLDER,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("员工销量录入成功")))
}

pub async fn update_emp_sales(
    State(_config): State<Config>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let id = json_i32(&body, "ID");
    if id == 0 {
        return Ok(Json(ApiResponse::err("记录ID不能为空")));
    }

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let emp_no = json_str(&body, "EmpNo");
    let emp_name = json_str(&body, "EmpName");
    let gdsno = json_str(&body, "GDSNO");
    let gdsdesc = json_str(&body, "GDSDesc");
    let qty = json_f64(&body, "Qty");
    let price = json_f64(&body, "Price");
    let amt = json_f64(&body, "Amt");
    let amt = if amt == 0.0 { qty * price } else { amt };
    let sale_date = json_opt_str(&body, "SaleDate")
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let state = json_opt_str(&body, "State").unwrap_or_else(|| "N".to_string());

    let sql = r#"UPDATE tSal_EmpSales SET EmpNo=@p1, EmpName=@p2, GDSNO=@p3, GDSDesc=@p4,
        Qty=@p5, Price=@p6, Amt=@p7, SaleDate=@p8, State=@p9, EDate=@p10, EUser=@p11 WHERE ID=@p12"#;

    conn.execute(
        sql,
        &[
            &emp_no.as_str(),
            &emp_name.as_str(),
            &gdsno.as_str(),
            &gdsdesc.as_str(),
            &qty,
            &price,
            &amt,
            &sale_date.as_str(),
            &state.as_str(),
            &now,
            &"system",
            &id,
        ],
    )
    .await?;

    Ok(Json(ApiResponse::msg("员工销量更新成功")))
}
