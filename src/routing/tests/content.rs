use super::*;

#[test]
fn rule3_images_or_tools_route_to_cloud() {
    let f = Features {
        needs_capabilities: true,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Capability)
    );
}

#[test]
fn rule3_local_supporting_tools_keeps_capability_requests_local() {
    let f = Features {
        needs_capabilities: true,
        ..Default::default()
    };
    let cfg = RoutingConfig {
        local_supports_tools: true,
        ..Default::default()
    };
    assert_eq!(
        decide(&f, &cfg, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

#[test]
fn rule4_prompt_over_local_context_minus_reply_reserve_routes_to_cloud() {
    let f = Features {
        est_prompt_tokens: 3585,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Context)
    );
}

#[test]
fn rule4_prompt_exactly_at_local_context_minus_reply_reserve_stays_local() {
    let f = Features {
        est_prompt_tokens: 3584,
        ..Default::default()
    };
    let cfg = RoutingConfig {
        complex_prompt_tokens: 100_000,
        ..Default::default()
    };
    assert_eq!(
        decide(&f, &cfg, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

#[test]
fn rule5_long_prompt_routes_to_cloud() {
    let f = Features {
        est_prompt_tokens: 1501,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Complexity)
    );
}

#[test]
fn rule5_prompt_exactly_at_complexity_threshold_stays_local() {
    let f = Features {
        est_prompt_tokens: 1500,
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

fn long_text(cue: &str) -> String {
    format!("{cue} {}", "x".repeat(250))
}

#[test]
fn rule5_long_message_with_reasoning_cue_routes_to_cloud() {
    let f = Features {
        last_user_text: long_text("Please PROVE this"),
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Complexity)
    );
}

#[test]
fn rule5_short_message_with_reasoning_cue_routes_to_cloud() {
    let f = Features {
        last_user_text: "prove it".into(),
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Cloud, Reason::Complexity)
    );
}

#[test]
fn rule5_long_message_without_cue_stays_local() {
    let f = Features {
        last_user_text: long_text("hello there"),
        ..Default::default()
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

const CUDA: &str = "Explain how CUDA warp divergence affects kernel throughput.";

fn text(t: &str) -> Features {
    Features {
        last_user_text: t.into(),
        ..Default::default()
    }
}

#[test]
fn rule5_classifier_catches_a_hard_prompt_without_a_cue() {
    assert!(crate::routing::complexity::score(CUDA, &RoutingConfig::default()) < 2);
    assert_eq!(
        route(&text(CUDA), Policy::Auto, None),
        ok(Target::Cloud, Reason::Classifier)
    );
}

#[test]
fn rule5_classifier_threshold_above_one_disables_it() {
    let cfg = RoutingConfig {
        classifier_threshold: 1.01,
        ..Default::default()
    };
    assert_eq!(
        decide(&text(CUDA), &cfg, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

#[test]
fn rule5_cue_hit_still_reports_complexity() {
    assert_eq!(
        route(&text("Prove that CUDA warps diverge."), Policy::Auto, None),
        ok(Target::Cloud, Reason::Complexity)
    );
}

/// The classifier sees the last message's size, not the conversation's: a short follow-up in a
/// long chat must not look hard just because the history is long.
#[test]
fn rule5_short_follow_up_in_a_long_chat_stays_local() {
    let f = Features {
        est_prompt_tokens: 1200,
        ..text("thanks, that worked")
    };
    assert_eq!(
        route(&f, Policy::Auto, None),
        ok(Target::Local, Reason::Default)
    );
}

#[test]
fn the_logged_classifier_score_is_the_one_rule5_compared() {
    let cfg = RoutingConfig::default();
    let hard = classifier_score(&text(CUDA), &cfg).unwrap();
    let easy = classifier_score(&text("hi"), &cfg).unwrap();
    assert!(hard >= cfg.classifier_threshold, "{hard}");
    assert!(easy < cfg.classifier_threshold, "{easy}");
    let off = RoutingConfig {
        classifier_threshold: 1.01,
        ..Default::default()
    };
    assert_eq!(classifier_score(&text(CUDA), &off), None);
}
