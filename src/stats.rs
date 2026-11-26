//! `slm-router stats`: summarise the JSONL request log.

use serde::Deserialize;
use std::io::BufRead;
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
    // Defaults keep hand-written or partial lines readable without counting them as errors.
    #[serde(default = "unknown")]
    reason: String,
    #[serde(default = "ok")]
    outcome: String,
    #[serde(default)]
    tokens_estimated: bool,
}

fn unknown() -> String {
    "unknown".into()
}

fn ok() -> String {
    "ok".into()
}

/// `name count` pairs, most frequent first, ties by name.
fn counts<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut m = std::collections::BTreeMap::<&str, usize>::new();
    names.for_each(|n| *m.entry(n).or_default() += 1);
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    let parts: Vec<String> = v.iter().map(|(n, c)| format!("{n} {c}")).collect();
    parts.join(", ")
}

/// `30s`, `15m`, `24h` or `7d`.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let bad = || format!("invalid --since value {s:?}: use a number plus s, m, h or d (e.g. 24h)");
    let split = s.len().saturating_sub(1);
    let (num, unit) = s.split_at_checked(split).ok_or_else(bad)?;
    let n: u64 = num.parse().map_err(|_| bad())?;
    let ms_per_unit: u64 = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(bad()),
    };
    // Milliseconds, because that is what the log timestamps use; a bigger value cannot be compared.
    n.checked_mul(ms_per_unit)
        .map(Duration::from_millis)
        .ok_or_else(bad)
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

/// Streams the log one line at a time; only the records inside the window are kept in memory.
pub fn report(log: impl BufRead, now_ms: u64, since: Option<Duration>) -> String {
    let cutoff = since.map_or(0, |d| now_ms.saturating_sub(d.as_millis() as u64));
    let (mut skipped, mut all) = (0, Vec::new());
    for line in log.split(b'\n') {
        let Ok(line) = line else {
            skipped += 1; // a read error ends the stream; report what was read
            break;
        };
        if line.trim_ascii().is_empty() {
            continue;
        }
        match serde_json::from_slice::<Rec>(&line) {
            Ok(r) if r.ts >= cutoff => all.push(r),
            Ok(_) => {}
            Err(_) => skipped += 1,
        }
    }
    let recs: Vec<&Rec> = all.iter().collect();

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
    if !recs.is_empty() {
        out.push(format!(
            "reasons: {}",
            counts(recs.iter().map(|r| r.reason.as_str()))
        ));
    }
    let failed = recs.iter().filter(|r| r.outcome != "ok");
    out.push(match failed.clone().count() {
        0 => "errors: 0".to_string(),
        n => format!(
            "errors: {n} ({})",
            counts(failed.map(|r| r.outcome.as_str()))
        ),
    });
    let est = recs.iter().filter(|r| r.tokens_estimated).count();
    out.push(format!(
        "tokens estimated: {est} of {} requests",
        recs.len()
    ));
    if skipped > 0 {
        out.push(format!("skipped unreadable log lines: {skipped}"));
    }
    out.join("\n")
}
