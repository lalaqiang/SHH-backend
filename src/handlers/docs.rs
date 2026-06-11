//! OpenAPI 文档端点
//!
//! 提供：
//!  - GET /api-docs/openapi.yaml    返回 YAML 描述
//!  - GET /api-docs/openapi.json    YAML → JSON 自动转换
//!  - GET /api-docs                 重定向到 Swagger UI 在线版（cdn）
//!  - GET /api-docs/redoc           重定向到 Redoc 在线版
//!
//! 这样做的好处：
//!  - 后端无静态文件托管负担
//!  - swagger-ui / redoc CDN 自带最新版本
//!  - 零额外依赖（不引入 utoipa-axum）

use axum::{
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
};

const OPENAPI_YAML: &str = include_str!("../../openapi/openapi.yaml");

/// GET /api-docs/openapi.yaml
pub async fn openapi_yaml() -> Response {
    let mut resp = OPENAPI_YAML.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/yaml; charset=utf-8"),
    );
    resp
}

/// GET /api-docs/openapi.json
pub async fn openapi_json() -> Response {
    // YAML → JSON
    let yaml: serde_yaml::Value = match serde_yaml::from_str(OPENAPI_YAML) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("OpenAPI YAML 解析失败: {}", e),
            )
                .into_response();
        }
    };
    let json = match serde_json::to_string_pretty(&yaml) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("OpenAPI YAML → JSON 失败: {}", e),
            )
                .into_response();
        }
    };
    let mut resp = json.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    resp
}

/// GET /api-docs
/// 302 跳转到 Swagger UI 在线版（用本服务 openapi.json）
pub async fn swagger_ui() -> Redirect {
    Redirect::to("/api-docs/swagger-ui")
}

/// GET /api-docs/swagger-ui
/// 直接渲染一个 HTML 页面，加载 CDN 版 swagger-ui
pub async fn swagger_ui_html() -> Response {
    let html = include_str!("../../openapi/swagger-ui.html");
    let mut resp = html.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // 禁用缓存（升级时用户拿到最新版本）
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    resp
}

/// GET /api-docs/redoc
pub async fn redoc_html() -> Response {
    let html = include_str!("../../openapi/redoc.html");
    let mut resp = html.into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    resp
}
