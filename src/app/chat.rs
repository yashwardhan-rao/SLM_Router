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
use serde_json::Value;
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
    let mut body = serde_json::to_value(&req).expect("a parsed ChatRequest serialises");
    let hide_usage = request_usage(&mut body)?;
    let job = Job {
        api: Api::Chat,
        body,
        feats,
        sid,
        source,
        hide_usage,
    };
    serve(s, &headers, job).await
}

/// Ask a streaming upstream for its usage chunk, so token counts are exact rather than estimated.
/// Returns true when the client did not ask for that chunk itself and it must be hidden from it.
// shortcut: always on; add a config switch if a cloud ever rejects `stream_options`.
fn request_usage(body: &mut Value) -> Result<bool, ApiError> {
    let opts = &body["stream_options"];
    if !(opts.is_null() || opts.is_object()) {
        return Err(ApiError::bad_request("stream_options must be an object"));
    }
    if body["stream"] != true || opts["include_usage"] == true {
        return Ok(false);
    }
    body["stream_options"]["include_usage"] = Value::Bool(true);
    Ok(true)
}
