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
        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
            self.out_chars += t.chars().count();
        }
    }

    /// (prompt, completion, estimated): the upstream's usage chunk, else chars / 4.
    pub fn tokens(&self, est_prompt: usize) -> (u64, u64, bool) {
        match self.usage {
            Some((p, c)) => (p, c, false),
            None => (est_prompt as u64, (self.out_chars / 4) as u64, true),
        }
    }
}
