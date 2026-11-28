//! Helpers for `tests/live_cloud.rs`: a real `slm-router` process between local Ollama and a real
//! cloud provider. Nothing here runs in the offline `cargo test`; the live tests are `#[ignore]`.

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

pub const OLLAMA: &str = "http://127.0.0.1:11434/v1";

/// The cloud tier, or print why the test is skipped and return.
macro_rules! cloud_or_skip {
    () => {
        match cloud() {
            Some(c) => c,
            None => return eprintln!("skipped: set GROQ_API_KEY or OPENAI_API_KEY"),
        }
    };
}

/// The cloud tier, from whichever key is set. Prices are the providers' listed rates.
pub struct Cloud {
    pub name: &'static str,
    pub base_url: &'static str,
    pub model: String,
    pub key_env: &'static str,
    pub prices: (f64, f64),
}

/// None when no key is set: the caller skips. `SLMR_LIVE_CLOUD_MODEL` overrides the model.
pub fn cloud() -> Option<Cloud> {
    let set = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    let pick = |m: &str| std::env::var("SLMR_LIVE_CLOUD_MODEL").unwrap_or_else(|_| m.into());
    if set("GROQ_API_KEY") {
        return Some(Cloud {
            name: "groq",
            base_url: "https://api.groq.com/openai/v1",
            model: pick("openai/gpt-oss-120b"),
            key_env: "GROQ_API_KEY",
            prices: (0.15, 0.60),
        });
    }
    set("OPENAI_API_KEY").then(|| Cloud {
        name: "openai",
        base_url: "https://api.openai.com/v1",
        model: pick("gpt-6-luna"),
        key_env: "OPENAI_API_KEY",
        prices: (0.10, 0.50),
    })
}

pub fn local_model() -> String {
    std::env::var("SLMR_LIVE_LOCAL_MODEL").unwrap_or_else(|_| "llama3.2:1b".into())
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

/// A port nothing listens on: bound, then released.
pub fn dead_url() -> String {
    format!("http://127.0.0.1:{}/v1", free_port())
}

/// A running router; killed on drop. Its log and config live in a temp dir.
pub struct Router {
    pub base: String,
    pub log: PathBuf,
    child: Child,
    dir: PathBuf,
}

impl Drop for Router {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub async fn boot(local_url: &str, c: &Cloud) -> Router {
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("slmr-live-{}-{port}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("live.jsonl");
    let toml = format!(
        "version = 1\n[server]\nbind = \"127.0.0.1:{port}\"\n\
         [[tier]]\nname = \"local\"\nbase_url = \"{local_url}\"\nmodel = \"{}\"\nlocal = true\n\
         max_context_tokens = 4096\n\
         [[tier]]\nname = \"{}\"\nbase_url = \"{}\"\nmodel = \"{}\"\napi_key_env = \"{}\"\n\
         supports_tools = true\nhard = true\ninput_per_mtok = {}\noutput_per_mtok = {}\n\
         [log]\npath = {:?}\n",
        local_model(),
        c.name,
        c.base_url,
        c.model,
        c.key_env,
        c.prices.0,
        c.prices.1,
        log.to_str().unwrap()
    );
    let cfg = dir.join("live.toml");
    std::fs::write(&cfg, toml).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_slm-router"))
        .args(["serve", "--config", cfg.to_str().unwrap()])
        // A ROUTER_API_KEY in the caller's shell would make these unauthenticated calls 401.
        .env_remove("ROUTER_API_KEY")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let r = Router {
        base: format!("http://127.0.0.1:{port}"),
        log,
        child,
        dir,
    };
    for _ in 0..100 {
        if reqwest::get(format!("{}/v1/models", r.base)).await.is_ok() {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("router did not start on {}", r.base);
}

/// POST a chat body; returns status and the raw body text.
pub async fn post(r: &Router, body: &Value) -> (u16, String) {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.base))
        .json(body)
        .timeout(std::time::Duration::from_secs(180))
        .send()
        .await
        .unwrap();
    (resp.status().as_u16(), resp.text().await.unwrap())
}

pub fn chat(model: &str, text: &str) -> Value {
    json!({"model": model, "max_tokens": 512,
           "messages": [{"role": "user", "content": text}]})
}

/// The log's last line, polled because streamed requests log after the body is sent.
pub async fn last_log(r: &Router, n: usize) -> Value {
    for _ in 0..100 {
        let text = std::fs::read_to_string(&r.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() >= n {
            return serde_json::from_str(lines[n - 1]).unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("log never reached {n} lines");
}

/// Every `data:` payload of an SSE body, asserting the stream is intact: each event is JSON and
/// it ends with `[DONE]`.
pub fn sse_events(body: &str) -> Vec<Value> {
    let data: Vec<&str> = body
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .collect();
    assert_eq!(
        data.last(),
        Some(&"[DONE]"),
        "stream not terminated:\n{body}"
    );
    data[..data.len() - 1]
        .iter()
        .map(|d| serde_json::from_str(d).unwrap_or_else(|e| panic!("bad event {d:?}: {e}")))
        .collect()
}

pub fn served_by(reply_model: &str, want: &str) -> bool {
    // OpenAI answers with a dated snapshot of the requested alias, with a date suffix.
    reply_model.starts_with(want)
}

pub fn weather_tool() -> Value {
    json!([{"type": "function", "function": {
        "name": "get_weather",
        "description": "Current weather for a city.",
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}},
                       "required": ["city"]}}}])
}
