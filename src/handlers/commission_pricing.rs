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
use crate::utils::{ApiResponse, build_pagination_sql_with_sort};
use crate::handlers::base_data::{row_to_json};
use crate::middleware::auth::Claims;

// =====================================================================
// 定价模板（pricing）相关 handler
// 注：提成模板（commission）的 CRUD 已统一改用 /generic/* 接口操作
//     tSys_Parameters 表，原 commission 端点已删除（避免重复维护）
// =====================================================================

#[derive(Deserialize)]
pub struct GetPricingTemplatesParams {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub keyword: Option<String>,
    pub sort_prop: Option<String>,
    pub sort_order: Option<String>,
}

pub async fn get_pricing_templates(
    State(_config): State<Config>,
    Json(params): Json<GetPricingTemplatesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;
    let page = params.page.unwrap_or(1);
    let page_size = std::cmp::min(params.page_size.unwrap_or(50), 1000);

    let base_query = "SELECT p.ParametersID, p.PCode, p.PName AS TemplateName, p.PKind AS PriceType, p.PValue AS Rate, p.PHelp AS Remark, p.EDate, p.EUser FROM tSys_Parameters p WHERE p.PKind = 'pricing' AND p.State <> 'D'";
    let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) t", base_query);
    let paginated_sql = build_pagination_sql_with_sort(base_query, page, page_size, params.sort_prop.as_deref(), params.sort_order.as_deref());

    let mut total: i32 = 0;
    let count_stream = conn.query(&count_sql, &[]).await?;
    if let Some(row) = count_stream.into_row().await? {
        total = row.get::<i32, _>("cnt").unwrap_or(0);
    }

    let data_stream = conn.query(&paginated_sql, &[]).await?;
    let rows: Vec<Row> = data_stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok_paginated(data, total as u64, page, page_size)))
}

#[derive(Deserialize)]
pub struct CreatePricingTemplateParams {
    pub PCode: Option<String>,
    pub PName: Option<String>,
    pub PKind: Option<String>,
    pub PTerm: Option<String>,
    pub PHelp: Option<String>,
    pub PValue: Option<String>,
}

pub async fn create_pricing_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<CreatePricingTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let p_code = body.PCode.as_deref().unwrap_or("");
    let p_name = body.PName.as_deref().unwrap_or("");
    let p_kind = body.PKind.as_deref().unwrap_or("pricing");
    let p_term = body.PTerm.as_deref().unwrap_or("ALL");
    let p_help = body.PHelp.as_deref().unwrap_or("");
    let p_value = body.PValue.as_deref().unwrap_or("{}");
    // ★ EUser 是 uniqueidentifier 列，必须传 UUID 字符串（claims.emp_id 是 UUID）
    //   claims.user_code 是 "admin" 等业务编码，SQL Server 无法转换为 uniqueidentifier
    let e_user: &str = if !claims.emp_id.is_empty() { claims.emp_id.as_str() } else { return Ok(Json(ApiResponse::err("登录用户缺少 EmpID，无法写入审计字段"))) };

    if p_name.is_empty() {
        return Ok(Json(ApiResponse::err("模板名称不能为空")));
    }

    let sql = r#"INSERT INTO tSys_Parameters (ParametersID, PCode, PName, PKind, PTerm, PHelp, PValue, EUser, EDate)
        VALUES (NEWID(), @p1, @p2, @p3, @p4, @p5, @p6, @p7, @p8)"#;

    conn.execute(sql, &[
        &p_code,
        &p_name,
        &p_kind,
        &p_term,
        &p_help,
        &p_value,
        &e_user,
        &now,
    ]).await?;

    Ok(Json(ApiResponse::msg("定价模板创建成功")))
}

#[derive(Deserialize)]
pub struct UpdatePricingTemplateParams {
    pub ParametersID: String,
    pub PCode: Option<String>,
    pub PName: Option<String>,
    pub PKind: Option<String>,
    pub PTerm: Option<String>,
    pub PHelp: Option<String>,
    pub PValue: Option<String>,
}

pub async fn update_pricing_template(
    Extension(claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<UpdatePricingTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let p_code = body.PCode.as_deref().unwrap_or("");
    let p_name = body.PName.as_deref().unwrap_or("");
    let p_kind = body.PKind.as_deref().unwrap_or("pricing");
    let p_term = body.PTerm.as_deref().unwrap_or("ALL");
    let p_help = body.PHelp.as_deref().unwrap_or("");
    let p_value = body.PValue.as_deref().unwrap_or("{}");
    // ★ EUser 是 uniqueidentifier 列，必须传 UUID 字符串（claims.emp_id 是 UUID）
    let e_user: &str = if !claims.emp_id.is_empty() { claims.emp_id.as_str() } else { return Ok(Json(ApiResponse::err("登录用户缺少 EmpID，无法写入审计字段"))) };

    if p_name.is_empty() {
        return Ok(Json(ApiResponse::err("模板名称不能为空")));
    }

    let sql = "UPDATE tSys_Parameters SET PCode = @p1, PName = @p2, PKind = @p3, PTerm = @p4, PHelp = @p5, PValue = @p6, EUser = @p7, EDate = @p8 WHERE ParametersID = @p9";

    conn.execute(sql, &[
        &p_code,
        &p_name,
        &p_kind,
        &p_term,
        &p_help,
        &p_value,
        &e_user,
        &now,
        &body.ParametersID.as_str(),
    ]).await?;

    Ok(Json(ApiResponse::msg("定价模板更新成功")))
}

#[derive(Deserialize)]
pub struct DeletePricingTemplateParams {
    pub ids: Vec<String>,
}

pub async fn delete_pricing_template(
    State(_config): State<Config>,
    Json(body): Json<DeletePricingTemplateParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    if body.ids.is_empty() {
        return Ok(Json(ApiResponse::err("请选择要删除的模板")));
    }

    for id in &body.ids {
        let sql = "DELETE FROM tSys_Parameters WHERE ParametersID = @p1";
        conn.execute(sql, &[&id.as_str()]).await?;
    }

    Ok(Json(ApiResponse::msg(&format!("成功删除{}个模板", body.ids.len()))))
}

#[derive(Deserialize)]
pub struct GetPricingRulesParams {
    pub template_id: Option<String>,
}

pub async fn get_pricing_rules(
    State(_config): State<Config>,
    Json(_params): Json<GetPricingRulesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    Ok(Json(ApiResponse::ok(vec![])))
}

#[derive(Deserialize)]
pub struct GetCustomerPricesParams {
    pub cust_id: Option<String>,
    pub gds_id: Option<String>,
}

pub async fn get_customer_prices(
    State(_config): State<Config>,
    Json(params): Json<GetCustomerPricesParams>,
) -> Result<Json<ApiResponse<Vec<serde_json::Value>>>> {
    let mut conn = get_pool().get().await?;

    let mut sql = r#"SELECT cp.CustID, cp.BrandID, cp.PLID,
        c.CustName, b.BrandName, p.PName AS PLName, p.PValue AS PLRate
        FROM tBas_CustPriceTac cp
        LEFT JOIN tBas_Cust c ON cp.CustID = c.CustID
        LEFT JOIN tBas_Brand b ON cp.BrandID = b.BrandID
        LEFT JOIN tSys_Parameters p ON cp.PLID = p.ParametersID
        WHERE 1=1"#.to_string();
    let mut query_params: Vec<Option<String>> = Vec::new();
    let mut pidx = 1;

    if let Some(cid) = &params.cust_id {
        if !cid.is_empty() {
            sql.push_str(&format!(" AND cp.CustID = @p{}", pidx));
            pidx += 1;
            query_params.push(Some(cid.clone()));
        }
    }

    if let Some(gid) = &params.gds_id {
        if !gid.is_empty() {
            sql.push_str(&format!(" AND cp.BrandID = @p{}", pidx));
            query_params.push(Some(gid.clone()));
        }
    }

    let param_refs: Vec<&dyn tiberius::ToSql> = query_params.iter().map(|v| v as &dyn tiberius::ToSql).collect();
    let stream = conn.query(&sql, &param_refs).await?;
    let rows: Vec<Row> = stream.into_first_result().await?;
    let data: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();

    Ok(Json(ApiResponse::ok(data)))
}

#[derive(Deserialize)]
pub struct SaveCustomerPriceParams {
    pub CustID: String,
    pub BrandID: String,
    pub PLID: Option<String>,
}

pub async fn save_customer_price(
    Extension(_claims): Extension<Claims>,
    State(_config): State<Config>,
    Json(body): Json<SaveCustomerPriceParams>,
) -> Result<Json<ApiResponse<serde_json::Value>>> {
    let mut conn = get_pool().get().await?;

    let check_sql = "SELECT COUNT(*) as cnt FROM tBas_CustPriceTac WHERE CustID = @p1 AND BrandID = @p2";
    let stream = conn.query(check_sql, &[&body.CustID.as_str(), &body.BrandID.as_str()]).await?;
    let mut exists = false;
    if let Some(row) = stream.into_row().await? {
        let cnt: i32 = row.get::<i32, _>("cnt").unwrap_or(0);
        exists = cnt > 0;
    }

    let plid = body.PLID.as_deref().unwrap_or("");

    if exists {
        let sql = "UPDATE tBas_CustPriceTac SET PLID = @p1 WHERE CustID = @p2 AND BrandID = @p3";
        conn.execute(sql, &[
            &plid,
            &body.CustID.as_str(),
            &body.BrandID.as_str(),
        ]).await?;
    } else {
        let sql = "INSERT INTO tBas_CustPriceTac (CustID, BrandID, PLID) VALUES (@p1, @p2, @p3)";
        conn.execute(sql, &[
            &body.CustID.as_str(),
            &body.BrandID.as_str(),
            &plid,
        ]).await?;
    }

    Ok(Json(ApiResponse::msg("客户定价保存成功")))
}
