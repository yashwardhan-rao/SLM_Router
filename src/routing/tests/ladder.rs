//! Routing over a `[[tier]]` ladder of more than two tiers.

use super::*;

pub(super) const HARD: &str = "prove that the sum of two even numbers is even";

pub(super) fn t(
    local: bool,
    tools: bool,
    ctx: Option<usize>,
    stateful: bool,
    hard: bool,
) -> TierSpec {
    TierSpec {
        local,
        supports_tools: tools,
        max_context_tokens: ctx,
        stateful,
        hard,
    }
}

/// small (local, 4k, no tools) -> mid (remote, 8k, tools) -> top (remote, tools, stateful, hard).
pub(super) fn three() -> Vec<TierSpec> {
    vec![
        t(true, false, Some(4096), false, false),
        t(false, true, Some(8192), false, false),
        t(false, true, None, true, true),
    ]
}

pub(super) fn cfg(tiers: Vec<TierSpec>) -> RoutingConfig {
    RoutingConfig {
        tiers,
        // Keep long prompts out of Rule 5 so context tests test context alone.
        complex_prompt_tokens: 100_000,
        ..RoutingConfig::default()
    }
}

pub(super) fn go(
    tiers: Vec<TierSpec>,
    f: &Features,
    policy: Policy,
    pin: Option<usize>,
) -> (usize, Reason) {
    let d = decide(f, &cfg(tiers), policy, pin).unwrap();
    (d.tier, d.reason)
}

fn err(tiers: Vec<TierSpec>, f: &Features, policy: Policy) -> RouteError {
    decide(f, &cfg(tiers), policy, None).unwrap_err()
}

pub(super) fn text(s: &str) -> Features {
    Features {
        last_user_text: s.into(),
        est_prompt_tokens: 20,
        ..Features::default()
    }
}

pub(super) fn tools() -> Features {
    Features {
        needs_capabilities: true,
        ..text("hi")
    }
}

pub(super) fn tokens(n: usize) -> Features {
    Features {
        est_prompt_tokens: n,
        ..text("hi")
    }
}

pub(super) fn stateful() -> Features {
    Features {
        requires_cloud: true,
        ..text("hi")
    }
}

#[test]
fn each_rule_picks_the_cheapest_tier_that_satisfies_it() {
    let a = Policy::Auto;
    assert_eq!(go(three(), &text("hi"), a, None), (0, Reason::Default));
    assert_eq!(go(three(), &tools(), a, None), (1, Reason::Capability));
    assert_eq!(go(three(), &tokens(6000), a, None), (1, Reason::Context));
    assert_eq!(go(three(), &stateful(), a, None), (2, Reason::Stateful));
    assert_eq!(go(three(), &text(HARD), a, None), (2, Reason::Complexity));
    let mut mid_hard = three();
    mid_hard[1].hard = true;
    assert_eq!(go(mid_hard, &text(HARD), a, None), (1, Reason::Complexity));
    let d = decide(&tools(), &cfg(three()), a, None).unwrap();
    assert_eq!(d.target, Target::Cloud);
}

#[test]
fn with_no_hard_tier_hard_is_dropped_before_capability() {
    // none is hard; only the middle tier has tools.
    let tiers = || {
        vec![
            t(true, false, None, false, false),
            t(false, true, None, false, false),
            t(false, false, None, false, false),
        ]
    };
    let both = Features {
        needs_capabilities: true,
        ..text(HARD)
    };
    let a = Policy::Auto;
    assert_eq!(go(tiers(), &both, a, None), (1, Reason::Capability));
    assert_eq!(go(tiers(), &text(HARD), a, None), (2, Reason::Complexity));
}

#[test]
fn a_policy_that_leaves_one_tier_logs_policy_but_still_checks_it() {
    assert_eq!(
        go(three(), &text(HARD), Policy::LocalOnly, None),
        (0, Reason::Policy)
    );
    assert_eq!(
        err(three(), &stateful(), Policy::LocalOnly),
        RouteError::NeedsCloud
    );
    assert_eq!(
        err(three(), &tokens(6000), Policy::LocalOnly),
        RouteError::ContextExceeded
    );
    // Two remote tiers: content still chooses between them.
    let c = Policy::CloudOnly;
    assert_eq!(go(three(), &text("hi"), c, None), (1, Reason::Default));
    assert_eq!(go(three(), &text(HARD), c, None), (2, Reason::Complexity));
}

#[test]
fn a_single_tier_under_auto_is_default_not_policy() {
    let one = vec![t(false, true, None, true, true)];
    assert_eq!(
        go(one, &text("hi"), Policy::Auto, None),
        (0, Reason::Default)
    );
}

#[test]
fn a_policy_with_no_tier_to_use_is_a_conflict() {
    let remote = vec![t(false, true, None, true, true)];
    assert_eq!(
        err(remote, &stateful(), Policy::LocalOnly),
        RouteError::PolicyConflict
    );
}

#[test]
fn forced_tiers_and_aliases() {
    let forced = |forced| Features {
        forced,
        ..text("hi")
    };
    let d = |f: Features, p| decide(&f, &cfg(three()), p, None);
    assert_eq!(
        d(forced(Forced::Tier(1)), Policy::Auto).map(|d| (d.tier, d.reason)),
        Ok((1, Reason::Forced))
    );
    assert_eq!(
        d(forced(Forced::Cloud), Policy::Auto).map(|d| d.tier),
        Ok(1)
    );
    assert_eq!(
        d(forced(Forced::Tier(2)), Policy::LocalOnly),
        Err(RouteError::PolicyConflict)
    );
    assert_eq!(
        d(forced(Forced::Tier(9)), Policy::Auto),
        Err(RouteError::NoSuchTier)
    );
    let stateful_forced = Features {
        requires_cloud: true,
        ..forced(Forced::Tier(1))
    };
    assert_eq!(
        d(stateful_forced, Policy::Auto),
        Err(RouteError::NeedsCloud)
    );
    let remote = vec![t(false, true, None, true, true)];
    let f = forced(Forced::Local);
    let no_local = decide(&f, &cfg(remote), Policy::Auto, None);
    assert_eq!(no_local, Err(RouteError::NoSuchTier));
}
