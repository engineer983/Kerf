//! The assistant's connection to a language model.
//!
//! Two wire protocols are spoken: Anthropic's Messages API, for Claude, and the
//! OpenAI-style chat completions API, which OpenAI, Ollama, LM Studio and many
//! others accept. Each turn is one request: the caller sends the conversation
//! so far and the tools on offer, and gets back the model's text and any tool
//! calls. The caller runs the tools and decides whether to go round again.
//!
//! Conversation history is kept in the protocol's own message format and
//! passed back unchanged, so that nothing a provider attaches to its messages
//! (reasoning blocks, fallback markers) is lost between turns.
//!
//! Nothing here can move a machine: the tools are whatever the caller offers.

mod anthropic;
mod http;
mod images;
mod openai;

pub use images::{generate_image, Generated, ImageEndpoint, ImageProtocol, Shape};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Protocol {
    Anthropic,
    OpenAi,
}

/// Where to send requests, and which model to ask for.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub protocol: Protocol,
    /// For example `https://api.anthropic.com` or `http://localhost:11434/v1`.
    pub base_url: String,
    pub model: String,
}

impl Endpoint {
    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }
}

/// A tool the model may call.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's input object.
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Stop {
    Done,
    /// It is waiting for the results of its tool calls.
    ToolUse,
    /// It ran out of room mid-answer.
    Truncated,
    /// The provider declined the request.
    Refused {
        explanation: String,
    },
}

/// One reply from the model.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    /// The reply in the protocol's own format, to append to the history as is.
    pub message: Value,
    pub stop: Stop,
    /// The model that actually answered, which can differ from the one asked for.
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// Sends the conversation and returns the model's next turn.
pub fn send(
    endpoint: &Endpoint,
    key: Option<&str>,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Result<Turn, String> {
    match endpoint.protocol {
        Protocol::Anthropic => {
            let key = key.ok_or("Add your Anthropic API key in the assistant's settings.")?;
            let body = anthropic::request_body(&endpoint.model, system, messages, tools);
            let reply = http::post(
                &endpoint.url("v1/messages"),
                &anthropic::headers(key, &endpoint.model),
                &body,
            )?;
            anthropic::parse_turn(&reply)
        }
        Protocol::OpenAi => {
            let body = openai::request_body(&endpoint.model, system, messages, tools);
            let reply = http::post(
                &endpoint.url("chat/completions"),
                &openai::headers(key),
                &body,
            )?;
            openai::parse_turn(&reply)
        }
    }
}

/// The models the endpoint offers.
pub fn list_models(endpoint: &Endpoint, key: Option<&str>) -> Result<Vec<ModelInfo>, String> {
    let (url, headers) = match endpoint.protocol {
        Protocol::Anthropic => {
            let key = key.ok_or("Add your Anthropic API key in the assistant's settings.")?;
            (
                endpoint.url("v1/models?limit=100"),
                anthropic::headers(key, ""),
            )
        }
        Protocol::OpenAi => (endpoint.url("models"), openai::headers(key)),
    };
    let reply = http::get(&url, &headers)?;
    let mut models: Vec<ModelInfo> = reply["data"]
        .as_array()
        .ok_or("The provider's list of models was not in the expected form.")?
        .iter()
        .filter_map(|model| {
            let id = model["id"].as_str()?.to_owned();
            let name = model["display_name"].as_str().unwrap_or(&id).to_owned();
            Some(ModelInfo { id, name })
        })
        .collect();
    if endpoint.protocol == Protocol::OpenAi {
        models.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(models)
}
