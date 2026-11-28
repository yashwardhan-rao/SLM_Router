//! Typed TOML config, loaded once at startup. Secrets come from env vars, never the file.

use crate::routing::{Policy, RoutingConfig};
use serde::Deserialize;
use std::net::SocketAddr;

mod tier;
pub use tier::TierCfg;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerCfg,
    /// Legacy two-tier form, with `cloud`; desugared by `ladder()`. Exclusive with `tier`.
    pub local: Option<LocalCfg>,
    pub cloud: Option<CloudCfg>,
    /// The ladder, cheapest first.
    #[serde(default)]
    pub tier: Vec<TierCfg>,
    #[serde(default)]
    pub routing: RoutingConfig,
    pub pricing: Option<Pricing>,
    #[serde(default)]
    pub session: SessionCfg,
    #[serde(default)]
    pub log: LogCfg,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerCfg {
    pub bind: String,
    pub policy: Policy,
    /// Largest accepted request body. Base64 images make bodies big, so the default is generous.
    pub max_body_bytes: usize,
    /// Add `stream_options.include_usage` to streamed chat requests so token counts are exact.
    /// Turn off for an upstream that rejects `stream_options`.
    pub request_stream_usage: bool,
    /// From `ROUTER_API_KEY`; clients must send it as a bearer token when set.
    #[serde(skip)]
    pub api_key: Option<String>,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".into(),
            policy: Policy::Auto,
            max_body_bytes: 64 * 1024 * 1024,
            request_stream_usage: true,
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalCfg {
    pub base_url: String,
    pub model: String,
    #[serde(default = "default_first_byte_ms")]
    pub first_byte_timeout_ms: u64,
    /// Consecutive failures (unreachable or 5xx) that open the circuit; 0 disables it.
    #[serde(default = "default_breaker_failures")]
    pub breaker_failures: u32,
    /// How long an open circuit skips local before letting one probe through.
    #[serde(default = "default_breaker_cooldown_ms")]
    pub breaker_cooldown_ms: u64,
}

fn default_first_byte_ms() -> u64 {
    15_000
}

pub(crate) fn default_breaker_failures() -> u32 {
    2
}

pub(crate) fn default_breaker_cooldown_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudCfg {
    pub base_url: String,
    pub model: String,
    /// Name of the env var holding the cloud key.
    pub api_key_env: String,
    #[serde(skip)]
    pub api_key: Option<String>,
}

/// USD per 1M tokens; used for the counterfactual saving against the top tier.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionCfg {
    pub ttl_secs: u64,
    pub max_pins: usize,
}

impl Default for SessionCfg {
    fn default() -> Self {
        Self {
            ttl_secs: 300,
            max_pins: 10_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogCfg {
    pub path: String,
}

impl Default for LogCfg {
    fn default() -> Self {
        Self {
            path: "slm-router.jsonl".into(),
        }
    }
}

impl Config {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let mut cfg: Self = toml::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
        tier::check(&cfg).map_err(|e| format!("invalid config: {e}"))?;
        cfg.routing.tiers = cfg.tier.iter().map(TierCfg::spec).collect();
        Ok(cfg)
    }

    /// Read, apply env secrets and an optional bind override, then validate.
    pub fn load(path: &str, bind: Option<String>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let mut cfg = Self::from_toml(&text)?;
        cfg.resolve_secrets(|k| std::env::var(k).ok());
        if let Some(b) = bind {
            cfg.server.bind = b;
        }
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn resolve_secrets(&mut self, env: impl Fn(&str) -> Option<String>) {
        // An empty variable (`KEY=` in a .env template) counts as unset, never as an empty secret.
        let env = |k: &str| env(k).filter(|v| !v.is_empty());
        if let Some(c) = &mut self.cloud {
            c.api_key = env(&c.api_key_env);
        }
        for t in &mut self.tier {
            t.api_key = t.api_key_env.as_deref().and_then(env);
        }
        self.server.api_key = env("ROUTER_API_KEY");
    }

    pub fn validate(&self) -> Result<(), String> {
        let addr: SocketAddr = self
            .server
            .bind
            .parse()
            .map_err(|_| format!("bad bind address {:?}", self.server.bind))?;
        if !addr.ip().is_loopback() && self.server.api_key.is_none() {
            return Err("non-loopback bind requires ROUTER_API_KEY".into());
        }
        if self.session.ttl_secs == 0 || self.session.max_pins == 0 {
            return Err("session.ttl_secs and session.max_pins must be > 0".into());
        }
        let t = self.routing.classifier_threshold;
        if !(t.is_finite() && t > 0.0) {
            return Err("routing.classifier_threshold must be > 0 (above 1 disables it)".into());
        }
        Ok(())
    }
}
