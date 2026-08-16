use crate::config::Config;
use axum::{
    Json,
    extract::Request,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
// Claims 定义已迁移至 `utils::jwt`，此处重导出以保持向后兼容
// （15 个 handler 文件通过 `use crate::middleware::auth::Claims` 引用，无需修改）
pub use crate::utils::jwt::{Claims, verify_token};

/// 统一构造 401 未授权响应。
///
/// 所有 401 场景对外返回相同的用户可读消息 "登录已过期，请重新登录"，
/// 通过 `code` 字段区分具体原因（AUTH_TOKEN_MISSING / AUTH_TOKEN_INVALID / AUTH_TOKEN_EXPIRED），
/// 供前端精确处理（如决定是否跳转登录页）。
fn unauthorized_response(code: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "success": false,
            "message": "登录已过期，请重新登录",
            "code": code
        })),
    )
        .into_response()
}

pub async fn auth_middleware(
    State(config): State<Config>,
    mut req: Request,
    next: Next,
) -> Response {
    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = match auth_header {
        Some(header) => {
            if header.starts_with("Bearer ") {
                &header[7..]
            } else {
                return unauthorized_response("AUTH_TOKEN_INVALID");
            }
        }
        None => {
            return unauthorized_response("AUTH_TOKEN_MISSING");
        }
    };

    let claims = match verify_token(&config.jwt_secret, token) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "JWT验证失败");
            return unauthorized_response("AUTH_TOKEN_EXPIRED");
        }
    };

    // P2-17 修复：检查 token 是否已被吊销（登出黑名单）
    //   旧版 logout 仅记录审计日志，token 仍可使用 24h，存在安全隐患
    //   现在 logout 会调用 revoke_token 将 jti 加入黑名单，此处检查
    if crate::utils::jwt::is_token_revoked(&claims) {
        tracing::warn!(
            user_code = %claims.user_code,
            jti = %claims.jti,
            "Token 已被吊销（用户已登出）"
        );
        return unauthorized_response("AUTH_TOKEN_EXPIRED");
    }

    req.extensions_mut().insert(claims);

    next.run(req).await
}
