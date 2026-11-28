//! `slm-router stats` and `slm-router report`: summarise the JSONL request log.

use std::io::BufRead;
use std::time::Duration;

mod markdown;
mod summary;
pub use markdown::markdown;
pub use summary::{Count, Latencies, Latency, Summary, summarize};

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

fn joined(c: &[Count]) -> String {
    let parts: Vec<String> = c
        .iter()
        .map(|c| format!("{} {}", c.name, c.count))
        .collect();
    parts.join(", ")
}

fn latency(l: Latency, target: &str) -> String {
    match (l.p50, l.p95) {
        (Some(a), Some(b)) => format!("latency_ms p50/p95 {target}: {a}/{b}"),
        _ => format!("latency_ms p50/p95 {target}: n/a"),
    }
}

/// The `stats` text, rendered from one `summarize` pass over the log.
pub fn report(log: impl BufRead, now_ms: u64, since: Option<Duration>) -> String {
    text(&summarize(log, now_ms, since))
}

pub fn text(s: &Summary) -> String {
    let saved = s.saved_usd.map_or("n/a".to_string(), |v| format!("{v:.6}"));
    let mut out = vec![
        format!(
            "requests: {} (local {}, cloud {})",
            s.requests, s.local, s.cloud
        ),
        format!("failovers: {}", s.failovers),
        format!("routed local: {:.1}%", s.pct_local),
        format!("tokens kept local: {}", s.tokens_kept_local),
        format!("saved_usd: {saved}"),
        latency(s.latency_ms.local, "local"),
        latency(s.latency_ms.cloud, "cloud"),
    ];
    if s.requests > 0 {
        out.push(format!("reasons: {}", joined(&s.reasons)));
        out.push(format!("tiers: {}", joined(&s.tiers)));
    }
    let failed: usize = s.errors.iter().map(|c| c.count).sum();
    out.push(match failed {
        0 => "errors: 0".to_string(),
        n => format!("errors: {n} ({})", joined(&s.errors)),
    });
    out.push(format!("client disconnects: {}", s.client_disconnects));
    out.push(format!(
        "tokens estimated: {} of {} requests",
        s.tokens_estimated, s.requests
    ));
    if s.skipped_lines > 0 {
        out.push(format!("skipped unreadable log lines: {}", s.skipped_lines));
    }
    out.join("\n")
}
