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

pub use http::{could_not_reach, describe_failure};
#[cfg(feature = "http")]
pub use images::generate_image;
pub use images::{image_request, read_image, Generated, ImageEndpoint, ImageProtocol, Shape};

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

/// A request to a provider: a POST of `body` as JSON, or a GET without one.
/// The reply is JSON too. This crate makes the request itself where it has an
/// HTTP client; in a browser the page makes it and hands back the reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

/// Sends the conversation and returns the model's next turn.
#[cfg(feature = "http")]
pub fn send(
    endpoint: &Endpoint,
    key: Option<&str>,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Result<Turn, String> {
    let reply = http::send(&turn_request(endpoint, key, system, messages, tools)?)?;
    read_turn(endpoint, &reply, tools, messages.len())
}

/// The request that sends the conversation to the model.
pub fn turn_request(
    endpoint: &Endpoint,
    key: Option<&str>,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Result<Request, String> {
    Ok(match endpoint.protocol {
        Protocol::Anthropic => {
            let key = key.ok_or("Add your Anthropic API key in the assistant's settings.")?;
            Request {
                url: endpoint.url("v1/messages"),
                headers: anthropic::headers(key, &endpoint.model),
                body: Some(anthropic::request_body(
                    &endpoint.model,
                    system,
                    messages,
                    tools,
                )),
            }
        }
        Protocol::OpenAi => Request {
            url: endpoint.url("chat/completions"),
            headers: openai::headers(key),
            body: Some(openai::request_body(
                &endpoint.model,
                system,
                messages,
                tools,
            )),
        },
    })
}

/// Reads the model's next turn from the reply to [`turn_request`]. `sent` is
/// how many messages the conversation had when it was sent.
pub fn read_turn(
    endpoint: &Endpoint,
    reply: &Value,
    tools: &[ToolSpec],
    sent: usize,
) -> Result<Turn, String> {
    match endpoint.protocol {
        Protocol::Anthropic => anthropic::parse_turn(reply),
        Protocol::OpenAi => {
            let mut turn = openai::parse_turn(reply)?;
            openai::recover_written_calls(&mut turn, tools, sent);
            Ok(turn)
        }
    }
}

/// The models the endpoint offers.
#[cfg(feature = "http")]
pub fn list_models(endpoint: &Endpoint, key: Option<&str>) -> Result<Vec<ModelInfo>, String> {
    let reply = http::send(&models_request(endpoint, key)?)?;
    read_models(endpoint, &reply)
}

/// The request that asks which models the endpoint offers.
pub fn models_request(endpoint: &Endpoint, key: Option<&str>) -> Result<Request, String> {
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
    Ok(Request {
        url,
        headers,
        body: None,
    })
}

/// Reads the list of models from the reply to [`models_request`].
pub fn read_models(endpoint: &Endpoint, reply: &Value) -> Result<Vec<ModelInfo>, String> {
    let mut models = parse_models(reply)?;
    if endpoint.protocol == Protocol::OpenAi {
        models.sort_by(|a, b| a.id.cmp(&b.id));
    }
    Ok(models)
}

fn parse_models(reply: &Value) -> Result<Vec<ModelInfo>, String> {
    Ok(reply["data"]
        .as_array()
        .ok_or("The provider's list of models was not in the expected form.")?
        .iter()
        .filter_map(|model| {
            let id = model["id"].as_str()?.to_owned();
            // Where a provider says what each model accepts, as OpenRouter
            // does, those that cannot call tools are left out: the assistant
            // could talk through them but not change the project.
            if let Some(accepted) = model["supported_parameters"].as_array() {
                if !accepted.iter().any(|p| p == "tools") {
                    return None;
                }
            }
            let name = model["display_name"]
                .as_str()
                .or(model["name"].as_str())
                .unwrap_or(&id)
                .to_owned();
            Some(ModelInfo { id, name })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_lists_are_read_from_each_providers_form() {
        let anthropic =
            json!({"data": [{"id": "claude-opus-5-5", "display_name": "Claude Opus 5.5"}]});
        assert_eq!(
            parse_models(&anthropic).unwrap(),
            [ModelInfo {
                id: "claude-opus-5-5".into(),
                name: "Claude Opus 5.5".into()
            }]
        );
        let ollama = json!({"data": [{"id": "qwen2.5-coder:7b", "object": "model"}]});
        assert_eq!(parse_models(&ollama).unwrap()[0].name, "qwen2.5-coder:7b");
        assert!(parse_models(&json!({"error": "no"})).is_err());
    }

    #[test]
    fn models_that_cannot_use_tools_are_left_out_where_that_is_known() {
        let openrouter = json!({"data": [
            {"id": "anthropic/claude-sonnet-5.5", "name": "Anthropic: Claude Sonnet 5.5", "supported_parameters": ["tools", "temperature"]},
            {"id": "some/image-model", "name": "Image only", "supported_parameters": ["temperature"]}
        ]});
        let models = parse_models(&openrouter).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "Anthropic: Claude Sonnet 5.5");
    }
}
