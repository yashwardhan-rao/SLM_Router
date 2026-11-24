use super::*;

#[test]
fn rule1_forced_cloud_model_routes_to_cloud() {
    let f = Features {
        forced: Forced::Cloud,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Forced)
    );
}

#[test]
fn rule2_cloud_pin_keeps_simple_request_on_cloud() {
    let f = Features::default();
    assert_eq!(
        route(&f, Policy::Auto, Some(Target::Cloud)),
        ok(Target::Cloud, Reason::Pinned)
    );
}

#[test]
fn rule2_local_pin_keeps_simple_request_on_local() {
    let f = Features::default();
    assert_eq!(
        route(&f, Policy::Auto, Some(Target::Local)),
        ok(Target::Local, Reason::Pinned)
    );
}

#[test]
fn rule2_local_pin_escalates_to_cloud_when_request_needs_it() {
    let f = Features {
        est_prompt_tokens: 9000,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, Some(Target::Local)),
        ok(Target::Cloud, Reason::Context)
    );
}

#[test]
fn rule1_cloud_only_policy_routes_to_cloud() {
    assert_eq!(
        route(&Features::default(), Policy::CloudOnly, None),
        ok(Target::Cloud, Reason::Policy)
    );
}

#[test]
fn rule1_local_only_policy_routes_to_local_even_for_tools() {
    let f = Features {
        needs_capabilities: true,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::LocalOnly, None),
        ok(Target::Local, Reason::Policy)
    );
}

#[test]
fn rule1_forced_model_conflicting_with_policy_is_an_error() {
    let to_cloud = Features {
        forced: Forced::Cloud,
        ..Default::default()
    };
    let to_local = Features {
        forced: Forced::Local,
        ..Default::default()
    };
    assert_eq!(
        route(&to_cloud, Policy::LocalOnly, None),
        Err(RouteError::PolicyConflict)
    );
    assert_eq!(
        route(&to_local, Policy::CloudOnly, None),
        Err(RouteError::PolicyConflict)
    );
}

#[test]
fn rule1_local_only_with_oversized_prompt_is_an_error() {
    let f = Features {
        est_prompt_tokens: 9000,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::LocalOnly, None),
        Err(RouteError::ContextExceeded)
    );
}

#[test]
fn rule1_policy_beats_pin() {
    assert_eq!(
        route(&Features::default(), Policy::LocalOnly, Some(Target::Cloud)),
        ok(Target::Local, Reason::Policy)
    );
}

#[test]
fn rule1_forced_local_with_oversized_prompt_is_an_error_under_any_policy() {
    let f = Features {
        forced: Forced::Local,
        est_prompt_tokens: 9000,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        Err(RouteError::ContextExceeded)
    );
}
