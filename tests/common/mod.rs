//! 集成测试公共 helper
//!
//! 提供：plain_router、send_get、send_get_with_ip、send_post_json、db_tests_enabled

#![allow(dead_code)]

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::Response,
    routing::get,
};
use tower::ServiceExt;

/// 测试用 dummy handler
pub async fn dummy_ok() -> &'static str {
    "ok"
}

/// 不带任何中间件的裸 router
pub fn plain_router() -> Router {
    Router::new().route("/test", get(dummy_ok))
}

/// 发送 GET 请求并返回 (status, body)
pub async fn send_get(router: &Router, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request builder");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024)
        .await
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    (status, body)
}

/// 发送带 X-Forwarded-For 的 GET 请求（用于模拟不同 IP）
pub async fn send_get_with_ip(router: &Router, uri: &str, ip: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("X-Forwarded-For", ip)
        .body(Body::empty())
        .expect("request builder");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024)
        .await
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    (status, body)
}

/// 发送 POST 请求（带 JSON body）
pub async fn send_post_json(router: &Router, uri: &str, body: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request builder");
    let resp = router.clone().oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 4096)
        .await
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_default();
    (status, body)
}

/// 是否需要运行 DB 集成测试（设置 TEST_DB_HOST 即视为开启）
pub fn db_tests_enabled() -> bool {
    std::env::var("TEST_DB_HOST").is_ok()
}

#[allow(dead_code)]
fn _unused(_: &Response) {}
