//! Request-level policy and routing / upstream error mapping.

use crate::{
    errors::ApiError,
    routing::{Policy, RouteError},
    upstream::{Api, Reply},
};
use axum::http::{HeaderValue, StatusCode};
use serde_json::Value;

/// A 404/405 without an OpenAI error body means the upstream lacks `/v1/responses`. One *with*
/// an error body (e.g. an unknown `previous_response_id`) is a real answer and is relayed as is.
pub fn unsupported(api: Api, tier: &str, r: &Reply) -> Option<ApiError> {
    let error_body =
        serde_json::from_slice::<Value>(&r.body).is_ok_and(|v| v.get("error").is_some());
    if api != Api::Responses || !matches!(r.status, 404 | 405) || error_body {
        return None;
    }
    let msg = format!("{tier} upstream does not implement /v1/responses");
    Some(
        ApiError::new(StatusCode::NOT_IMPLEMENTED, "invalid_request_error", msg)
            .code("responses_unsupported"),
    )
}

/// The header can only tighten the configured default, never loosen it.
pub fn effective_policy(cfg: Policy, header: Option<&HeaderValue>) -> Result<Policy, ApiError> {
    let Some(h) = header else { return Ok(cfg) };
    let want = match h.to_str().ok() {
        Some("auto") => Policy::Auto,
        Some("local-only") => Policy::LocalOnly,
        Some("cloud-only") => Policy::CloudOnly,
        _ => {
            return Err(ApiError::bad_request(
                "X-Router-Policy must be auto, local-only or cloud-only",
            ));
        }
    };
    Ok(if cfg == Policy::Auto { want } else { cfg })
}

pub fn route_error(e: RouteError) -> ApiError {
    match e {
        RouteError::PolicyConflict => {
            ApiError::bad_request("requested model conflicts with the routing policy")
                .code("policy_conflict")
        }
        RouteError::ContextExceeded => {
            ApiError::bad_request("prompt exceeds the context of every tier it may use")
                .code("context_length_exceeded")
        }
        RouteError::NeedsCloud => ApiError::bad_request(
            "previous_response_id, conversation, store and background need a stateful tier",
        )
        .code("stateful_requires_cloud"),
        RouteError::NoSuchTier => {
            ApiError::bad_request("no tier is configured for the requested model")
                .code("model_not_found")
        }
    }
}
