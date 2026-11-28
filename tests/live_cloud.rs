//! Live integration against a real cloud provider. Never runs in the offline fast check.
//!
//!   GROQ_API_KEY=... cargo test --test live_cloud -- --ignored --test-threads=1 --nocapture
//!   (or OPENAI_API_KEY for gpt-6-luna; SLMR_LIVE_CLOUD_MODEL / SLMR_LIVE_LOCAL_MODEL override the models)
//!
//! Needs Ollama on 127.0.0.1:11434 with the local model pulled, except for the failover test.
//! With no key set, each test prints why and passes without calling anything.

#[macro_use]
mod live;
use live::*;
use serde_json::{Value, json};

#[tokio::test]
#[ignore = "live: needs GROQ_API_KEY or OPENAI_API_KEY; run with --ignored"]
async fn live_1_auto_request_routes_local() {
    let c = cloud_or_skip!();
    let r = boot(OLLAMA, &c).await;
    let (status, body) = post(
        &r,
        &chat("slm-router/auto", "What is the capital of France?"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        served_by(v["model"].as_str().unwrap(), &local_model()),
        "{v}"
    );
    let l = last_log(&r, 1).await;
    assert_eq!(
        (l["tier"].as_str(), l["reason"].as_str()),
        (Some("local"), Some("default")),
        "{l}"
    );
    assert!(
        l["saved_usd"].as_f64().unwrap() > 0.0,
        "a local answer saves the cloud price: {l}"
    );
}

#[tokio::test]
#[ignore = "live: needs GROQ_API_KEY or OPENAI_API_KEY; run with --ignored"]
async fn live_2_forced_cloud_non_streaming() {
    let c = cloud_or_skip!();
    let r = boot(OLLAMA, &c).await;
    let (status, body) = post(
        &r,
        &chat("slm-router/cloud", "Reply with the single word: pong"),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(served_by(v["model"].as_str().unwrap(), &c.model), "{v}");
    assert!(v["choices"][0]["finish_reason"].is_string(), "{v}");
    let l = last_log(&r, 1).await;
    assert_eq!(
        (l["tier"].as_str(), l["reason"].as_str()),
        (Some(c.name), Some("forced"))
    );
    assert_eq!(l["tokens_estimated"], false, "{l}");
    assert!(l["prompt_tokens"].as_u64() > Some(0) && l["completion_tokens"].as_u64() > Some(0));
    assert_eq!(
        l["saved_usd"], 0.0,
        "the top tier saves nothing, but is priced: {l}"
    );
}

#[tokio::test]
#[ignore = "live: needs GROQ_API_KEY or OPENAI_API_KEY; run with --ignored"]
async fn live_3_streaming_to_cloud_is_intact_and_counted() {
    let c = cloud_or_skip!();
    let r = boot(OLLAMA, &c).await;
    let mut body = chat(
        "slm-router/cloud",
        "Count from 1 to 5, one number per line.",
    );
    body["stream"] = json!(true);
    // 1) The client did not ask for usage: the router asks for it and hides the usage chunk.
    let (status, text) = post(&r, &body).await;
    assert_eq!(status, 200, "{text}");
    let events = sse_events(&text);
    let content: String = events
        .iter()
        .filter_map(|e| e["choices"][0]["delta"]["content"].as_str())
        .collect();
    assert!(!content.is_empty(), "no content in stream:\n{text}");
    assert!(
        !events
            .iter()
            .any(|e| e["choices"] == json!([]) && e["usage"].is_object()),
        "usage chunk leaked to a client that did not ask for it:\n{text}"
    );
    let l = last_log(&r, 1).await;
    assert_eq!(
        (l["stream"].as_bool(), l["outcome"].as_str()),
        (Some(true), Some("ok"))
    );
    assert_eq!(
        l["tokens_estimated"], false,
        "{} sent no usage chunk for stream_options.include_usage; tokens were estimated: {l}",
        c.name
    );
    assert!(l["completion_tokens"].as_u64() > Some(0), "{l}");
    // 2) The client asked for usage itself: it gets the usage chunk, untouched.
    body["stream_options"] = json!({"include_usage": true});
    let (_, text) = post(&r, &body).await;
    let usage = sse_events(&text)
        .into_iter()
        .find(|e| e["usage"].is_object());
    assert!(
        usage.is_some(),
        "client asked for usage but got none:\n{text}"
    );
    assert_eq!(last_log(&r, 2).await["tokens_estimated"], false);
}

#[tokio::test]
#[ignore = "live: needs GROQ_API_KEY or OPENAI_API_KEY; run with --ignored"]
async fn live_4_tool_calls_pass_through_to_cloud() {
    let c = cloud_or_skip!();
    let r = boot(OLLAMA, &c).await;
    let mut body = chat(
        "slm-router/auto",
        "What is the weather in Paris? Use the tool.",
    );
    body["tools"] = weather_tool();
    body["tool_choice"] = json!("auto");
    let (status, text) = post(&r, &body).await;
    assert_eq!(status, 200, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let msg = &v["choices"][0]["message"];
    let call = &msg["tool_calls"][0];
    assert_eq!(call["function"]["name"], "get_weather", "{v}");
    let args: Value =
        serde_json::from_str(call["function"]["arguments"].as_str().unwrap()).unwrap();
    assert!(args["city"].as_str().unwrap().contains("Paris"), "{args}");
    let l = last_log(&r, 1).await;
    assert_eq!(
        (l["tier"].as_str(), l["reason"].as_str()),
        (Some(c.name), Some("capability"))
    );
    // Second turn: the assistant's tool call and our tool result go back through the router.
    body["messages"].as_array_mut().unwrap().extend([
        msg.clone(),
        json!({"role": "tool", "tool_call_id": call["id"], "content": "{\"temp_c\": 18, \"sky\": \"clear\"}"}),
    ]);
    let (status, text) = post(&r, &body).await;
    assert_eq!(status, 200, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(
        v["choices"][0]["message"]["content"]
            .as_str()
            .is_some_and(|s| s.contains("18")),
        "{v}"
    );
}

#[tokio::test]
#[ignore = "live: needs GROQ_API_KEY or OPENAI_API_KEY; run with --ignored"]
async fn live_5_dead_local_fails_over_to_cloud_before_the_first_byte() {
    let c = cloud_or_skip!();
    let r = boot(&dead_url(), &c).await;
    let (status, text) = post(
        &r,
        &chat("slm-router/auto", "What is the capital of France?"),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(served_by(v["model"].as_str().unwrap(), &c.model), "{v}");
    let l = last_log(&r, 1).await;
    assert_eq!(
        (l["tier"].as_str(), l["reason"].as_str()),
        (Some(c.name), Some("local_unavailable"))
    );
    assert_eq!(l["failed_over"], true, "{l}");
    // Streamed: every chunk comes from the cloud model, so nothing was spliced from two models.
    let mut body = chat("slm-router/auto", "Say hello in three languages.");
    body["stream"] = json!(true);
    let (status, text) = post(&r, &body).await;
    assert_eq!(status, 200, "{text}");
    let events = sse_events(&text);
    assert!(!events.is_empty());
    for e in &events {
        assert!(
            served_by(e["model"].as_str().unwrap_or(""), &c.model),
            "{e}"
        );
    }
    let l = last_log(&r, 2).await;
    assert_eq!(
        (l["failed_over"].as_bool(), l["outcome"].as_str()),
        (Some(true), Some("ok"))
    );
    assert_eq!(l["tokens_estimated"], false, "{l}");
}
