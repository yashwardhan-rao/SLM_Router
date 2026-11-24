//! The axum application: bearer auth, `/v1/models`, `/v1/chat/completions`.

mod chat;
mod guard;
mod policy;
mod sse;
mod stream;
mod telemetry;

use crate::{config::Config, errors::ApiError, logging::Logger, session::PinStore};
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

pub struct AppState {
    pub cfg: Config,
    pub http: reqwest::Client,
    pub pins: PinStore,
    pub logger: Logger,
    pub seq: AtomicU64,
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
        http,
        cfg,
    });
    Router::new()
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat::chat))
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
        // shortcut: plain string compare, not constant-time; fine for a loopback-first proxy.
        if sent.and_then(|v| v.strip_prefix("Bearer ")) != Some(key.as_str()) {
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
