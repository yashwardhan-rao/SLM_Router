//! Token accounting and the counterfactual cloud saving for the request log.

use crate::logging::{Record, now_ms};
use crate::routing::{Policy, Reason, Target};
use crate::{config::Pricing, session::SessionSource};
use serde_json::Value;

/// (prompt, completion, estimated). Uses the upstream `usage` when present, else a chars/4 estimate.
pub fn usage(body: &[u8], est_prompt: usize) -> (u64, u64, bool) {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let u = &v["usage"];
    match (u["prompt_tokens"].as_u64(), u["completion_tokens"].as_u64()) {
        (Some(p), Some(c)) => (p, c, false),
        _ => {
            let text = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
            (est_prompt as u64, (text.chars().count() / 4) as u64, true)
        }
    }
}

/// What cloud would have cost for a request served locally. Null when no pricing is configured.
pub fn saved_usd(
    target: Target,
    pricing: Option<Pricing>,
    prompt: u64,
    completion: u64,
) -> Option<f64> {
    let p = pricing?;
    Some(match target {
        Target::Cloud => 0.0,
        Target::Local => {
            (prompt as f64 * p.input_per_mtok + completion as f64 * p.output_per_mtok) / 1e6
        }
    })
}

pub fn new_record(seq: u64, session_id: String, source: SessionSource, policy: Policy) -> Record {
    let ts = now_ms();
    Record {
        v: 1,
        ts,
        request_id: format!("{ts:x}-{seq}"),
        session_id,
        session_source: source,
        policy,
        target: Target::Local,
        reason: Reason::Default,
        failed_over: false,
        pinned: false,
        stream: false,
        status: 0,
        outcome: "ok",
        ttfb_ms: 0,
        total_ms: 0,
        prompt_tokens: 0,
        completion_tokens: 0,
        tokens_estimated: false,
        saved_usd: None,
    }
}
