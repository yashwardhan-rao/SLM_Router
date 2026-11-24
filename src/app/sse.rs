//! Incremental scan of an OpenAI event stream for token usage, without altering the bytes.

use serde_json::Value;

/// Lines longer than this are not SSE we care about; drop them instead of buffering forever.
const MAX_LINE: usize = 1 << 20;

#[derive(Default)]
pub struct SseScan {
    buf: Vec<u8>,
    usage: Option<(u64, u64)>,
    out_chars: usize,
}

impl SseScan {
    /// Feed raw upstream bytes; chunks may split an event anywhere.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            self.line(&line);
        }
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
        }
    }

    fn line(&mut self, line: &[u8]) {
        let Ok(text) = std::str::from_utf8(line) else {
            return;
        };
        let Some(data) = text.trim().strip_prefix("data:") else {
            return;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            return;
        };
        let u = &v["usage"];
        if let (Some(p), Some(c)) = (u["prompt_tokens"].as_u64(), u["completion_tokens"].as_u64()) {
            self.usage = Some((p, c));
        }
        // `/v1/responses`: terminal events (completed, incomplete, failed) carry `response.usage`;
        // `created` / `in_progress` carry null and are skipped by the `as_u64` checks.
        let r = &v["response"]["usage"];
        if let (Some(p), Some(c)) = (r["input_tokens"].as_u64(), r["output_tokens"].as_u64()) {
            self.usage = Some((p, c));
        }
        let delta = match v["type"] == "response.output_text.delta" {
            true => v["delta"].as_str(),
            false => v["choices"][0]["delta"]["content"].as_str(),
        };
        if let Some(t) = delta {
            self.out_chars += t.chars().count();
        }
    }

    /// (prompt, completion, estimated): the upstream's usage event, else chars / 4.
    pub fn tokens(&self, est_prompt: usize) -> (u64, u64, bool) {
        match self.usage {
            Some((p, c)) => (p, c, false),
            None => (est_prompt as u64, (self.out_chars / 4) as u64, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SseScan;

    fn scan(chunks: &[&str]) -> SseScan {
        let mut s = SseScan::default();
        chunks.iter().for_each(|c| s.feed(c.as_bytes()));
        s
    }

    const CREATED: &str = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"usage\":null}}\n\n";
    const DELTA: &str = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"12345678\"}\n\n";

    fn terminal(kind: &str) -> String {
        format!(
            "event: response.{kind}\ndata: {{\"type\":\"response.{kind}\",\"response\":{{\"usage\":{{\"input_tokens\":27,\"output_tokens\":3,\"total_tokens\":30}}}}}}\n\n"
        )
    }

    #[test]
    fn responses_usage_comes_from_every_terminal_event_split_or_not() {
        for kind in ["completed", "incomplete", "failed"] {
            let t = terminal(kind);
            let (a, b) = t.split_at(t.len() / 2);
            assert_eq!(
                scan(&[CREATED, DELTA, a, b]).tokens(99),
                (27, 3, false),
                "{kind}"
            );
        }
    }

    #[test]
    fn done_sentinel_is_optional() {
        let t = terminal("completed");
        assert_eq!(scan(&[DELTA, &t]).tokens(99), (27, 3, false));
        assert_eq!(
            scan(&[DELTA, &t, "data: [DONE]\n\n"]).tokens(99),
            (27, 3, false)
        );
    }

    #[test]
    fn a_null_usage_after_a_real_one_does_not_clobber_it() {
        assert_eq!(
            scan(&[&terminal("completed"), CREATED]).tokens(99),
            (27, 3, false)
        );
    }

    #[test]
    fn deltas_without_a_terminal_event_are_estimated() {
        assert_eq!(scan(&[CREATED, DELTA, DELTA]).tokens(99), (99, 4, true));
    }

    #[test]
    fn chat_usage_still_parses() {
        let chunks = [
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        ];
        assert_eq!(scan(&chunks).tokens(99), (7, 3, false));
    }
}
