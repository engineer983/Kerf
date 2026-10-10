//! Anthropic's Messages API.

use serde_json::{json, Value};

use crate::{Stop, ToolCall, ToolSpec, Turn};

const API_VERSION: &str = "2023-06-01";
/// Room for a long answer without risking the HTTP timeout of a single,
/// unstreamed request.
const MAX_TOKENS: u32 = 16_000;
/// Lets the API re-run a request on another model when a safety classifier
/// declines it, instead of returning the refusal.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Whether the model takes `fallbacks: "default"`. Sending it to a model
/// that does not is an error, so it is limited to the families documented
/// to accept it.
fn supports_default_fallback(model: &str) -> bool {
    ["claude-opus-5", "claude-fable-5-1", "claude-sonnet-5-5"]
        .iter()
        .any(|family| model.starts_with(family))
}

pub(crate) fn headers(key: &str, model: &str) -> Vec<(String, String)> {
    let mut headers = vec![
        ("x-api-key".to_owned(), key.to_owned()),
        ("anthropic-version".to_owned(), API_VERSION.to_owned()),
    ];
    if supports_default_fallback(model) {
        headers.push(("anthropic-beta".to_owned(), FALLBACK_BETA.to_owned()));
    }
    headers
}

pub(crate) fn request_body(
    model: &str,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        // The tools and the system prompt are the same on every request, so
        // they are marked for caching: later turns reuse them at a fraction
        // of the cost. Anything that varies belongs in `messages`.
        "system": [{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
        "messages": messages,
    });
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(
                |t| json!({"name": t.name, "description": t.description, "input_schema": t.schema}),
            )
            .collect();
    }
    if supports_default_fallback(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

pub(crate) fn parse_turn(reply: &Value) -> Result<Turn, String> {
    let content = reply["content"]
        .as_array()
        .ok_or("The reply had no content.")?;
    // Why it stopped is read before the content: a refusal can arrive with
    // partial or no content.
    let stop = match reply["stop_reason"].as_str() {
        Some("tool_use") => Stop::ToolUse,
        Some("max_tokens") => Stop::Truncated,
        Some("refusal") => Stop::Refused {
            explanation: reply["stop_details"]["explanation"]
                .as_str()
                .unwrap_or("The model declined this request.")
                .to_owned(),
        },
        _ => Stop::Done,
    };

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    for block in content {
        match block["type"].as_str() {
            Some("text") => text.push_str(block["text"].as_str().unwrap_or("")),
            Some("tool_use") => tool_calls.push(ToolCall {
                id: block["id"].as_str().unwrap_or("").to_owned(),
                name: block["name"].as_str().unwrap_or("").to_owned(),
                input: block["input"].clone(),
            }),
            // Reasoning and fallback blocks are not shown, but stay in
            // `message` so they go back to the API unchanged.
            _ => {}
        }
    }

    Ok(Turn {
        text,
        tool_calls,
        message: json!({"role": "assistant", "content": content}),
        stop,
        model: reply["model"].as_str().unwrap_or("").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolSpec {
        ToolSpec {
            name: "set_material".into(),
            description: "Choose the material.".into(),
            schema: json!({"type": "object", "properties": {"material": {"type": "string"}}}),
        }
    }

    #[test]
    fn request_carries_cached_system_tools_and_fallback() {
        let messages = [json!({"role": "user", "content": "Use walnut."})];
        let body = request_body(
            "claude-opus-5-5",
            "You help with carving.",
            &messages,
            &[tool()],
        );
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["system"][0]["text"], "You help with carving.");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][0]["name"], "set_material");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"], json!(messages));
        assert_eq!(body["fallbacks"], "default");
        // Thinking and effort are left to the model's defaults.
        assert!(body.get("thinking").is_none() && body.get("output_config").is_none());

        let headers = headers("sk-test", "claude-opus-5-5");
        assert!(headers.contains(&("x-api-key".into(), "sk-test".into())));
        assert!(headers.contains(&("anthropic-version".into(), "2023-06-01".into())));
        assert!(headers.contains(&("anthropic-beta".into(), FALLBACK_BETA.into())));
    }

    #[test]
    fn models_without_default_fallback_are_not_sent_it() {
        let body = request_body("claude-haiku-5-5", "s", &[], &[]);
        assert!(body.get("fallbacks").is_none());
        assert!(body.get("tools").is_none());
        assert!(!headers("k", "claude-haiku-5-5")
            .iter()
            .any(|(name, _)| name == "anthropic-beta"));
    }

    #[test]
    fn a_tool_call_is_read_and_the_message_kept_whole() {
        let reply = json!({
            "model": "claude-opus-5-5",
            "stop_reason": "tool_use",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "abc"},
                {"type": "text", "text": "I'll switch the material."},
                {"type": "tool_use", "id": "toolu_1", "name": "set_material", "input": {"material": "walnut"}}
            ]
        });
        let turn = parse_turn(&reply).unwrap();
        assert_eq!(turn.stop, Stop::ToolUse);
        assert_eq!(turn.text, "I'll switch the material.");
        assert_eq!(
            turn.tool_calls,
            [ToolCall {
                id: "toolu_1".into(),
                name: "set_material".into(),
                input: json!({"material": "walnut"})
            }]
        );
        // The thinking block must go back to the API exactly as it came.
        assert_eq!(turn.message["role"], "assistant");
        assert_eq!(turn.message["content"], reply["content"]);
    }

    #[test]
    fn refusals_and_truncation_are_reported() {
        let refused = json!({
            "model": "claude-opus-5-5",
            "stop_reason": "refusal",
            "stop_details": {"type": "refusal", "category": null, "explanation": "Not something I can help with."},
            "content": []
        });
        assert_eq!(
            parse_turn(&refused).unwrap().stop,
            Stop::Refused {
                explanation: "Not something I can help with.".into()
            }
        );
        let cut = json!({"model": "m", "stop_reason": "max_tokens", "content": [{"type": "text", "text": "Half an ans"}]});
        assert_eq!(parse_turn(&cut).unwrap().stop, Stop::Truncated);
        assert!(parse_turn(&json!({"type": "error"})).is_err());
    }
}
