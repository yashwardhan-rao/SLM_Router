//! Pure routing core: request features + policy + pin in, target + reason out. No I/O.

use serde::{Deserialize, Serialize};

mod classifier;
mod complexity;
mod features;
mod ladder;
#[cfg(test)]
mod tests;

pub use complexity::RoutingConfig;
pub use ladder::{TierSpec, next_up};

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
    /// `slm-router/local`: the cheapest local tier.
    Local,
    /// `slm-router/cloud`: the cheapest non-local tier.
    Cloud,
    /// `slm-router/<tier name>`, resolved to its ladder index.
    Tier(usize),
}

/// Rule that decided the route; logged verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    LocalUnavailable,
    /// A non-local tier was unreachable or answered 5xx, and the request moved up the ladder.
    TierUnavailable,
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
    /// Index into the ladder.
    pub tier: usize,
    /// Whether that tier is local; what the log's `target` field records.
    pub target: Target,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteError {
    /// Forced model contradicts the effective policy (HTTP 400).
    PolicyConflict,
    /// Prompt exceeds the context of every tier it may use (HTTP 400).
    ContextExceeded,
    /// Request depends on server-side state, but no tier it may use keeps it (HTTP 400).
    NeedsCloud,
    /// `slm-router/local` or `slm-router/cloud` with no such tier configured (HTTP 400).
    NoSuchTier,
}

#[derive(Debug, Clone, Default)]
pub struct Features {
    pub forced: Forced,
    /// Images, tools, tool_choice or JSON-schema response_format present.
    pub needs_capabilities: bool,
    pub est_prompt_tokens: usize,
    pub last_user_text: String,
    /// Stateful `/v1/responses` fields; a tier without `stateful` would silently drop them.
    pub requires_cloud: bool,
}

pub fn decide(
    f: &Features,
    cfg: &RoutingConfig,
    policy: Policy,
    pin: Option<usize>,
) -> Result<Decision, RouteError> {
    let specs = cfg.specs();
    let specs = &specs[..];
    let alias = |local: bool| specs.iter().position(|t| t.local == local);
    let forced = match f.forced {
        Forced::Auto => None,
        Forced::Local => Some(alias(true).ok_or(RouteError::NoSuchTier)?),
        Forced::Cloud => Some(alias(false).ok_or(RouteError::NoSuchTier)?),
        Forced::Tier(i) if i < specs.len() => Some(i),
        Forced::Tier(_) => return Err(RouteError::NoSuchTier),
    };
    if let Some(tier) = forced {
        return ladder::forced(specs, tier, f, policy);
    }
    let cands = ladder::allowed(specs, policy);
    let hard = complexity::rule5(&f.last_user_text, f.est_prompt_tokens, cfg);
    let auto = ladder::pick(specs, &cands, f, hard)?;
    if policy != Policy::Auto {
        // A policy that leaves one tier decided the route; with several, content still chooses.
        return Ok(match cands.len() {
            1 => ladder::dec(specs, auto.tier, Reason::Policy),
            _ => auto,
        });
    }
    // Rule 2: a pin sticks and only escalates. Below the pin, take the cheapest tier at or above
    // it that fits; if none can serve the request, the pin is ignored for this request.
    Ok(match pin {
        Some(p) if auto.tier < p => {
            let above: Vec<usize> = cands.into_iter().filter(|&i| i >= p).collect();
            ladder::pick(specs, &above, f, hard)
                .map_or(auto, |d| ladder::dec(specs, d.tier, Reason::Pinned))
        }
        Some(p) if auto.tier == p => ladder::dec(specs, p, Reason::Pinned),
        _ => auto,
    })
}

/// The Rule 5 classifier's probability for the last user message, logged so the threshold can be
/// tuned on real traffic. Same inputs as rule 5, so it is the score rule 5's classifier gives (or
/// would give, when an earlier rule, a cue, a pin or forcing decided) this message. None for
/// prompts long enough that rule 5 never runs the classifier, which keeps a huge paste cheap.
pub fn classifier_score(f: &Features, cfg: &RoutingConfig) -> Option<f32> {
    if f.est_prompt_tokens > cfg.complex_prompt_tokens {
        return None;
    }
    complexity::score_of(&f.last_user_text, cfg)
}
