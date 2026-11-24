//! The axum application: bearer auth, `/v1/models`, `/v1/chat/completions`, `/v1/responses`.

mod chat;
mod dispatch;
mod guard;
mod policy;
mod responses;
mod sse;
mod stream;
mod target;
mod telemetry;

use crate::{
    circuit::Breaker, config::Config, errors::ApiError, logging::Logger, session::PinStore,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::header::AUTHORIZATION,
    middleware::{Next, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::AtomicU64};
use std::time::Duration;
use subtle::ConstantTimeEq;

pub struct AppState {
    pub cfg: Config,
    pub http: reqwest::Client,
    pub pins: PinStore,
    pub logger: Logger,
    pub seq: AtomicU64,
    pub breaker: Breaker,
}

pub fn router(cfg: Config) -> Router {
    let body_limit = cfg.server.max_body_bytes;
    let http = reqwest::Client::builder()
        // A provider that stalls (no bytes for this long) ends the request instead of hanging it.
        .read_timeout(Duration::from_secs(120))
        .build()
        .expect("reqwest client builds with its default TLS backend");
    let state = Arc::new(AppState {
        pins: PinStore::new(
            Duration::from_secs(cfg.session.ttl_secs),
            cfg.session.max_pins,
        ),
        logger: Logger::new(&cfg.log.path),
        seq: AtomicU64::new(0),
        breaker: Breaker::new(
            cfg.local.breaker_failures,
            Duration::from_millis(cfg.local.breaker_cooldown_ms),
        ),
        http,
        cfg,
    });
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat::chat))
        .route("/v1/responses", post(responses::responses))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(from_fn_with_state(state.clone(), auth))
        .with_state(state)
}

async fn auth(State(s): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    if let Some(key) = &s.cfg.server.api_key {
        let sent = req
            .headers()
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        let token = sent.and_then(|v| v.strip_prefix("Bearer "));
        // `ct_eq` is constant-time in the bytes; only a length mismatch returns early.
        if !token.is_some_and(|t| bool::from(t.as_bytes().ct_eq(key.as_bytes()))) {
            return ApiError::unauthorized().into_response();
        }
    }
    next.run(req).await
}

async fn models() -> Json<Value> {
    let data: Vec<Value> = ["auto", "local", "cloud"]
        .iter()
        .map(|m| json!({"id": format!("slm-router/{m}"), "object": "model", "owned_by": "slm-router"}))
        .collect();
    Json(json!({"object": "list", "data": data}))
}
