mod common;
use common::TempDir;
use serde_json::json;
use std::process::{Command, Output};

fn now_ms() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    d.unwrap().as_millis() as u64
}

/// One log line with only the fields `stats` reads.
fn line(
    ts: u64,
    target: &str,
    failed_over: bool,
    tokens: (u64, u64),
    saved: Option<f64>,
    ms: u64,
) -> String {
    json!({"ts": ts, "target": target, "failed_over": failed_over, "prompt_tokens": tokens.0,
           "completion_tokens": tokens.1, "saved_usd": saved, "total_ms": ms})
    .to_string()
}

fn run_stats(log_lines: Option<&[String]>, extra: &[&str]) -> Output {
    run_stats_with("", log_lines, extra)
}

/// `prefix` is TOML placed before the required sections, e.g. a `[server]` table.
fn run_stats_with(prefix: &str, log_lines: Option<&[String]>, extra: &[&str]) -> Output {
    let raw = log_lines.map(|l| (l.join("\n") + "\n").into_bytes());
    run_stats_raw(prefix, raw, extra)
}

fn run_stats_raw(prefix: &str, raw: Option<Vec<u8>>, extra: &[&str]) -> Output {
    let dir = TempDir::new();
    let log = dir.log();
    if let Some(bytes) = raw {
        std::fs::write(&log, bytes).unwrap();
    }
    let cfg = dir.0.join("c.toml");
    let toml = format!(
        "{prefix}[local]\nbase_url = \"http://x/v1\"\nmodel = \"m\"\n[cloud]\nbase_url = \"http://y/v1\"\n\
         model = \"m\"\napi_key_env = \"X\"\n[log]\npath = {:?}\n",
        log.to_str().unwrap()
    );
    std::fs::write(&cfg, toml).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_slm-router"));
    cmd.args(["stats", "--config", cfg.to_str().unwrap()])
        .args(extra);
    cmd.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn sample() -> Vec<String> {
    let t = now_ms();
    let mut v: Vec<String> = [100, 200, 300, 400, 1000]
        .iter()
        .map(|ms| line(t, "local", false, (10, 5), Some(0.0001), *ms))
        .collect();
    v.push(line(t, "cloud", true, (10, 5), Some(0.0), 500));
    v.push(line(t, "cloud", false, (10, 5), Some(0.0), 700));
    v
}

#[test]
fn report_has_counts_percent_tokens_savings_and_latency_percentiles() {
    let out = run_stats(Some(&sample()), &[]);
    assert!(out.status.success());
    let text = stdout(&out);
    for want in [
        "requests: 7 (local 5, cloud 2)",
        "failovers: 1",
        "routed local: 71.4%",
        "tokens kept local: 75",
        "saved_usd: 0.000500",
        "latency_ms p50/p95 local: 300/1000",
        "latency_ms p50/p95 cloud: 500/700",
    ] {
        assert!(
            text.lines().any(|l| l == want),
            "missing line {want:?} in:\n{text}"
        );
    }
}

#[test]
fn since_excludes_older_requests() {
    let lines = vec![
        line(0, "local", false, (1, 1), None, 10),
        line(now_ms(), "cloud", false, (1, 1), None, 20),
    ];
    let text = stdout(&run_stats(Some(&lines), &["--since", "1h"]));
    assert!(
        text.lines().any(|l| l == "requests: 1 (local 0, cloud 1)"),
        "{text}"
    );
}

#[test]
fn saved_usd_is_na_when_no_pricing_was_configured() {
    let lines = vec![line(now_ms(), "local", false, (1, 1), None, 10)];
    let text = stdout(&run_stats(Some(&lines), &[]));
    assert!(text.lines().any(|l| l == "saved_usd: n/a"), "{text}");
}

#[test]
fn missing_log_is_not_an_error() {
    let out = run_stats(None, &[]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("no requests logged yet"));
}

#[test]
fn bad_since_value_fails_with_a_message() {
    let out = run_stats(Some(&sample()), &["--since", "soon"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--since"));
}

#[test]
fn stats_does_not_apply_serve_only_validation() {
    // A server config that binds publicly needs ROUTER_API_KEY to serve, but not to read a report.
    let server = "[server]\nbind = \"0.0.0.0:8080\"\n";
    let out = run_stats_with(server, Some(&sample()), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains("requests: 7"));
}

#[test]
fn a_huge_since_value_is_a_clean_error_not_a_panic() {
    let out = run_stats(Some(&sample()), &["--since", "213503982334601d"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("--since") && !err.contains("panicked"),
        "{err}"
    );
}

#[test]
fn invalid_utf8_line_is_skipped_not_fatal() {
    let mut raw = sample().join("\n").into_bytes();
    raw.extend_from_slice(b"\n\xff\xfe not utf8\n");
    let o = run_stats_raw("", Some(raw), &[]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout(&o);
    assert!(out.contains("requests: 7"), "{out}");
    assert!(out.contains("skipped unreadable log lines: 1"), "{out}");
}
