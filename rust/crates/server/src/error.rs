//! One error type for handlers: `Result<Response, AppError>` plus `?` replaces the
//! hand-mapped `match ... Err(e) => server_error(..)` arms.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub(crate) struct AppError(Response);

pub(crate) type Handled = Result<Response, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        self.0
    }
}

impl From<Response> for AppError {
    fn from(r: Response) -> Self {
        AppError(r)
    }
}

impl AppError {
    /// `{"error": msg}`
    pub(crate) fn new(status: StatusCode, msg: impl Into<String>) -> Self {
        AppError((status, Json(json!({ "error": msg.into() }))).into_response())
    }

    /// `{"detail": msg}` — the shape the boat-assignment and student-level routes
    /// document in CLAUDE.md.
    pub(crate) fn detail(status: StatusCode, msg: impl Into<String>) -> Self {
        AppError((status, Json(json!({ "detail": msg.into() }))).into_response())
    }

    pub(crate) fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }

    pub(crate) fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, msg)
    }

    pub(crate) fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }

    pub(crate) fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }

    /// Generic 500: log the real cause, return an opaque body so DB messages and
    /// paths don't leak.
    pub(crate) fn internal(context: &str, e: impl std::fmt::Debug) -> Self {
        eprintln!("[error] {context}: {e:?}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
}
