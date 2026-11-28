//! `[[tier]]` entries, and the legacy `[local]` + `[cloud]` pair desugared into a two-tier ladder.

use super::{Config, Pricing, default_breaker_cooldown_ms, default_breaker_failures};
use crate::routing::TierSpec;
use serde::Deserialize;
use std::{collections::HashSet, time::Duration};

/// One OpenAI-compatible upstream on the ladder.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TierCfg {
    /// Logged, and forced with `slm-router/<name>`.
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// Name of the env var holding this tier's key; none sends no key.
    pub api_key_env: Option<String>,
    #[serde(skip)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub supports_tools: bool,
    pub max_context_tokens: Option<usize>,
    #[serde(default)]
    pub stateful: bool,
    #[serde(default)]
    pub hard: bool,
    pub input_per_mtok: Option<f64>,
    pub output_per_mtok: Option<f64>,
    pub first_byte_timeout_ms: Option<u64>,
    /// Consecutive failures that open this tier's breaker; 0 disables it.
    #[serde(default = "default_breaker_failures")]
    pub breaker_failures: u32,
    #[serde(default = "default_breaker_cooldown_ms")]
    pub breaker_cooldown_ms: u64,
}

/// First-byte wait for a local tier with none set: long enough for a cold model load.
const LOCAL_FIRST_BYTE_MS: u64 = 15_000;

impl TierCfg {
    pub fn spec(&self) -> TierSpec {
        TierSpec {
            local: self.local,
            supports_tools: self.supports_tools,
            max_context_tokens: self.max_context_tokens,
            stateful: self.stateful,
            hard: self.hard,
        }
    }

    pub fn pricing(&self) -> Option<Pricing> {
        Some(Pricing {
            input_per_mtok: self.input_per_mtok?,
            output_per_mtok: self.output_per_mtok?,
        })
    }

    /// The set timeout; else 15 s for a local tier and none for a remote one.
    pub fn first_byte_timeout(&self) -> Option<Duration> {
        let local = self.local.then_some(LOCAL_FIRST_BYTE_MS);
        self.first_byte_timeout_ms
            .or(local)
            .map(Duration::from_millis)
    }
}

impl Config {
    /// The ladder to serve. The legacy form becomes `local` then `cloud`, with today's semantics:
    /// cloud takes tools, any context, stateful and hard prompts, and has no breaker.
    pub fn ladder(&self) -> Vec<TierCfg> {
        let (Some(l), Some(c)) = (&self.local, &self.cloud) else {
            return self.tier.clone();
        };
        let local = TierCfg {
            name: "local".into(),
            base_url: l.base_url.clone(),
            model: l.model.clone(),
            api_key_env: None,
            api_key: None,
            local: true,
            supports_tools: self.routing.local_supports_tools,
            max_context_tokens: Some(self.routing.local_max_context_tokens),
            stateful: false,
            hard: false,
            input_per_mtok: None,
            output_per_mtok: None,
            first_byte_timeout_ms: Some(l.first_byte_timeout_ms),
            breaker_failures: l.breaker_failures,
            breaker_cooldown_ms: l.breaker_cooldown_ms,
        };
        let cloud = TierCfg {
            name: "cloud".into(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
            api_key_env: Some(c.api_key_env.clone()),
            api_key: c.api_key.clone(),
            local: false,
            supports_tools: true,
            max_context_tokens: None,
            stateful: true,
            hard: true,
            input_per_mtok: self.pricing.map(|p| p.input_per_mtok),
            output_per_mtok: self.pricing.map(|p| p.output_per_mtok),
            first_byte_timeout_ms: None,
            breaker_failures: 0,
            breaker_cooldown_ms: default_breaker_cooldown_ms(),
        };
        vec![local, cloud]
    }

    /// Settings that parse but likely do not do what was meant. Printed at startup, never fatal.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.tier.is_empty() {
            return out;
        }
        // shortcut: a key set to its default value is not caught; serde cannot tell us it was set.
        let d = crate::routing::RoutingConfig::default();
        if self.routing.local_supports_tools != d.local_supports_tools
            || self.routing.local_max_context_tokens != d.local_max_context_tokens
        {
            out.push(
                "[routing] local_supports_tools / local_max_context_tokens are ignored with \
                 [[tier]]; set supports_tools / max_context_tokens on each tier"
                    .into(),
            );
        }
        let Some(top) = self.tier.last().and_then(TierCfg::pricing) else {
            return out;
        };
        for t in &self.tier[..self.tier.len() - 1] {
            if let Some(p) = t.pricing()
                && (p.input_per_mtok > top.input_per_mtok
                    || p.output_per_mtok > top.output_per_mtok)
            {
                out.push(format!(
                    "tier {:?} is priced above the top tier; its saved_usd will be negative",
                    t.name
                ));
            }
        }
        out
    }
}

fn check_price(what: &str, p: Option<f64>) -> Result<(), String> {
    match p {
        Some(v) if !(v.is_finite() && v >= 0.0) => Err(format!("{what} must be a number >= 0")),
        _ => Ok(()),
    }
}

/// Shape checks every command needs, so `ladder()` never sees a half-written config.
pub(super) fn check(cfg: &Config) -> Result<(), String> {
    let legacy = (cfg.local.is_some(), cfg.cloud.is_some());
    match (cfg.tier.is_empty(), legacy) {
        (true, (true, true)) | (false, (false, false)) => {}
        (false, _) => return Err("use [[tier]] entries or [local] + [cloud], not both".into()),
        (true, _) => {
            return Err("config needs [[tier]] entries, or both [local] and [cloud]".into());
        }
    }
    if let Some(p) = cfg.pricing {
        if !cfg.tier.is_empty() {
            return Err("[pricing] is for [local] + [cloud]; set prices on each [[tier]]".into());
        }
        check_price("pricing.input_per_mtok", Some(p.input_per_mtok))?;
        check_price("pricing.output_per_mtok", Some(p.output_per_mtok))?;
    }
    let mut seen = HashSet::new();
    for t in &cfg.tier {
        let n = &t.name;
        if n.is_empty() || n == "auto" || !seen.insert(n.as_str()) {
            return Err(format!(
                "tier name {n:?} must be non-empty, unique and not \"auto\""
            ));
        }
        if t.input_per_mtok.is_some() != t.output_per_mtok.is_some() {
            return Err(format!(
                "tier {n:?}: set both input_per_mtok and output_per_mtok, or neither"
            ));
        }
        check_price(&format!("tier {n:?} input_per_mtok"), t.input_per_mtok)?;
        check_price(&format!("tier {n:?} output_per_mtok"), t.output_per_mtok)?;
    }
    Ok(())
}
