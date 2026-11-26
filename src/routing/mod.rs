//! Pure routing core: request features + policy + pin in, target + reason out. No I/O.

use serde::{Deserialize, Serialize};

mod classifier;
mod complexity;
mod features;
#[cfg(test)]
mod tests;

pub use complexity::RoutingConfig;

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
    CircuitOpen,
    Forced,
    Policy,
    Stateful,
    Pinned,
    Capability,
    Context,
    Complexity,
    Classifier,
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
    /// Request depends on server-side state only the cloud keeps, but must go local (HTTP 400).
    NeedsCloud,
}

#[derive(Debug, Clone, Default)]
pub struct Features {
    pub forced: Forced,
    /// Images, tools, tool_choice or JSON-schema response_format present.
    pub needs_capabilities: bool,
    pub est_prompt_tokens: usize,
    pub last_user_text: String,
    /// Stateful `/v1/responses` fields; the local model would silently drop them.
    pub requires_cloud: bool,
}

/// Tokens kept free in the local context for the reply.
const REPLY_RESERVE: usize = 512;

fn over_context(f: &Features, cfg: &RoutingConfig) -> bool {
    f.est_prompt_tokens > cfg.local_max_context_tokens.saturating_sub(REPLY_RESERVE)
}

fn dec(target: Target, reason: Reason) -> Decision {
    Decision { target, reason }
}

/// Rules 3-6: content-based routing, ignoring forcing, policy and pins.
fn auto_route(f: &Features, cfg: &RoutingConfig) -> Decision {
    if f.requires_cloud {
        return dec(Target::Cloud, Reason::Stateful);
    }
    if f.needs_capabilities && !cfg.local_supports_tools {
        return dec(Target::Cloud, Reason::Capability);
    }
    if over_context(f, cfg) {
        return dec(Target::Cloud, Reason::Context);
    }
    if let Some(reason) = complexity::rule5(&f.last_user_text, f.est_prompt_tokens, cfg) {
        return dec(Target::Cloud, reason);
    }
    dec(Target::Local, Reason::Default)
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
    match forced {
        Some((Target::Local, _)) if f.requires_cloud => Err(RouteError::NeedsCloud),
        Some((Target::Local, _)) if over_context(f, cfg) => Err(RouteError::ContextExceeded),
        Some((target, reason)) => Ok(dec(target, reason)),
        None => Ok(apply_pin(pin, auto_route(f, cfg))),
    }
}

/// The Rule 5 classifier's probability for the last user message, logged so the threshold can be
/// tuned on real traffic. The same inputs as rule 5, so it is the score rule 5 compared.
/// None for prompts long enough that rule 5 never runs the classifier, which also keeps a huge
/// paste from paying for it.
pub fn classifier_score(f: &Features, cfg: &RoutingConfig) -> Option<f32> {
    if f.est_prompt_tokens > cfg.complex_prompt_tokens {
        return None;
    }
    complexity::score_of(&f.last_user_text, cfg)
}

/// Rule 2: a pin sticks; only local -> cloud escalation may change it.
fn apply_pin(pin: Option<Target>, auto: Decision) -> Decision {
    match pin {
        Some(Target::Cloud) => dec(Target::Cloud, Reason::Pinned),
        Some(Target::Local) if auto.target == Target::Local => dec(Target::Local, Reason::Pinned),
        _ => auto,
    }
}
