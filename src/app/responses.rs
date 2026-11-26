//! `POST /v1/responses`, streaming and non-streaming. The body is forwarded as received
//! (bar `model`); a chat-shaped view of it drives routing and session fingerprinting.

use super::{
    AppState,
    dispatch::{Job, serve},
};
use crate::{
    errors::ApiError,
    schema::{features, responses_view, stateful},
    session::session_id,
    upstream::Api,
};
use axum::{
    body::Bytes,
    extract::{State, rejection::BytesRejection},
    http::HeaderMap,
    response::Response,
};
use serde_json::Value;
use std::sync::Arc;

pub async fn responses(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    // Over-limit bodies get an OpenAI-style 413 instead of axum's plain-text one.
    let body =
        body.map_err(|e| ApiError::new(e.status(), "invalid_request_error", e.body_text()))?;
    let body: Value = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))?;
    let view = responses_view(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))?;
    let mut feats = features(&view);
    feats.requires_cloud = stateful(&body);
    let hdr = headers.get("x-session-id").and_then(|v| v.to_str().ok());
    let (sid, source) = session_id(hdr, &view);
    let job = Job {
        api: Api::Responses,
        body,
        feats,
        sid,
        source,
        hide_usage: false,
    };
    serve(s, &headers, job).await
}
