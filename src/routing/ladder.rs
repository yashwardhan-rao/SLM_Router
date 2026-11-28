//! The tier ladder: what each tier can serve, and the cheapest one that fits a request. No I/O.

use super::{Decision, Features, Policy, Reason, RouteError, RoutingConfig, Target};
use std::borrow::Cow;

/// What one tier can serve. The ladder is in config order, cheapest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierSpec {
    /// Runs on this machine; `local-only` keeps requests on these tiers.
    pub local: bool,
    pub supports_tools: bool,
    /// None means no limit the router knows of.
    pub max_context_tokens: Option<usize>,
    /// Keeps server-side `/v1/responses` state (`previous_response_id`, `store`, ...).
    pub stateful: bool,
    /// May take prompts Rule 5 calls hard.
    pub hard: bool,
}

impl RoutingConfig {
    /// The ladder `decide` routes over. With no `[[tier]]` list it is the legacy local + cloud
    /// pair, built from the `local_*` settings on each call so changes to them apply.
    pub fn specs(&self) -> Cow<'_, [TierSpec]> {
        if !self.tiers.is_empty() {
            return Cow::Borrowed(&self.tiers);
        }
        let local = TierSpec {
            local: true,
            supports_tools: self.local_supports_tools,
            max_context_tokens: Some(self.local_max_context_tokens),
            stateful: false,
            hard: false,
        };
        let cloud = TierSpec {
            local: false,
            supports_tools: true,
            max_context_tokens: None,
            stateful: true,
            hard: true,
        };
        Cow::Owned(vec![local, cloud])
    }
}

/// Tokens kept free in a tier's context for the reply.
const REPLY_RESERVE: usize = 512;

pub(super) fn fits_context(t: &TierSpec, f: &Features) -> bool {
    t.max_context_tokens
        .is_none_or(|m| f.est_prompt_tokens <= m.saturating_sub(REPLY_RESERVE))
}

/// Hard constraints: a tier failing these cannot serve the request at all.
fn servable(t: &TierSpec, f: &Features) -> bool {
    (t.stateful || !f.requires_cloud) && fits_context(t, f)
}

fn capable(t: &TierSpec, f: &Features) -> bool {
    t.supports_tools || !f.needs_capabilities
}

/// The first rule, in rule order, that `t` fails. `hard` is Rule 5's verdict for the request.
fn fails(t: &TierSpec, f: &Features, hard: Option<Reason>) -> Option<Reason> {
    if !(t.stateful || !f.requires_cloud) {
        return Some(Reason::Stateful);
    }
    if !capable(t, f) {
        return Some(Reason::Capability);
    }
    if !fits_context(t, f) {
        return Some(Reason::Context);
    }
    hard.filter(|_| !t.hard)
}

pub(super) fn dec(specs: &[TierSpec], tier: usize, reason: Reason) -> Decision {
    let target = match specs[tier].local {
        true => Target::Local,
        false => Target::Cloud,
    };
    Decision {
        tier,
        target,
        reason,
    }
}

/// Tiers the policy allows, in ladder order.
pub(super) fn allowed(specs: &[TierSpec], policy: Policy) -> Vec<usize> {
    let ok = |t: &TierSpec| match policy {
        Policy::Auto => true,
        Policy::LocalOnly => t.local,
        Policy::CloudOnly => !t.local,
    };
    (0..specs.len()).filter(|&i| ok(&specs[i])).collect()
}

/// The cheapest candidate meeting every rule. Failing that, drop Rule 5 and then the capability
/// rule, taking the highest candidate left, so a tool-capable tier wins over one without tools.
/// The reason is the first rule the cheapest candidate fails, whichever tier is chosen.
pub(super) fn pick(
    specs: &[TierSpec],
    cands: &[usize],
    f: &Features,
    hard: Option<Reason>,
) -> Result<Decision, RouteError> {
    let Some(&cheapest) = cands.first() else {
        return Err(RouteError::PolicyConflict);
    };
    let reason = fails(&specs[cheapest], f, hard).unwrap_or(Reason::Default);
    let chosen = cands
        .iter()
        .find(|&&i| fails(&specs[i], f, hard).is_none())
        .or_else(|| {
            let full = |i: usize| servable(&specs[i], f) && capable(&specs[i], f);
            cands.iter().rev().find(|&&i| full(i))
        })
        .or_else(|| cands.iter().rev().find(|&&i| servable(&specs[i], f)));
    match chosen {
        Some(&i) => Ok(dec(specs, i, reason)),
        None if f.requires_cloud && !cands.iter().any(|&i| specs[i].stateful) => {
            Err(RouteError::NeedsCloud)
        }
        None => Err(RouteError::ContextExceeded),
    }
}

/// A forced tier must be allowed by the policy and able to serve the request.
pub(super) fn forced(
    specs: &[TierSpec],
    tier: usize,
    f: &Features,
    policy: Policy,
) -> Result<Decision, RouteError> {
    if !allowed(specs, policy).contains(&tier) {
        return Err(RouteError::PolicyConflict);
    }
    let t = &specs[tier];
    if f.requires_cloud && !t.stateful {
        return Err(RouteError::NeedsCloud);
    }
    if !fits_context(t, f) {
        return Err(RouteError::ContextExceeded);
    }
    Ok(dec(specs, tier, Reason::Forced))
}

/// Failover target: the next tier above `from` that the policy allows and that can serve the
/// request, preferring one that also has the capabilities it needs. Never moves down.
pub fn next_up(specs: &[TierSpec], from: usize, f: &Features, policy: Policy) -> Option<usize> {
    let above: Vec<usize> = allowed(specs, policy)
        .into_iter()
        .filter(|&i| i > from && servable(&specs[i], f))
        .collect();
    let capable_one = above.iter().find(|&&i| capable(&specs[i], f));
    capable_one.or(above.first()).copied()
}
