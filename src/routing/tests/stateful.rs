use super::*;

fn stateful() -> Features {
    Features {
        requires_cloud: true,
        ..Default::default()
    }
}

#[test]
fn stateful_goes_to_cloud_even_when_local_supports_tools_and_is_pinned() {
    let cfg = RoutingConfig {
        local_supports_tools: true,
        ..Default::default()
    };
    assert_eq!(
        decide(&stateful(), &cfg, Policy::Auto, Some(0)),
        ok(Target::Cloud, Reason::Stateful)
    );
}

#[test]
fn stateful_cannot_be_forced_or_pinned_by_policy_to_local() {
    let forced = Features {
        forced: Forced::Local,
        ..stateful()
    };
    assert_eq!(
        route(&forced, Policy::Auto, None),
        Err(RouteError::NeedsCloud)
    );
    assert_eq!(
        route(&stateful(), Policy::LocalOnly, None),
        Err(RouteError::NeedsCloud)
    );
}

#[test]
fn stateful_under_cloud_only_is_a_policy_route() {
    assert_eq!(
        route(&stateful(), Policy::CloudOnly, None),
        ok(Target::Cloud, Reason::Policy)
    );
}
