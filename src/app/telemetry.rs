//! Token accounting and the counterfactual cloud saving for the request log.

use crate::logging::{Record, now_ms};
use crate::routing::{Policy, Reason, Target};
use crate::{config::Pricing, schema::text_of, session::SessionSource};
use serde_json::Value;

/// (prompt, completion, estimated). Uses the upstream `usage` when present, else a chars/4 estimate.
pub fn usage(body: &[u8], est_prompt: usize) -> (u64, u64, bool) {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let u = &v["usage"];
    // Chat names them prompt/completion, `/v1/responses` names them input/output.
    let p = u["prompt_tokens"].as_u64().or(u["input_tokens"].as_u64());
    let c = u["completion_tokens"]
        .as_u64()
        .or(u["output_tokens"].as_u64());
    match (p, c) {
        (Some(p), Some(c)) => (p, c, false),
        _ => {
            let chars = match v["choices"][0]["message"]["content"].as_str() {
                Some(t) => t.chars().count(),
                None => v["output"].as_array().map_or(0, |items| {
                    items
                        .iter()
                        .map(|i| text_of(&i["content"]).chars().count())
                        .sum()
                }),
            };
            (est_prompt as u64, (chars / 4) as u64, true)
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

#[cfg(test)]
mod tests {
    use super::usage;

    #[test]
    fn responses_body_usage_is_read() {
        let body = br#"{"object":"response","output":[],"usage":{"input_tokens":27,"output_tokens":10,"total_tokens":37}}"#;
        assert_eq!(usage(body, 99), (27, 10, false));
    }

    #[test]
    fn responses_body_without_usage_estimates_from_output_text() {
        let body = br#"{"object":"response","output":[
            {"type":"reasoning","summary":[]},
            {"type":"message","content":[{"type":"output_text","text":"12345678"}]},
            {"type":"message","content":[{"type":"output_text","text":"1234"}]}]}"#;
        assert_eq!(usage(body, 99), (99, 3, true));
    }

    #[test]
    fn chat_body_still_works() {
        let body = br#"{"choices":[{"message":{"content":"12345678"}}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        assert_eq!(usage(body, 99), (5, 2, false));
        assert_eq!(
            usage(br#"{"choices":[{"message":{"content":"12345678"}}]}"#, 99),
            (99, 2, true)
        );
    }
}
