#![allow(dead_code)]
mod fake;
mod stream;
pub use fake::*;
#[allow(unused_imports)] // only the streaming test binaries use these helpers
pub use stream::*;

use serde_json::{Value, json};
use slm_router::{app, config::Config};
use std::path::PathBuf;

/// Temp directory removed on drop; holds the JSONL log.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        // The clock is too coarse to be unique across parallel tests; a counter is.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("slmr-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    pub fn log(&self) -> PathBuf {
        self.0.join("log.jsonl")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn config(local: &str, cloud: &str, log: &std::path::Path) -> Config {
    let toml = format!(
        "[local]\nbase_url = \"{local}\"\nmodel = \"llama-test\"\nfirst_byte_timeout_ms = 300\n\
         [cloud]\nbase_url = \"{cloud}\"\nmodel = \"cloud-test\"\napi_key_env = \"X\"\n\
         [pricing]\ninput_per_mtok = 3.0\noutput_per_mtok = 15.0\n\
         [log]\npath = {:?}\n",
        log.to_str().unwrap()
    );
    let mut c = Config::from_toml(&toml).unwrap();
    c.cloud.api_key = Some("sk-cloud".into());
    c
}

pub async fn serve(cfg: Config) -> String {
    spawn(app::router(cfg)).await
}

pub fn log_lines(path: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

pub fn user(text: &str) -> Value {
    json!({"model": "gpt-4", "messages": [{"role": "user", "content": text}]})
}

pub async fn post_chat(base: &str, body: &Value, headers: &[(&str, &str)]) -> (u16, Value) {
    let mut r = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .json(body);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let resp = r.send().await.unwrap();
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

/// Router wired to two live fakes, with its log in a temp dir.
pub struct Rig {
    pub base: String,
    pub local: Fake,
    pub cloud: Fake,
    pub dir: TempDir,
}

pub async fn rig(local_mode: Mode, cloud_mode: Mode, tweak: impl FnOnce(&mut Config)) -> Rig {
    let local = fake(local_mode, "local").await;
    let cloud = fake(cloud_mode, "cloud").await;
    let dir = TempDir::new();
    let mut cfg = config(&local.url, &cloud.url, &dir.log());
    tweak(&mut cfg);
    Rig {
        base: serve(cfg).await,
        local,
        cloud,
        dir,
    }
}

/// Poll until `n` log lines exist: streaming requests log after the body has been delivered.
pub async fn log_lines_wait(path: &std::path::Path, n: usize) -> Vec<Value> {
    for _ in 0..100 {
        let lines = log_lines(path);
        if lines.len() >= n {
            return lines;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("log never reached {n} lines");
}

pub async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("timed out waiting for: {what}");
}
