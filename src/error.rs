use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Db(#[from] tiberius::error::Error),
    #[error("数据库连接池错误: {0}")]
    Pool(#[from] bb8::RunError<bb8_tiberius::Error>),
    #[error("JWT错误: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("未授权")]
    Unauthorized,
    #[error("{0}")]
    BadRequest(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("DB: {}", e)),
            AppError::Pool(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Pool: {}", e)),
            AppError::Jwt(e) => (StatusCode::UNAUTHORIZED, format!("JWT: {}", e)),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "未登录或Token已过期".into()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        let body = serde_json::json!({
            "success": false,
            "message": message
        });
        (status, axum::Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
