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

/// Small local models often write a tool call out as text, such as
/// `{"name": "find_pictures", "arguments": {"query": "eagle"}}`, instead of
/// sending it as one. Where the text holds such a call to a tool that exists,
/// it is turned into a real call, so the tool runs rather than the person
/// being shown JSON. `seed` keeps the made-up ids distinct between turns.
pub(crate) fn recover_written_calls(turn: &mut Turn, tools: &[ToolSpec], seed: usize) {
    if !turn.tool_calls.is_empty() || turn.stop != Stop::Done {
        return;
    }
    let text = turn.text.clone();
    let bytes = text.as_bytes();
    let mut calls = Vec::new();
    let mut prose = String::new();
    let mut kept_from = 0;
    let mut at = 0;
    while at < bytes.len() {
        let found = (bytes[at] == b'{')
            .then(|| object_end(bytes, at))
            .flatten()
            .and_then(|end| Some((end, written_call(&text[at..end], tools)?)));
        match found {
            Some((end, (name, input))) => {
                prose.push_str(&text[kept_from..at]);
                calls.push(ToolCall {
                    id: format!("written_{seed}_{}", calls.len() + 1),
                    name,
                    input,
                });
                at = end;
                kept_from = end;
            }
            None => at += 1,
        }
    }
    if calls.is_empty() {
        return;
    }
    prose.push_str(&text[kept_from..]);
    // What wrapped the calls is dropped along with them.
    for wrapper in ["<tool_call>", "</tool_call>", "```json", "```"] {
        prose = prose.replace(wrapper, "");
    }
    turn.text = prose.trim().to_owned();
    turn.message = json!({
        "role": "assistant",
        "content": turn.text,
        "tool_calls": calls.iter().map(|call| json!({
            "id": call.id,
            "type": "function",
            "function": {"name": call.name, "arguments": call.input.to_string()},
        })).collect::<Vec<_>>(),
    });
    turn.tool_calls = calls;
    turn.stop = Stop::ToolUse;
}

/// Where the JSON object starting at `start` ends, if its braces balance.
fn object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for (i, &byte) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// Reads `{"name": ..., "arguments": {...}}` as a call to one of `tools`.
fn written_call(object: &str, tools: &[ToolSpec]) -> Option<(String, Value)> {
    let value: Value = serde_json::from_str(object).ok()?;
    // Some models nest the call under "function".
    let call = if value["function"].is_object() {
        &value["function"]
    } else {
        &value
    };
    let name = call["name"].as_str()?;
    if !tools.iter().any(|tool| tool.name == name) {
        return None;
    }
    let arguments = match (&call["arguments"], &call["parameters"]) {
        (Value::Null, Value::Null) => return None,
        (Value::Null, parameters) => parameters,
        (arguments, _) => arguments,
    };
    let input = match arguments {
        Value::String(text) => serde_json::from_str(text).ok()?,
        Value::Object(_) => arguments.clone(),
        _ => return None,
    };
    Some((name.to_owned(), input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: String::new(),
            schema: json!({"type": "object"}),
        }
    }

    fn said(text: &str) -> Turn {
        let reply = json!({"model": "m", "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": text}}]});
        parse_turn(&reply).unwrap()
    }

    #[test]
    fn a_tool_call_written_as_text_becomes_a_real_one() {
        let tools = [tool("find_pictures"), tool("set_operation")];
        let mut turn = said(
            "{\"name\": \"find_pictures\", \"arguments\": {\"query\": \"Bald Eagle {flag}\"}}\n\nThis should show you any images. Choose one and I can load it for you.",
        );
        recover_written_calls(&mut turn, &tools, 3);
        assert_eq!(turn.stop, Stop::ToolUse);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "find_pictures");
        assert_eq!(
            turn.tool_calls[0].input,
            json!({"query": "Bald Eagle {flag}"})
        );
        assert_eq!(turn.tool_calls[0].id, "written_3_1");
        assert_eq!(
            turn.text,
            "This should show you any images. Choose one and I can load it for you."
        );
        // The history gets a message the tool's result can be matched to.
        assert_eq!(turn.message["tool_calls"][0]["id"], "written_3_1");
        assert_eq!(
            turn.message["tool_calls"][0]["function"]["arguments"],
            "{\"query\":\"Bald Eagle {flag}\"}"
        );
    }

    #[test]
    fn wrapped_and_repeated_written_calls_are_found() {
        let tools = [tool("get_project"), tool("set_operation")];
        let mut turn = said(
            "<tool_call>\n{\"name\": \"get_project\", \"arguments\": {}}\n</tool_call>\n```json\n{\"function\": {\"name\": \"set_operation\", \"parameters\": {\"operation\": 1, \"depth\": 3}}}\n```",
        );
        recover_written_calls(&mut turn, &tools, 0);
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(
            turn.tool_calls[1].input,
            json!({"operation": 1, "depth": 3})
        );
        assert_eq!(turn.text, "");
    }

    #[test]
    fn ordinary_text_with_braces_is_left_alone() {
        let tools = [tool("get_project")];
        for text in [
            "Use {curly} braces, or {\"name\": \"something_else\", \"arguments\": {}}.",
            "The settings are {\"name\": \"get_project\"} and an unclosed { brace",
            "No JSON here at all.",
        ] {
            let mut turn = said(text);
            recover_written_calls(&mut turn, &tools, 0);
            assert_eq!(turn.stop, Stop::Done, "{text}");
            assert!(turn.tool_calls.is_empty());
            assert_eq!(turn.text, text);
        }
    }

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
