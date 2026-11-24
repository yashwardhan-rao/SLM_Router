//! OpenAI-style error bodies for every router-originated failure.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub kind: &'static str,
    pub code: Option<&'static str>,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            kind,
            code: None,
            message: message.into(),
        }
    }

    pub fn code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", message)
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "invalid or missing bearer token",
        )
    }

    pub fn upstream(status: StatusCode, message: impl Into<String>) -> Self {
        Self::new(status, "upstream_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body =
            json!({"error": {"message": self.message, "type": self.kind, "code": self.code}});
        (self.status, Json(body)).into_response()
    }
}
