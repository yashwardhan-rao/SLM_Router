//! `POST /v1/chat/completions`, streaming and non-streaming.

use super::{
    AppState,
    dispatch::{Job, serve},
};
use crate::{
    errors::ApiError,
    schema::{ChatRequest, features},
    session::session_id,
    upstream::Api,
};
use axum::{
    body::Bytes,
    extract::{State, rejection::BytesRejection},
    http::HeaderMap,
    response::Response,
};
use std::sync::Arc;

pub async fn chat(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, ApiError> {
    // Over-limit bodies get an OpenAI-style 413 instead of axum's plain-text one.
    let body =
        body.map_err(|e| ApiError::new(e.status(), "invalid_request_error", e.body_text()))?;
    let req: ChatRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))?;
    let feats = features(&req);
    let hdr = headers.get("x-session-id").and_then(|v| v.to_str().ok());
    let (sid, source) = session_id(hdr, &req);
    // Typed parse validates the shape; re-serialising keeps unknown fields via `extra`.
    let body = serde_json::to_value(&req).expect("a parsed ChatRequest serialises");
    let job = Job {
        api: Api::Chat,
        body,
        feats,
        sid,
        source,
    };
    serve(s, &headers, job).await
}
