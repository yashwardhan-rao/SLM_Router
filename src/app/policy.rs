//! Request-level policy and routing-error mapping.

use crate::{
    errors::ApiError,
    routing::{Policy, RouteError},
};
use axum::http::HeaderValue;

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
            ApiError::bad_request("prompt exceeds the local model's context")
                .code("context_length_exceeded")
        }
    }
}
