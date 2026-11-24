use super::*;

fn route(f: &Features, policy: Policy, pin: Option<Target>) -> Result<Decision, RouteError> {
    decide(f, &RoutingConfig::default(), policy, pin)
}

fn ok(target: Target, reason: Reason) -> Result<Decision, RouteError> {
    Ok(Decision { target, reason })
}
mod content;
mod policy;
mod stateful;
