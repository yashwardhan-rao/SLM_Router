#![allow(dead_code)]
use axum::{
    Json, Router, extract::State, http::HeaderMap, http::StatusCode, response::IntoResponse,
    routing::post,
};
use serde_json::{Value, json};
use slm_router::{app, config::Config};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct Seen {
    pub auth: Option<String>,
    pub body: Value,
}

#[derive(Clone)]
pub enum Mode {
    Ok,
    Status(u16, &'static str),
    Hang,
}

pub struct Fake {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    pub fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    pub fn last(&self) -> Seen {
        self.seen
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("upstream saw no request")
    }
}

type St = (Mode, &'static str, Arc<Mutex<Vec<Seen>>>);

async fn handle(
    State((mode, tag, seen)): State<St>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let auth = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    seen.lock().unwrap().push(Seen { auth, body });
    match mode {
        Mode::Ok => {
            let usage = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
            let msg = json!({"role": "assistant", "content": "hello"});
            let out = json!({"id": "x", "model": tag, "choices": [{"index": 0, "message": msg}], "usage": usage});
            (StatusCode::OK, out.to_string())
        }
        Mode::Status(code, text) => (StatusCode::from_u16(code).unwrap(), text.to_string()),
        Mode::Hang => {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            unreachable!()
        }
    }
}

async fn spawn(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

pub async fn fake(mode: Mode, tag: &'static str) -> Fake {
    let seen = Arc::new(Mutex::new(vec![]));
    let app = Router::new()
        .route("/v1/chat/completions", post(handle))
        .with_state((mode, tag, seen.clone()));
    Fake {
        url: format!("{}/v1", spawn(app).await),
        seen,
    }
}

/// An upstream that is down: it accepts connections and drops them without a response.
/// The port stays held for the whole test run, so no concurrent test's fake can reuse it
/// (freeing it to get a true "connection refused" races with other tests binding port 0).
pub fn dead_url() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            drop(conn);
        }
    });
    url
}

/// Temp directory removed on drop; holds the JSONL log.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
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
