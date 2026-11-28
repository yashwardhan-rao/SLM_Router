//! One pass over the JSONL log into a `Summary`, which `stats`, `report` and `report --json` all
//! render, so their numbers cannot disagree.

use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::time::Duration;

/// The log fields stats reads; extra fields in a line are ignored.
#[derive(Deserialize)]
struct Rec {
    /// Log schema version; absent in hand-written lines, which are read as version 1.
    #[serde(default = "one")]
    v: u8,
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
    /// Absent before v0.6.0, when `target` was the tier.
    #[serde(default)]
    tier: Option<String>,
}

fn one() -> u8 {
    1
}

fn unknown() -> String {
    "unknown".into()
}

fn ok() -> String {
    "ok".into()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Count {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Latency {
    pub p50: Option<u64>,
    pub p95: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Latencies {
    pub local: Latency,
    pub cloud: Latency,
}

/// Everything `stats` and `report` print. Its JSON form is the `report --json` schema, version 1.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub v: u8,
    pub requests: usize,
    pub local: usize,
    pub cloud: usize,
    pub failovers: usize,
    pub pct_local: f64,
    pub tokens_kept_local: u64,
    /// Null when no line in the window carries a price-based saving.
    pub saved_usd: Option<f64>,
    pub latency_ms: Latencies,
    /// Most frequent first, ties by name.
    pub reasons: Vec<Count>,
    pub tiers: Vec<Count>,
    /// Outcomes other than ok and client_disconnect.
    pub errors: Vec<Count>,
    pub client_disconnects: usize,
    pub tokens_estimated: usize,
    /// Lines that are not valid JSON, lack required fields, or carry another log schema version.
    pub skipped_lines: usize,
}

fn counts<'a>(names: impl Iterator<Item = &'a str>) -> Vec<Count> {
    let mut m = std::collections::BTreeMap::<&str, usize>::new();
    names.for_each(|n| *m.entry(n).or_default() += 1);
    let mut v: Vec<Count> = m
        .into_iter()
        .map(|(n, c)| Count {
            name: n.to_string(),
            count: c,
        })
        .collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.count));
    v
}

/// Nearest-rank percentile of an ascending slice.
fn percentile(sorted: &[u64], p: f64) -> Option<u64> {
    let rank = (p * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.max(1) - 1).copied()
}

fn latency(recs: &[Rec], target: &str) -> Latency {
    let mut ms: Vec<u64> = recs
        .iter()
        .filter(|r| r.target == target)
        .map(|r| r.total_ms)
        .collect();
    ms.sort_unstable();
    Latency {
        p50: percentile(&ms, 0.5),
        p95: percentile(&ms, 0.95),
    }
}

/// Streams the log one line at a time; only the records inside the window are kept in memory.
pub fn summarize(log: impl BufRead, now_ms: u64, since: Option<Duration>) -> Summary {
    let cutoff = since.map_or(0, |d| now_ms.saturating_sub(d.as_millis() as u64));
    let (mut skipped, mut recs) = (0, Vec::new());
    for line in log.split(b'\n') {
        let Ok(line) = line else {
            skipped += 1; // a read error ends the stream; report what was read
            break;
        };
        if line.trim_ascii().is_empty() {
            continue;
        }
        match serde_json::from_slice::<Rec>(&line) {
            Ok(r) if r.v != 1 => skipped += 1,
            Ok(r) if r.ts >= cutoff => recs.push(r),
            Ok(_) => {}
            Err(_) => skipped += 1,
        }
    }
    let is_local = |r: &&Rec| r.target == "local";
    let local = recs.iter().filter(is_local).count();
    let saved: Vec<f64> = recs.iter().filter_map(|r| r.saved_usd).collect();
    let tiers: Vec<String> = recs
        .iter()
        .map(|r| r.tier.clone().unwrap_or_else(|| r.target.clone()))
        .collect();
    Summary {
        v: 1,
        requests: recs.len(),
        local,
        cloud: recs.len() - local,
        failovers: recs.iter().filter(|r| r.failed_over).count(),
        pct_local: match recs.len() {
            0 => 0.0,
            n => local as f64 * 100.0 / n as f64,
        },
        tokens_kept_local: recs
            .iter()
            .filter(is_local)
            .map(|r| r.prompt_tokens + r.completion_tokens)
            .sum(),
        saved_usd: (!saved.is_empty()).then(|| saved.iter().sum()),
        latency_ms: Latencies {
            local: latency(&recs, "local"),
            cloud: latency(&recs, "cloud"),
        },
        reasons: counts(recs.iter().map(|r| r.reason.as_str())),
        tiers: counts(tiers.iter().map(String::as_str)),
        // A client hanging up is not the router's or upstream's failure; it is counted apart.
        errors: counts(
            recs.iter()
                .map(|r| r.outcome.as_str())
                .filter(|o| *o != "ok" && *o != "client_disconnect"),
        ),
        client_disconnects: recs
            .iter()
            .filter(|r| r.outcome == "client_disconnect")
            .count(),
        tokens_estimated: recs.iter().filter(|r| r.tokens_estimated).count(),
        skipped_lines: skipped,
    }
}
