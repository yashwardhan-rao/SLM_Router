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
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn has_image(content: &Value) -> bool {
    content
        .as_array()
        .is_some_and(|ps| ps.iter().any(|p| p["type"] == "image_url"))
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
    // Rough estimate (chars / 4): good enough for threshold routing, no tokenizer dependency.
    let chars: usize = r
        .messages
        .iter()
        .map(|m| text_of(&m.content).chars().count())
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
        est_prompt_tokens: chars / 4,
        last_user_text: last_user_text.unwrap_or_default(),
    }
}
