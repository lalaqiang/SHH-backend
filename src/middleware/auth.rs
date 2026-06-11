use axum::{
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    extract::Request,
    Json,
};
use jsonwebtoken::{decode, DecodingKey, Validation, Algorithm};
use serde::{Deserialize, Serialize};
use crate::config::Config;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub user_code: String,
    pub user_name: String,
    pub exp: usize,
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
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "success": false,
                        "message": "无效的Authorization头格式"
                    })),
                )
                    .into_response();
            }
        }
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "success": false,
                    "message": "缺少Authorization头"
                })),
            )
                .into_response();
        }
    };

    let token_data = match decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_ref()),
        &Validation::new(Algorithm::HS256),
    ) {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("JWT验证失败: {}", e);
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "success": false,
                    "message": "Token无效或已过期"
                })),
            )
                .into_response();
        }
    };

    let claims = token_data.claims;

    req.extensions_mut().insert(claims);

    next.run(req).await
}
