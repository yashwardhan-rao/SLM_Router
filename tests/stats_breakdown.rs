//! `stats` lines that explain routing: reasons, errors and estimated tokens.

mod common;
use common::*;
use serde_json::json;

#[test]
fn report_explains_why_requests_were_routed_and_how_many_failed() {
    let t = now_ms();
    let l = |target, reason, outcome, est| {
        json!({"ts": t, "target": target, "failed_over": false, "prompt_tokens": 1,
               "completion_tokens": 1, "saved_usd": null, "total_ms": 10,
               "reason": reason, "outcome": outcome, "tokens_estimated": est})
        .to_string()
    };
    let lines = vec![
        l("local", "default", "ok", false),
        l("local", "default", "ok", false),
        l("cloud", "classifier", "ok", true),
        l("cloud", "complexity", "upstream_error", false),
        l("cloud", "classifier", "client_disconnect", true),
        l("local", "default", "upstream_error", false),
    ];
    let text = stdout(&run_stats(Some(&lines), &[]));
    has_lines(
        &text,
        &[
            "reasons: default 3, classifier 2, complexity 1",
            "errors: 2 (upstream_error 2)",
            "client disconnects: 1",
            "tokens estimated: 2 of 6 requests",
        ],
    );
}

#[test]
fn lines_without_reason_or_outcome_are_not_counted_as_errors() {
    let lines = vec![line(now_ms(), "local", false, (1, 1), None, 10)];
    let text = stdout(&run_stats(Some(&lines), &[]));
    has_lines(
        &text,
        &[
            "reasons: unknown 1",
            "errors: 0",
            "client disconnects: 0",
            "tokens estimated: 0 of 1 requests",
        ],
    );
}

#[test]
fn report_counts_tiers_and_falls_back_to_target_for_old_lines() {
    let t = now_ms();
    let mut lines = vec![line(t, "local", false, (1, 1), None, 10)];
    for tier in ["mid", "mid", "top"] {
        let l = json!({"ts": t, "target": "cloud", "tier": tier, "failed_over": false,
                       "prompt_tokens": 1, "completion_tokens": 1, "saved_usd": null, "total_ms": 10});
        lines.push(l.to_string());
    }
    let text = stdout(&run_stats(Some(&lines), &[]));
    has_lines(&text, &["tiers: mid 2, local 1, top 1"]);
}
