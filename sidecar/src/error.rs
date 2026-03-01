//! Error types for the erenshor-llm sidecar.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Application errors with HTTP response mapping.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Service unavailable: {0}")]
    Unavailable(String),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

/// JSON error response body matching OpenAI error format.
#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    code: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_type, code) = match &self {
            AppError::BadRequest(_) => (
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "bad_request",
            ),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found_error", "not_found"),
            AppError::Internal(_) | AppError::Anyhow(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal_error",
            ),
            AppError::Unavailable(_) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "service_unavailable",
            ),
        };

        let body = ErrorResponse {
            error: ErrorDetail {
                message: self.to_string(),
                error_type: error_type.to_string(),
                code: code.to_string(),
            },
        };

        (status, axum::Json(body)).into_response()
    }
}

/// Result type alias for route handlers.
pub type AppResult<T> = Result<T, AppError>;
