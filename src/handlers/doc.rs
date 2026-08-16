//! 统一单据服务 HTTP handler
//!
//! 暴露给前端的统一入口：
//!   - POST /api/doc/save                保存（新增/更新）
//!   - POST /api/doc/approve             审核
//!   - POST /api/doc/unapprove           反审
//!   - POST /api/doc/void                作废
//!   - POST /api/doc/generate-from-source 参照生单
//!   - POST /api/doc/graph               取元数据（含版本号）

use axum::{
    Extension, Json,
    extract::{ConnectInfo, State},
};
use serde_json::{Value, json};
use std::net::SocketAddr;

use crate::config::Config;
use crate::db::get_pool;
use crate::error::Result;
use crate::metadata::doc_graph;
use crate::middleware::auth::Claims;
use crate::services::doc_service::{
    self, ApproveDocParams, GenerateFromSourceParams, GenerateFromSourceResponse, SaveDocParams,
    VoidDocParams,
};
use crate::utils::ApiResponse;

/// POST /api/doc/save
pub async fn doc_save(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<SaveDocParams>,
) -> Result<Json<ApiResponse<Value>>> {
    let mut conn = get_pool().get().await?;
    match doc_service::save_doc(&mut conn, &claims.user_code, &claims.user_name, params).await {
        Ok(resp) => Ok(Json(ApiResponse::ok(json!(resp)))),
        Err(err) => match err {
            doc_service::ApproveError::Shortage(items) => Ok(Json(ApiResponse::err_with_data(
                "库存不足，无法保存",
                "STOCK_INSUFFICIENT",
                json!({ "shortage_list": items }),
            ))),
            doc_service::ApproveError::Msg(msg) => Ok(Json(ApiResponse::err(&msg))),
        },
    }
}

/// POST /api/doc/approve
pub async fn doc_approve(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<ApproveDocParams>,
) -> Result<Json<ApiResponse<Value>>> {
    let mut conn = get_pool().get().await?;
    match doc_service::approve_doc(&mut conn, &claims.user_code, &claims.user_name, params).await {
        Ok(msg) => Ok(Json(ApiResponse::msg(&msg))),
        Err(err) => match err {
            doc_service::ApproveError::Shortage(items) => Ok(Json(ApiResponse::err_with_data(
                "库存不足，无法审核",
                "STOCK_INSUFFICIENT",
                json!({ "shortage_list": items }),
            ))),
            doc_service::ApproveError::Msg(msg) => Ok(Json(ApiResponse::err(&msg))),
        },
    }
}

/// POST /api/doc/unapprove
pub async fn doc_unapprove(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<ApproveDocParams>,
) -> Result<Json<ApiResponse<String>>> {
    let mut conn = get_pool().get().await?;
    match doc_service::unapprove_doc(&mut conn, &claims.user_code, &claims.user_name, params).await
    {
        Ok(msg) => Ok(Json(ApiResponse::msg(&msg))),
        Err(msg) => Ok(Json(ApiResponse::err(&msg))),
    }
}

/// POST /api/doc/void
pub async fn doc_void(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<VoidDocParams>,
) -> Result<Json<ApiResponse<String>>> {
    let mut conn = get_pool().get().await?;
    match doc_service::void_doc(&mut conn, &claims.user_code, &claims.user_name, params).await {
        Ok(msg) => Ok(Json(ApiResponse::msg(&msg))),
        Err(msg) => Ok(Json(ApiResponse::err(&msg))),
    }
}

/// POST /api/doc/generate-from-source
pub async fn doc_generate_from_source(
    State(_config): State<Config>,
    Extension(claims): Extension<Claims>,
    Json(params): Json<GenerateFromSourceParams>,
) -> Result<Json<ApiResponse<GenerateFromSourceResponse>>> {
    let mut conn = get_pool().get().await?;
    match doc_service::generate_from_source(&mut conn, &claims.user_code, params).await {
        Ok(resp) => Ok(Json(ApiResponse::ok(resp))),
        Err(msg) => Ok(Json(ApiResponse::err(&msg))),
    }
}

/// POST /api/doc/graph
/// 返回 doc_graph 元数据（含版本号），前端用于与本地 docGraph.js 对账
///
/// P1-10 修复：用 ConnectInfo<SocketAddr> 获取客户端真实 IP（原 empty_ip() 永远返回空字符串）
pub async fn doc_graph(
    State(_config): State<Config>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<ApiResponse<Value>>> {
    let resp = doc_graph::build_graph_response();
    let all_meta = doc_graph::all_docs();
    Ok(Json(ApiResponse::ok(json!({
        "version": resp.version,
        "docs": resp.docs,
        "edges": resp.edges,
        "kind_map": resp.kind_map,
        "all_meta": all_meta,
        "ip": addr.ip().to_string(),
    }))))
}
