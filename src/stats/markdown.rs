//! `slm-router report`: the summary as a shareable Markdown document.

use super::{Count, Latency, Summary};

/// Tier names and reasons come from config and the log; keep them from breaking a table row.
fn cell(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(['\n', '\r'], " ")
}

fn ms(v: Option<u64>) -> String {
    v.map_or("n/a".into(), |v| v.to_string())
}

fn table(out: &mut Vec<String>, title: &str, col: &str, rows: &[Count]) {
    out.push(format!("\n## {title}\n"));
    out.push(format!("| {col} | Requests |\n| :--- | ---: |"));
    for c in rows {
        out.push(format!("| {} | {} |", cell(&c.name), c.count));
    }
}

/// `since` is the `--since` value as typed, shown as the report's window.
pub fn markdown(s: &Summary, since: Option<&str>) -> String {
    let window = since.map_or("all logged requests".into(), |d| {
        format!("last {}", cell(d))
    });
    let saved = s.saved_usd.map_or("n/a".into(), |v| format!("{v:.6}"));
    let mut out = vec![
        "# SLM_Router report\n".to_string(),
        format!("Window: {window}."),
    ];
    if s.requests == 0 {
        out.push("\nNo requests logged yet.".into());
    }
    out.push("\n| Metric | Value |\n| :--- | ---: |".into());
    for (k, v) in [
        (
            "Requests",
            format!("{} (local {}, cloud {})", s.requests, s.local, s.cloud),
        ),
        ("Routed local", format!("{:.1}%", s.pct_local)),
        ("Failovers", s.failovers.to_string()),
        ("Tokens kept local", s.tokens_kept_local.to_string()),
        ("Saved USD", saved),
        (
            "Tokens estimated",
            format!("{} of {}", s.tokens_estimated, s.requests),
        ),
        ("Client disconnects", s.client_disconnects.to_string()),
    ] {
        out.push(format!("| {k} | {v} |"));
    }
    out.push("\n## Latency (total ms)\n".into());
    out.push("| Target | p50 | p95 |\n| :--- | ---: | ---: |".into());
    let lat = |t: &str, l: Latency| format!("| {t} | {} | {} |", ms(l.p50), ms(l.p95));
    out.push(lat("local", s.latency_ms.local));
    out.push(lat("cloud", s.latency_ms.cloud));
    if s.requests > 0 {
        table(&mut out, "Routing reasons", "Reason", &s.reasons);
        table(&mut out, "Tiers", "Tier", &s.tiers);
    }
    if s.errors.is_empty() {
        out.push("\nNo errors.".into());
    } else {
        table(&mut out, "Errors", "Outcome", &s.errors);
    }
    if s.skipped_lines > 0 {
        out.push(format!(
            "\nSkipped {} unreadable log lines.",
            s.skipped_lines
        ));
    }
    out.join("\n") + "\n"
}
