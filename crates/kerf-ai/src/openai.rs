//! The OpenAI-style chat completions API, as spoken by OpenAI, Ollama, LM
//! Studio and other compatible servers.

use serde_json::{json, Value};

use crate::{Stop, ToolCall, ToolSpec, Turn};

/// Local servers need no key; hosted ones take it as a bearer token.
pub(crate) fn headers(key: Option<&str>) -> Vec<(String, String)> {
    key.filter(|k| !k.is_empty())
        .map(|k| vec![("Authorization".to_owned(), format!("Bearer {k}"))])
        .unwrap_or_default()
}

pub(crate) fn request_body(
    model: &str,
    system: &str,
    messages: &[Value],
    tools: &[ToolSpec],
) -> Value {
    let mut all = vec![json!({"role": "system", "content": system})];
    all.extend_from_slice(messages);
    let mut body = json!({"model": model, "messages": all});
    if !tools.is_empty() {
        body["tools"] = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {"name": t.name, "description": t.description, "parameters": t.schema}
                })
            })
            .collect();
    }
    body
}

pub(crate) fn parse_turn(reply: &Value) -> Result<Turn, String> {
    let choice = &reply["choices"][0];
    let message = choice["message"]
        .as_object()
        .ok_or("The reply had no message.")?;

    let tool_calls: Vec<ToolCall> = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    // Arguments arrive as a JSON string; some servers send an object.
                    let arguments = &call["function"]["arguments"];
                    let input = match arguments.as_str() {
                        Some(text) => serde_json::from_str(text)
                            .unwrap_or_else(|_| json!({"_unreadable": text})),
                        None => arguments.clone(),
                    };
                    ToolCall {
                        id: call["id"].as_str().unwrap_or("").to_owned(),
                        name: call["function"]["name"].as_str().unwrap_or("").to_owned(),
                        input,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let stop = match choice["finish_reason"].as_str() {
        _ if !tool_calls.is_empty() => Stop::ToolUse,
        Some("length") => Stop::Truncated,
        Some("content_filter") => Stop::Refused {
            explanation: "The provider's content filter stopped this reply.".to_owned(),
        },
        _ => Stop::Done,
    };

    let mut kept = Value::Object(message.clone());
    kept["role"] = json!("assistant");
    Ok(Turn {
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        tool_calls,
        message: kept,
        stop,
        model: reply["model"].as_str().unwrap_or("").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_puts_the_system_prompt_first_and_wraps_tools() {
        let tool = ToolSpec {
            name: "set_material".into(),
            description: "Choose the material.".into(),
            schema: json!({"type": "object"}),
        };
        let messages = [json!({"role": "user", "content": "Use walnut."})];
        let body = request_body("llama3.1", "You help with carving.", &messages, &[tool]);
        assert_eq!(
            body["messages"][0],
            json!({"role": "system", "content": "You help with carving."})
        );
        assert_eq!(body["messages"][1], messages[0]);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert!(request_body("m", "s", &[], &[]).get("tools").is_none());

        assert!(headers(None).is_empty() && headers(Some("")).is_empty());
        assert_eq!(
            headers(Some("sk-1")),
            [("Authorization".to_owned(), "Bearer sk-1".to_owned())]
        );
    }

    #[test]
    fn tool_calls_are_read_from_string_or_object_arguments() {
        let reply = json!({
            "model": "llama3.1",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "set_material", "arguments": "{\"material\":\"walnut\"}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "get_project", "arguments": {}}},
                        {"id": "call_3", "type": "function", "function": {"name": "broken", "arguments": "{not json"}}
                    ]
                }
            }]
        });
        let turn = parse_turn(&reply).unwrap();
        assert_eq!(turn.stop, Stop::ToolUse);
        assert_eq!(turn.text, "");
        assert_eq!(turn.tool_calls[0].input, json!({"material": "walnut"}));
        assert_eq!(turn.tool_calls[1].input, json!({}));
        assert_eq!(
            turn.tool_calls[2].input,
            json!({"_unreadable": "{not json"})
        );
        // The message goes back with its tool calls intact.
        assert_eq!(turn.message["tool_calls"][0]["id"], "call_1");
    }

    #[test]
    fn plain_answers_and_cut_off_ones() {
        let done = json!({"model": "m", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "Try 0.2 mm."}}]});
        let turn = parse_turn(&done).unwrap();
        assert_eq!((turn.stop, turn.text.as_str()), (Stop::Done, "Try 0.2 mm."));
        let cut = json!({"model": "m", "choices": [{"finish_reason": "length", "message": {"content": "Half"}}]});
        assert_eq!(parse_turn(&cut).unwrap().stop, Stop::Truncated);
        assert!(parse_turn(&json!({"error": "nope"})).is_err());
    }
}
