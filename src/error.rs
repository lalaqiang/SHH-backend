use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// 统一应用错误类型。
///
/// 对外响应策略（Sprint6-35）：
/// - `Db` / `Pool` / `Jwt` / `Internal`：底层错误细节只写服务端日志，对外返回通用中文消息
///   （避免向客户端泄露数据库表名、SQL 片段、驱动版本、JWT 实现细节等信息）
/// - `BadRequest`：400 + 业务消息（调用方传入的 msg 视为可对用户展示）
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("数据库错误: {0}")]
    Db(#[from] tiberius::error::Error),
    #[error("数据库连接池错误: {0}")]
    Pool(#[from] bb8::RunError<bb8_tiberius::Error>),
    #[error("JWT错误: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("{0}")]
    BadRequest(String),
    #[error("内部错误: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message, code) = match self {
            // 基础设施错误：日志保留完整信息，对外只返回通用消息
            AppError::Db(e) => {
                tracing::error!(error = ?e, "DB error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "数据库错误，请稍后重试".to_string(),
                    "SYS_DB_ERROR",
                )
            }
            AppError::Pool(e) => {
                tracing::error!(error = ?e, "DB pool error");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "服务繁忙，请稍后重试".to_string(),
                    "SYS_DB_UNAVAILABLE",
                )
            }
            AppError::Jwt(e) => {
                tracing::warn!(error = ?e, "JWT error");
                (
                    StatusCode::UNAUTHORIZED,
                    "登录已过期，请重新登录".to_string(),
                    "AUTH_TOKEN_EXPIRED",
                )
            }
            // 业务错误：消息可对用户展示
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg, "BAD_REQUEST"),
            AppError::Internal(msg) => {
                // 内部错误消息可能包含敏感信息，只写日志
                tracing::error!(error = %msg, "Internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "服务器内部错误，请稍后重试".to_string(),
                    "SYS_INTERNAL_ERROR",
                )
            }
        };
        let body = serde_json::json!({
            "success": false,
            "message": message,
            "code": code
        });
        (status, axum::Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
