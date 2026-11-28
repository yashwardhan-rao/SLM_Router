//! `slm-router report`: Markdown and JSON from the same summary `stats` prints.

mod common;
use common::*;
use serde_json::{Value, json};
use slm_router::stats::{markdown, summarize, text};

fn fixture() -> String {
    let mut l = vec![
        line(10, "local", false, (100, 50), Some(0.002), 100),
        line(20, "local", false, (10, 5), Some(0.001), 300),
        line(30, "cloud", true, (40, 20), Some(0.0), 900),
    ];
    l.push(
        json!({"v": 1, "ts": 40, "target": "cloud", "tier": "big|one", "failed_over": false,
               "prompt_tokens": 1, "completion_tokens": 0, "saved_usd": 0.0, "total_ms": 50,
               "reason": "forced", "outcome": "upstream_error", "tokens_estimated": true})
        .to_string(),
    );
    // A future schema version is skipped, never misread.
    l.push(json!({"v": 2, "ts": 50, "target": "local"}).to_string());
    l.join("\n") + "\n"
}

#[test]
fn json_and_stats_text_come_from_the_same_numbers() {
    let s = summarize(fixture().as_bytes(), 100, None);
    assert_eq!(
        (s.requests, s.local, s.cloud, s.failovers, s.skipped_lines),
        (4, 2, 2, 1, 1)
    );
    assert_eq!(s.tokens_kept_local, 165);
    assert!((s.saved_usd.unwrap() - 0.003).abs() < 1e-12);
    assert_eq!(
        (s.latency_ms.local.p50, s.latency_ms.local.p95),
        (Some(100), Some(300))
    );
    let j: Value = serde_json::to_value(&s).unwrap();
    assert_eq!(j["v"], 1);
    assert_eq!(j["latency_ms"]["cloud"], json!({"p50": 50, "p95": 900}));
    assert_eq!(j["errors"], json!([{"name": "upstream_error", "count": 1}]));
    let t = text(&s);
    has_lines(
        &t,
        &[
            "requests: 4 (local 2, cloud 2)",
            "failovers: 1",
            "tokens kept local: 165",
            "saved_usd: 0.003000",
            "latency_ms p50/p95 local: 100/300",
            "latency_ms p50/p95 cloud: 50/900",
            "errors: 1 (upstream_error 1)",
            "tokens estimated: 1 of 4 requests",
            "skipped unreadable log lines: 1",
        ],
    );
}

#[test]
fn markdown_has_overview_latency_and_breakdown_tables() {
    let md = markdown(&summarize(fixture().as_bytes(), 100, None), Some("24h"));
    has_lines(
        &md,
        &[
            "# SLM_Router report",
            "Window: last 24h.",
            "| Requests | 4 (local 2, cloud 2) |",
            "| Routed local | 50.0% |",
            "| Saved USD | 0.003000 |",
            "| local | 100 | 300 |",
            "| cloud | 50 | 900 |",
            "| unknown | 3 |",
            "| forced | 1 |",
            "| big\\|one | 1 |",
            "| upstream_error | 1 |",
            "Skipped 1 unreadable log lines.",
        ],
    );
}

#[test]
fn an_empty_log_gives_a_zero_summary() {
    let s = summarize(&b""[..], 0, None);
    assert_eq!(
        (s.requests, s.saved_usd, s.latency_ms.local.p50),
        (0, None, None)
    );
    let md = markdown(&s, None);
    has_lines(
        &md,
        &[
            "No requests logged yet.",
            "Window: all logged requests.",
            "| Saved USD | n/a |",
            "| local | n/a | n/a |",
            "No errors.",
        ],
    );
    assert!(!md.contains("## Tiers"), "{md}");
    assert_eq!(serde_json::to_value(&s).unwrap()["saved_usd"], Value::Null);
}

#[test]
fn report_cli_prints_markdown_or_json_and_handles_a_missing_log() {
    let o = run_cmd("report", "", Some(fixture().into_bytes()), &[]);
    assert!(o.status.success());
    assert!(stdout(&o).starts_with("# SLM_Router report"));
    let o = run_cmd("report", "", Some(fixture().into_bytes()), &["--json"]);
    let j: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(
        (j["requests"].as_u64(), j["v"].as_u64()),
        (Some(4), Some(1))
    );
    let o = run_cmd("report", "", None, &["--json", "--since", "1h"]);
    assert!(o.status.success());
    let j: Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(j["requests"], 0);
    let o = run_cmd("report", "", None, &["--since", "soon"]);
    assert!(!o.status.success());
}

#[tokio::test]
async fn report_reflects_real_routing_against_fake_upstreams() {
    let r = rig(Mode::Ok, Mode::Ok, |_| {}).await;
    post_chat(&r.base, &user("What is the capital of France?"), &[]).await;
    let cloud = json!({"model": "slm-router/cloud",
        "messages": [{"role": "user", "content": "hi"}]});
    post_chat(&r.base, &cloud, &[]).await;
    let log = std::fs::read(r.dir.log()).unwrap();
    let s = summarize(&log[..], u64::MAX, None);
    assert_eq!((s.requests, s.local, s.cloud), (2, 1, 1));
    assert!(s.saved_usd.unwrap() > 0.0);
    assert!(s.latency_ms.local.p50.is_some() && s.latency_ms.cloud.p50.is_some());
    let md = markdown(&s, None);
    has_lines(
        &md,
        &[
            "| Requests | 2 (local 1, cloud 1) |",
            "| default | 1 |",
            "| forced | 1 |",
        ],
    );
}
