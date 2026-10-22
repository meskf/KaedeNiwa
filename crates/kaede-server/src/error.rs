//! HTTP API 错误。

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    #[error("internal error: {0}")]
    Internal(String),
    /// 端点曾经存在但已废弃（410 Gone）。用于 e2ee-key 等已下线端点。
    #[error("gone: {0}")]
    Gone(String),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::Internal(format!("db: {}", e))
    }
}

impl From<argon2::password_hash::Error> for ApiError {
    fn from(e: argon2::password_hash::Error) -> Self {
        ApiError::Internal(format!("argon2: {}", e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, "bad_request", m.clone()),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "unauthorized".to_string(),
            ),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, "not_found", m.clone()),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, "conflict", m.clone()),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, "forbidden", m.clone()),
            ApiError::TooManyRequests(m) => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                m.clone(),
            ),
            ApiError::Internal(m) => {
                tracing::error!("internal: {}", m);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error".to_string(),
                )
            }
            ApiError::Gone(m) => (StatusCode::GONE, "gone", m.clone()),
        };
        (status, Json(json!({ "error": code, "message": message }))).into_response()
    }
}
