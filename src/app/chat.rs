//! `POST /v1/chat/completions` (non-streaming).

use super::{
    AppState,
    telemetry::{new_record, saved_usd, usage},
};
use crate::{
    errors::ApiError,
    routing::{Policy, Reason, RouteError, Target, decide},
    schema::{ChatRequest, features},
    session::session_id,
    upstream::{Reply, Unreachable, post_json},
};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

/// The header can only tighten the configured default, never loosen it.
fn effective_policy(cfg: Policy, header: Option<&HeaderValue>) -> Result<Policy, ApiError> {
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

fn route_error(e: RouteError) -> ApiError {
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

fn relay(r: Reply) -> Response {
    let status = StatusCode::from_u16(r.status).unwrap_or(StatusCode::BAD_GATEWAY);
    (status, [(CONTENT_TYPE, "application/json")], r.body).into_response()
}

pub async fn chat(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let mut req: ChatRequest = serde_json::from_slice(&body)
        .map_err(|e| ApiError::bad_request(format!("invalid request body: {e}")))?;
    if req.stream == Some(true) {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "invalid_request_error",
            "streaming is not supported yet",
        )
        .code("stream_not_supported"));
    }
    let policy = effective_policy(s.cfg.server.policy, headers.get("x-router-policy"))?;
    let feats = features(&req);
    let started = Instant::now();
    let hdr = headers.get("x-session-id").and_then(|v| v.to_str().ok());
    let (sid, source) = session_id(hdr, &req);
    let decision = decide(&feats, &s.cfg.routing, policy, s.pins.get(&sid)).map_err(route_error)?;

    let seq = s.seq.fetch_add(1, Ordering::Relaxed);
    let mut rec = new_record(seq, sid.clone(), source, policy);
    rec.target = decision.target;
    rec.reason = decision.reason;
    rec.pinned = decision.reason == Reason::Pinned;

    let mut result = call(&s, &mut req, decision.target).await;
    if decision.target == Target::Local && local_failed(&result) {
        // Failover only for auto-routed requests; forced-local and local-only must not leak to cloud.
        if decision.reason == Reason::Forced || policy == Policy::LocalOnly {
            rec.status = 503;
            rec.outcome = "upstream_error";
            rec.total_ms = started.elapsed().as_millis() as u64;
            s.logger.write(&rec);
            return Err(ApiError::upstream(
                StatusCode::SERVICE_UNAVAILABLE,
                "local model unavailable and cloud fallback is not allowed",
            )
            .code("local_unavailable"));
        }
        (rec.target, rec.reason, rec.failed_over, rec.pinned) =
            (Target::Cloud, Reason::LocalUnavailable, true, false);
        result = call(&s, &mut req, Target::Cloud).await;
    }
    rec.total_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok(reply) => {
            let ok = reply.status < 400;
            let (p, c, est) = match ok {
                true => usage(&reply.body, feats.est_prompt_tokens),
                false => (0, 0, false),
            };
            rec.status = reply.status;
            rec.ttfb_ms = reply.ttfb_ms;
            rec.outcome = if ok { "ok" } else { "upstream_error" };
            (
                rec.prompt_tokens,
                rec.completion_tokens,
                rec.tokens_estimated,
            ) = (p, c, est);
            rec.saved_usd = saved_usd(rec.target, s.cfg.pricing, p, c);
            if !matches!(decision.reason, Reason::Forced | Reason::Policy) {
                s.pins.set(&sid, rec.target);
            }
            s.logger.write(&rec);
            Ok(relay(reply))
        }
        Err(e) => {
            rec.status = 502;
            rec.outcome = "upstream_error";
            s.logger.write(&rec);
            Err(ApiError::upstream(
                StatusCode::BAD_GATEWAY,
                format!("upstream unreachable: {}", e.0),
            ))
        }
    }
}

fn local_failed(r: &Result<Reply, Unreachable>) -> bool {
    r.as_ref().map_or(true, |reply| reply.status >= 500)
}

async fn call(s: &AppState, req: &mut ChatRequest, target: Target) -> Result<Reply, Unreachable> {
    match target {
        Target::Local => {
            req.model = s.cfg.local.model.clone();
            let timeout = Duration::from_millis(s.cfg.local.first_byte_timeout_ms);
            post_json(&s.http, &s.cfg.local.base_url, None, req, Some(timeout)).await
        }
        Target::Cloud => {
            req.model = s.cfg.cloud.model.clone();
            let key = s.cfg.cloud.api_key.as_deref();
            post_json(&s.http, &s.cfg.cloud.base_url, key, req, None).await
        }
    }
}
