//! Pure routing core: request features + policy + pin in, target + reason out. No I/O.

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Local,
    Cloud,
}

/// Effective policy after merging config default and the client header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    #[default]
    Auto,
    LocalOnly,
    CloudOnly,
}

/// What the client's `model` field asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Forced {
    #[default]
    Auto,
    Local,
    Cloud,
}

/// Rule that decided the route; logged verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    LocalUnavailable,
    Forced,
    Policy,
    Pinned,
    Capability,
    Context,
    Complexity,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub target: Target,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// Forced model contradicts the effective policy (HTTP 400).
    PolicyConflict,
    /// Prompt exceeds the local context and policy forbids cloud (HTTP 400).
    ContextExceeded,
}

#[derive(Debug, Clone, Default)]
pub struct Features {
    pub forced: Forced,
    /// Images, tools, tool_choice or JSON-schema response_format present.
    pub needs_capabilities: bool,
    pub est_prompt_tokens: usize,
    pub last_user_text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoutingConfig {
    pub local_supports_tools: bool,
    pub local_max_context_tokens: usize,
    pub complex_prompt_tokens: usize,
    pub cue_min_chars: usize,
    pub reasoning_cues: Vec<String>,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            local_supports_tools: false,
            local_max_context_tokens: 8000,
            complex_prompt_tokens: 1500,
            cue_min_chars: 200,
            reasoning_cues: ["prove", "step by step", "refactor", "analyze", "derive"]
                .map(String::from)
                .to_vec(),
        }
    }
}

fn dec(target: Target, reason: Reason) -> Decision {
    Decision { target, reason }
}

/// Rules 3-6: content-based routing, ignoring forcing, policy and pins.
fn auto_route(f: &Features, cfg: &RoutingConfig) -> Decision {
    if f.needs_capabilities && !cfg.local_supports_tools {
        return dec(Target::Cloud, Reason::Capability);
    }
    if f.est_prompt_tokens > cfg.local_max_context_tokens {
        return dec(Target::Cloud, Reason::Context);
    }
    if f.est_prompt_tokens > cfg.complex_prompt_tokens || has_reasoning_cue(f, cfg) {
        return dec(Target::Cloud, Reason::Complexity);
    }
    dec(Target::Local, Reason::Default)
}

fn has_reasoning_cue(f: &Features, cfg: &RoutingConfig) -> bool {
    if f.last_user_text.chars().count() <= cfg.cue_min_chars {
        return false;
    }
    let text = f.last_user_text.to_lowercase();
    cfg.reasoning_cues
        .iter()
        .any(|c| text.contains(&c.to_lowercase()))
}

pub fn decide(
    f: &Features,
    cfg: &RoutingConfig,
    policy: Policy,
    pin: Option<Target>,
) -> Result<Decision, RouteError> {
    let forced = match (f.forced, policy) {
        (Forced::Cloud, Policy::LocalOnly) | (Forced::Local, Policy::CloudOnly) => {
            return Err(RouteError::PolicyConflict);
        }
        (Forced::Cloud, _) => Some((Target::Cloud, Reason::Forced)),
        (Forced::Local, _) => Some((Target::Local, Reason::Forced)),
        (Forced::Auto, Policy::CloudOnly) => Some((Target::Cloud, Reason::Policy)),
        (Forced::Auto, Policy::LocalOnly) => Some((Target::Local, Reason::Policy)),
        (Forced::Auto, Policy::Auto) => None,
    };
    let over_context = f.est_prompt_tokens > cfg.local_max_context_tokens;
    match forced {
        Some((Target::Local, _)) if over_context => Err(RouteError::ContextExceeded),
        Some((target, reason)) => Ok(dec(target, reason)),
        None => Ok(apply_pin(pin, auto_route(f, cfg))),
    }
}

/// Rule 2: a pin sticks; only local -> cloud escalation may change it.
fn apply_pin(pin: Option<Target>, auto: Decision) -> Decision {
    match pin {
        Some(Target::Cloud) => dec(Target::Cloud, Reason::Pinned),
        Some(Target::Local) if auto.target == Target::Local => dec(Target::Local, Reason::Pinned),
        _ => auto,
    }
}
