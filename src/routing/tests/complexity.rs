use super::*;
use crate::routing::complexity::score;

fn s(text: &str) -> u32 {
    score(text, &RoutingConfig::default())
}

fn routed(text: &str) -> Result<Decision, RouteError> {
    let f = Features {
        last_user_text: text.into(),
        ..Default::default()
    };
    route(&f, Policy::Auto, None)
}

#[test]
fn cues_match_whole_words_only() {
    for t in [
        "improve it",
        "approve it",
        "fixing it",
        "debugging it",
        "prefix it",
        "redesigned",
    ] {
        assert_eq!(s(t), 0, "{t}");
    }
}

#[test]
fn hyphens_and_punctuation_separate_words() {
    assert_eq!(s("a lock-free queue"), 1);
    assert_eq!(s("weigh the trade-off"), 2);
    assert_eq!(s("go step-by-step"), 2);
    assert_eq!(s("does it compile?"), 1);
}

#[test]
fn a_trailing_s_on_the_last_cue_word_matches() {
    assert_eq!(s("list the trade-offs"), 2);
    assert_eq!(s("it leaks"), 1);
}

#[test]
fn each_signal_has_its_own_weight() {
    assert_eq!(s("prove it"), 2, "reasoning");
    assert_eq!(s("there is a bug"), 1, "one hint");
    assert_eq!(s("run `ls`"), 1, "backtick");
    assert_eq!(s("x == 1; y != 2"), 1, "two code markers");
    assert_eq!(s("a; b"), 0, "one code marker is not code");
    assert_eq!(s("call foo_bar(x)"), 1, "snake_case plus a call");
    assert_eq!(s("What does O(n) mean?"), 1, "big-O with no second signal");
    assert_eq!(s("is it O(n log n)"), 1, "big-O alone is weak");
    assert_eq!(s("hello(n)"), 0, "big-O needs a word boundary");
}

#[test]
fn reasoning_counts_once_and_hints_cap_at_two() {
    assert_eq!(s("prove and derive, step by step"), 2);
    assert_eq!(s("bug fix deadlock leak panic"), 2);
    assert_eq!(s("bug bug bug"), 1, "hints count distinct cues");
}

#[test]
fn one_weak_signal_stays_local_two_go_to_cloud() {
    assert_eq!(routed("there is a bug"), ok(Target::Local, Reason::Default));
    assert_eq!(
        routed("run `ls` please"),
        ok(Target::Local, Reason::Default)
    );
    for t in ["fix this bug", "fix `x`", "prove it", "why is it O(1)"] {
        assert_eq!(routed(t), ok(Target::Cloud, Reason::Complexity), "{t}");
    }
}

#[test]
fn reasoning_cues_come_from_config() {
    let cfg = RoutingConfig {
        reasoning_cues: vec!["Pontificate".into()],
        ..Default::default()
    };
    assert_eq!(score("please pontificate", &cfg), 2);
    assert_eq!(score("please prove it", &cfg), 0);
}

#[test]
fn threshold_and_min_chars_are_configurable() {
    let f = Features {
        last_user_text: "there is a bug".into(),
        ..Default::default()
    };
    let low = RoutingConfig {
        complexity_threshold: 1,
        ..Default::default()
    };
    assert_eq!(
        decide(&f, &low, Policy::Auto, None),
        ok(Target::Cloud, Reason::Complexity)
    );
    let long_only = RoutingConfig {
        cue_min_chars: 200,
        ..Default::default()
    };
    assert_eq!(score("prove it", &long_only), 0);
}
