//! `slm-router stats`: summarise the JSONL request log.

use serde::Deserialize;
use std::time::Duration;

/// The log fields stats reads; extra fields in a line are ignored.
#[derive(Deserialize)]
struct Rec {
    ts: u64,
    target: String,
    failed_over: bool,
    prompt_tokens: u64,
    completion_tokens: u64,
    saved_usd: Option<f64>,
    total_ms: u64,
}

/// `30s`, `15m`, `24h` or `7d`.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let bad = || format!("invalid --since value {s:?}: use a number plus s, m, h or d (e.g. 24h)");
    let split = s.len().saturating_sub(1);
    let (num, unit) = s.split_at_checked(split).ok_or_else(bad)?;
    let n: u64 = num.parse().map_err(|_| bad())?;
    let secs = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        _ => return Err(bad()),
    };
    Ok(Duration::from_secs(n * secs))
}

/// Nearest-rank percentile of an ascending slice.
fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.max(1) - 1).copied()
}

fn latency(recs: &[&Rec], target: &str) -> String {
    let mut ms: Vec<u64> = recs
        .iter()
        .filter(|r| r.target == target)
        .map(|r| r.total_ms)
        .collect();
    ms.sort_unstable();
    match (percentile(&ms, 0.5), percentile(&ms, 0.95)) {
        (Some(a), Some(b)) => format!("latency_ms p50/p95 {target}: {a}/{b}"),
        _ => format!("latency_ms p50/p95 {target}: n/a"),
    }
}

pub fn report(log: &str, now_ms: u64, since: Option<Duration>) -> String {
    let parsed: Vec<Option<Rec>> = log
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).ok())
        .collect();
    let skipped = parsed.iter().filter(|r| r.is_none()).count();
    let cutoff = since.map_or(0, |d| now_ms.saturating_sub(d.as_millis() as u64));
    let recs: Vec<&Rec> = parsed.iter().flatten().filter(|r| r.ts >= cutoff).collect();

    let local = recs.iter().filter(|r| r.target == "local").count();
    let cloud = recs.len() - local;
    let pct = if recs.is_empty() {
        0.0
    } else {
        local as f64 * 100.0 / recs.len() as f64
    };
    let kept: u64 = recs
        .iter()
        .filter(|r| r.target == "local")
        .map(|r| r.prompt_tokens + r.completion_tokens)
        .sum();
    let saved: Vec<f64> = recs.iter().filter_map(|r| r.saved_usd).collect();
    let saved = if saved.is_empty() {
        "n/a".to_string()
    } else {
        format!("{:.6}", saved.iter().sum::<f64>())
    };

    let mut out = vec![
        format!("requests: {} (local {local}, cloud {cloud})", recs.len()),
        format!(
            "failovers: {}",
            recs.iter().filter(|r| r.failed_over).count()
        ),
        format!("routed local: {pct:.1}%"),
        format!("tokens kept local: {kept}"),
        format!("saved_usd: {saved}"),
        latency(&recs, "local"),
        latency(&recs, "cloud"),
    ];
    if skipped > 0 {
        out.push(format!("skipped unreadable log lines: {skipped}"));
    }
    out.join("\n")
}
