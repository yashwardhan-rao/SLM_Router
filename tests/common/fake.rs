//! In-process fake OpenAI-compatible upstreams on ephemeral ports.
use axum::{
    Json, Router, body::Body, extract::DefaultBodyLimit, extract::State, http::HeaderMap,
    http::StatusCode, response::IntoResponse, routing::post,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering};
use std::time::Duration;
use tokio_stream::wrappers::ReceiverStream;

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
    /// Event-stream of these `data:` payloads followed by `[DONE]`.
    Sse(&'static [&'static str]),
    /// Like `Sse`, but each event is cut in two TCP chunks (40ms apart) to test line reassembly.
    SseSplit(&'static [&'static str]),
    /// One event, then the connection dies mid-stream.
    SseDies,
    /// An endless stream; `Fake::dropped` flips once the consumer goes away.
    SseForever,
}

pub struct Fake {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub dropped: Arc<AtomicBool>,
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
    pub fn was_dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct Shared {
    mode: Mode,
    tag: &'static str,
    seen: Arc<Mutex<Vec<Seen>>>,
    dropped: Arc<AtomicBool>,
}

type Chunk = Result<String, std::io::Error>;

fn sse_response(rx: tokio::sync::mpsc::Receiver<Chunk>) -> axum::response::Response {
    let body = Body::from_stream(ReceiverStream::new(rx));
    ([("content-type", "text/event-stream")], body).into_response()
}

fn event(payload: &str) -> String {
    format!("data: {payload}\n\n")
}

async fn handle(
    State(s): State<Shared>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let auth = h
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    s.seen.lock().unwrap().push(Seen { auth, body });
    match s.mode {
        Mode::Ok => {
            let usage = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
            let msg = json!({"role": "assistant", "content": "hello"});
            let out = json!({"id": "x", "model": s.tag, "choices": [{"index": 0, "message": msg}], "usage": usage});
            (StatusCode::OK, out.to_string()).into_response()
        }
        Mode::Status(code, text) => (StatusCode::from_u16(code).unwrap(), text).into_response(),
        Mode::Hang => {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            unreachable!()
        }
        Mode::Sse(payloads) => {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            tokio::spawn(async move {
                for p in payloads {
                    let _ = tx.send(Ok(event(p))).await;
                }
                let _ = tx.send(Ok(event("[DONE]"))).await;
            });
            sse_response(rx)
        }
        Mode::SseSplit(payloads) => {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            tokio::spawn(async move {
                for p in payloads {
                    let e = event(p);
                    let (a, b) = e.split_at(e.len() / 2);
                    let _ = tx.send(Ok(a.to_string())).await;
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    let _ = tx.send(Ok(b.to_string())).await;
                }
            });
            sse_response(rx)
        }
        Mode::SseDies => {
            let (tx, rx) = tokio::sync::mpsc::channel(16);
            tokio::spawn(async move {
                let first = r#"{"choices":[{"delta":{"content":"partial text"}}]}"#;
                let _ = tx.send(Ok(event(first))).await;
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = tx
                    .send(Err(std::io::Error::other("upstream crashed")))
                    .await;
            });
            sse_response(rx)
        }
        Mode::SseForever => {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let dropped = s.dropped.clone();
            tokio::spawn(async move {
                let chunk = r#"{"choices":[{"delta":{"content":"x"}}]}"#;
                while tx.send(Ok(event(chunk))).await.is_ok() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                dropped.store(true, Ordering::SeqCst);
            });
            sse_response(rx)
        }
    }
}

pub async fn spawn(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

pub async fn fake(mode: Mode, tag: &'static str) -> Fake {
    let seen = Arc::new(Mutex::new(vec![]));
    let dropped = Arc::new(AtomicBool::new(false));
    let shared = Shared {
        mode,
        tag,
        seen: seen.clone(),
        dropped: dropped.clone(),
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(handle))
        .layer(DefaultBodyLimit::disable())
        .with_state(shared);
    Fake {
        url: format!("{}/v1", spawn(app).await),
        seen,
        dropped,
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
