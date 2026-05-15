use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Redis error: {0}")]
    Redis(String),

    #[error("Rate limited")]
    RateLimited { retry_after: u64 },
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Log internal errors server-side before sanitizing for the client
        match &self {
            AppError::Database(msg) => tracing::error!(error = %msg, "Database error"),
            AppError::Redis(msg) => tracing::error!(error = %msg, "Redis error"),
            AppError::Internal(msg) => tracing::error!(error = %msg, "Internal error"),
            _ => {}
        }

        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into()),
            AppError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into()),
            AppError::Redis(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error".into()),
            AppError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "Rate limited".into()),
        };

        let body = serde_json::json!({ "error": message });
        let mut response = (status, Json(body)).into_response();

        if let AppError::RateLimited { retry_after } = &self {
            let mut headers = HeaderMap::new();
            if let Ok(val) = HeaderValue::from_str(&retry_after.to_string()) {
                headers.insert("Retry-After", val);
            }
            response.headers_mut().extend(headers);
        }

        response
    }
}

impl From<surrealdb::Error> for AppError {
    fn from(err: surrealdb::Error) -> Self {
        AppError::Database(err.to_string())
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(err: jsonwebtoken::errors::Error) -> Self {
        tracing::debug!(error = %err, "JWT validation failed");
        AppError::Unauthorized("Invalid token".into())
    }
}

impl From<deadpool_redis::PoolError> for AppError {
    fn from(err: deadpool_redis::PoolError) -> Self {
        AppError::Redis(err.to_string())
    }
}

impl From<redis::RedisError> for AppError {
    fn from(err: redis::RedisError) -> Self {
        AppError::Redis(err.to_string())
    }
}
