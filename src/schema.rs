//! Typed view of an OpenAI chat request. Unknown fields ride along in `extra` so the proxy stays drop-in.

use crate::routing::{Features, Forced};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Text of a message: a plain string, or the `text` parts of a content array.
pub fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| {
                matches!(
                    p["type"].as_str(),
                    Some("text" | "input_text" | "output_text")
                )
            })
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn has_image(content: &Value) -> bool {
    content.as_array().is_some_and(|ps| {
        ps.iter().any(|p| {
            matches!(
                p["type"].as_str(),
                Some("image_url" | "input_image" | "input_file")
            )
        })
    })
}

fn msg(role: &str, content: Value) -> Message {
    let extra = Map::new();
    Message {
        role: role.into(),
        content,
        extra,
    }
}

/// One `/v1/responses` input item as a chat message, for routing features only.
fn item_message(item: &Value) -> Option<Message> {
    if let Some(role) = item["role"].as_str() {
        let role = if role == "developer" { "system" } else { role };
        return Some(msg(role, item["content"].clone()));
    }
    match item["type"].as_str()? {
        "input_text" | "input_image" | "input_file" => {
            Some(msg("user", Value::Array(vec![item.clone()])))
        }
        "function_call" => Some(msg("assistant", item["arguments"].clone())),
        "function_call_output" => Some(msg("tool", item["output"].clone())),
        // reasoning, item_reference and unknown items carry no prompt text we can see.
        _ => None,
    }
}

/// A chat-shaped view of a `/v1/responses` body so `features` and `session_id` apply unchanged.
/// The original body is what gets forwarded.
pub fn responses_view(v: &Value) -> Result<ChatRequest, String> {
    let model = v["model"].as_str().ok_or("`model` must be a string")?;
    let mut messages = Vec::new();
    if let Some(i) = v["instructions"].as_str().filter(|s| !s.is_empty()) {
        messages.push(msg("system", i.into()));
    }
    match &v["input"] {
        Value::String(s) => messages.push(msg("user", s.as_str().into())),
        Value::Array(items) => messages.extend(items.iter().filter_map(item_message)),
        _ => return Err("`input` must be a string or an array of items".into()),
    }
    let some = |x: &Value| Some(x.clone()).filter(|x| !x.is_null());
    Ok(ChatRequest {
        model: model.into(),
        messages,
        stream: v["stream"].as_bool(),
        tools: some(&v["tools"]),
        tool_choice: some(&v["tool_choice"]),
        response_format: some(&v["text"]["format"]),
        extra: Map::new(),
    })
}

/// True when the request depends on state only the cloud keeps; Ollama ignores these silently.
/// SDKs send `null` / `false` defaults on stateless calls, so only set values count.
pub fn stateful(v: &Value) -> bool {
    let prev = v["previous_response_id"]
        .as_str()
        .is_some_and(|s| !s.is_empty());
    let conv = match &v["conversation"] {
        Value::String(s) => !s.is_empty(),
        c => c.is_object(),
    };
    let item_ref = v["input"]
        .as_array()
        .is_some_and(|items| items.iter().any(|i| i["type"] == "item_reference"));
    prev || conv || v["store"] == true || v["background"] == true || item_ref
}

fn needs_capabilities(r: &ChatRequest) -> bool {
    let tools = r
        .tools
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|t| !t.is_empty());
    let choice = r
        .tool_choice
        .as_ref()
        .is_some_and(|c| !c.is_null() && c != "none");
    let schema = r
        .response_format
        .as_ref()
        .is_some_and(|f| f["type"] == "json_schema");
    tools || choice || schema || r.messages.iter().any(|m| has_image(&m.content))
}

pub fn features(r: &ChatRequest) -> Features {
    let forced = match r.model.as_str() {
        "slm-router/local" => Forced::Local,
        "slm-router/cloud" => Forced::Cloud,
        _ => Forced::Auto,
    };
    // +4 per message approximates the chat template's role and separator tokens.
    let est: usize = r
        .messages
        .iter()
        .map(|m| crate::tokens::estimate(&text_of(&m.content)) + 4)
        .sum();
    let last_user_text = r
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| text_of(&m.content));
    Features {
        forced,
        needs_capabilities: needs_capabilities(r),
        est_prompt_tokens: est,
        last_user_text: last_user_text.unwrap_or_default(),
        // Chat has no server-side state; `/v1/responses` sets this from `stateful`.
        requires_cloud: false,
    }
}
