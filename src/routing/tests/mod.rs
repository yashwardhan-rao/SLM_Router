use super::*;

/// Ladder index of a target in the legacy two-tier ladder.
fn idx(t: Target) -> usize {
    match t {
        Target::Local => 0,
        Target::Cloud => 1,
    }
}

fn route(f: &Features, policy: Policy, pin: Option<Target>) -> Result<Decision, RouteError> {
    decide(f, &RoutingConfig::default(), policy, pin.map(idx))
}

fn ok(target: Target, reason: Reason) -> Result<Decision, RouteError> {
    let tier = idx(target);
    Ok(Decision {
        tier,
        target,
        reason,
    })
}
mod classifier;
mod complexity;
mod content;
mod features;
mod ladder;
mod ladder_moves;
mod policy;
mod stateful;
