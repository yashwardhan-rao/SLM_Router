//! Incremental scan of an OpenAI event stream for token usage, without altering the bytes.

use serde_json::Value;

/// Lines longer than this are not SSE we care about; drop them instead of buffering forever.
const MAX_LINE: usize = 1 << 20;

#[derive(Default)]
pub struct SseScan {
    buf: Vec<u8>,
    usage: Option<(u64, u64)>,
    out_chars: usize,
    /// Drop the usage-only chunk (and its blank line) from what is forwarded.
    hide_usage: bool,
    skip_blank: bool,
}

impl SseScan {
    /// A scan that also filters: `feed` returns the bytes to forward, minus the usage chunk.
    pub fn hiding_usage() -> Self {
        Self {
            hide_usage: true,
            ..Self::default()
        }
    }

    /// Feed raw upstream bytes; chunks may split an event anywhere. When hiding usage, returns
    /// the complete lines to forward; otherwise returns nothing and the caller forwards `bytes`.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let usage_only = self.line(&line);
            let skip = usage_only || (self.skip_blank && line.trim_ascii().is_empty());
            self.skip_blank = usage_only;
            if self.hide_usage && !skip {
                out.extend_from_slice(&line);
            }
        }
        if self.buf.len() > MAX_LINE {
            // Not an event we parse; when filtering, the client still gets every byte of it.
            out.append(&mut self.buf);
            if !self.hide_usage {
                out.clear();
            }
        }
        out
    }

    pub fn filters(&self) -> bool {
        self.hide_usage
    }

    /// Bytes still buffered when the stream ends (an unterminated last line), if filtering.
    /// It is scanned too, so a usage chunk without a final newline is still counted and hidden.
    pub fn tail(&mut self) -> Vec<u8> {
        let last = std::mem::take(&mut self.buf);
        let usage_only = self.line(&last);
        match self.hide_usage && !usage_only {
            true => last,
            false => Vec::new(),
        }
    }

    /// Scans one line; true when it is a chat usage-only chunk (`choices: []` plus `usage`).
    fn line(&mut self, line: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(line) else {
            return false;
        };
        let Some(data) = text.trim().strip_prefix("data:") else {
            return false;
        };
        let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
            return false;
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
        v["choices"].as_array().is_some_and(Vec::is_empty) && v["usage"].is_object()
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
mod tests;
