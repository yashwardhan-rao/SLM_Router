//! Helpers for tests that run the `slm-router stats` CLI.

use super::TempDir;
use serde_json::json;
use std::process::{Command, Output};

pub fn now_ms() -> u64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    d.unwrap().as_millis() as u64
}

/// One log line with only the fields `stats` reads.
pub fn line(
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

pub fn run_stats(log_lines: Option<&[String]>, extra: &[&str]) -> Output {
    run_stats_with("", log_lines, extra)
}

/// `prefix` is TOML placed before the required sections, e.g. a `[server]` table.
pub fn run_stats_with(prefix: &str, log_lines: Option<&[String]>, extra: &[&str]) -> Output {
    let raw = log_lines.map(|l| (l.join("\n") + "\n").into_bytes());
    run_stats_raw(prefix, raw, extra)
}

pub fn run_stats_raw(prefix: &str, raw: Option<Vec<u8>>, extra: &[&str]) -> Output {
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

pub fn has_lines(text: &str, wants: &[&str]) {
    for want in wants {
        assert!(
            text.lines().any(|l| l == *want),
            "missing {want:?} in:\n{text}"
        );
    }
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
