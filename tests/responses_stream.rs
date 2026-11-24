//! `POST /v1/responses` with `stream: true`: byte-exact relay, usage, mid-stream failure.
mod common;
use common::*;
use serde_json::{Value, json};

const CREATED: &str = r#"{"type":"response.created","sequence_number":0,"response":{"id":"resp_1","status":"in_progress","usage":null}}"#;
const DELTA: &str = r#"{"type":"response.output_text.delta","sequence_number":1,"delta":"Hello.","item_id":"msg_1","output_index":0,"content_index":0}"#;
const COMPLETED: &str = r#"{"type":"response.completed","sequence_number":2,"response":{"id":"resp_1","status":"completed","usage":{"input_tokens":27,"output_tokens":3,"total_tokens":30}}}"#;

fn stream_resp() -> Value {
    json!({"model": "gpt-4.1", "input": "say hi", "stream": true})
}

async fn fetch_resp(base: &str, body: &Value) -> (u16, String) {
    let resp = reqwest::Client::new()
        .post(format!("{base}/v1/responses"))
        .json(body)
        .send()
        .await
        .unwrap();
    (resp.status().as_u16(), resp.text().await.unwrap())
}

#[tokio::test]
async fn stream_is_relayed_byte_for_byte_and_logs_completed_usage() {
    let r = rig(
        Mode::SseSplit(&[CREATED, DELTA, COMPLETED]),
        Mode::Ok,
        |_| {},
    )
    .await;
    let (status, body) = fetch_resp(&r.base, &stream_resp()).await;
    let want: String = [CREATED, DELTA, COMPLETED]
        .iter()
        .map(|p| format!("data: {p}\n\n"))
        .collect();
    assert_eq!((status, body), (200, want));
    assert_eq!(r.local.last().path, "/v1/responses");
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (
            &l["prompt_tokens"],
            &l["completion_tokens"],
            &l["tokens_estimated"],
            &l["stream"]
        ),
        (&json!(27), &json!(3), &json!(false), &json!(true))
    );
}

#[tokio::test]
async fn a_trailing_done_is_harmless() {
    let r = rig(Mode::Sse(&[CREATED, DELTA, COMPLETED]), Mode::Ok, |_| {}).await;
    let (status, body) = fetch_resp(&r.base, &stream_resp()).await;
    assert_eq!(status, 200);
    assert!(body.ends_with("data: [DONE]\n\n"));
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(
        (&l["prompt_tokens"], &l["completion_tokens"]),
        (&json!(27), &json!(3))
    );
}

#[tokio::test]
async fn mid_stream_death_ends_with_a_native_error_event() {
    let r = rig(Mode::SseDies, Mode::Ok, |_| {}).await;
    let (status, body) = fetch_resp(&r.base, &stream_resp()).await;
    assert_eq!(status, 200);
    let last = body.trim_end().rsplit("\n\n").next().unwrap();
    assert!(last.starts_with("event: error\ndata: "), "{body}");
    let data: Value = serde_json::from_str(last.split_once("data: ").unwrap().1).unwrap();
    assert_eq!(
        (&data["type"], &data["code"]),
        (&json!("error"), &json!("stream_interrupted"))
    );
    let l = &log_lines_wait(&r.dir.log(), 1).await[0];
    assert_eq!(l["outcome"], "stream_error");
}
