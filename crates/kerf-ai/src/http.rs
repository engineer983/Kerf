//! JSON over HTTP, with failures turned into messages a person can act on.

use serde_json::Value;

/// Makes a request and reads the reply as JSON.
#[cfg(feature = "http")]
pub(crate) fn send(request: &crate::Request) -> Result<Value, String> {
    /// Models can take minutes on a hard question; shorter would cut them off.
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
    let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
    let mut call = match request.body {
        Some(_) => agent.post(&request.url),
        None => agent.get(&request.url),
    };
    for (name, value) in &request.headers {
        call = call.set(name, value);
    }
    let result = match &request.body {
        Some(body) => call.send_json(body),
        None => call.call(),
    };
    match result {
        Ok(response) => response
            .into_json()
            .map_err(|e| format!("The provider's reply could not be read: {e}")),
        Err(ureq::Error::Status(status, response)) => {
            let body: Value = response.into_json().unwrap_or(Value::Null);
            Err(describe_failure(status, &body))
        }
        Err(ureq::Error::Transport(transport)) => {
            Err(could_not_reach(&request.url, &transport.to_string()))
        }
    }
}

/// Explains a request that never got an answer.
pub fn could_not_reach(url: &str, why: &str) -> String {
    format!(
        "Could not reach {}: {why}. Check the address and that the service is running.",
        host(url)
    )
}

fn host(url: &str) -> &str {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest)
}

/// Explains an error status, using the provider's own message where it gave one.
/// Both protocols put it at `error.message`; some servers send a bare string.
pub fn describe_failure(status: u16, body: &Value) -> String {
    let detail = body["error"]["message"]
        .as_str()
        .or_else(|| body["error"].as_str())
        .or_else(|| body["message"].as_str())
        .unwrap_or("");
    let meaning = match status {
        401 | 403 => "The API key was not accepted.",
        404 => "The model or address was not found.",
        429 => "The provider is limiting requests; wait a moment and try again.",
        400 | 422 => "The provider rejected the request.",
        500..=599 => "The provider had a problem or is overloaded; try again shortly.",
        _ => "The request failed.",
    };
    if detail.is_empty() {
        format!("{meaning} (HTTP {status})")
    } else {
        format!("{meaning} (HTTP {status}: {detail})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failures_carry_the_providers_own_message() {
        let anthropic = json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}});
        assert_eq!(
            describe_failure(401, &anthropic),
            "The API key was not accepted. (HTTP 401: invalid x-api-key)"
        );
        let ollama = json!({"error": "model 'nope' not found"});
        assert!(describe_failure(404, &ollama).contains("model 'nope' not found"));
        assert_eq!(
            describe_failure(529, &Value::Null),
            "The provider had a problem or is overloaded; try again shortly. (HTTP 529)"
        );
    }

    #[test]
    fn host_is_taken_from_the_address() {
        assert_eq!(
            host("http://localhost:11434/v1/chat/completions"),
            "localhost:11434"
        );
        assert_eq!(
            host("https://api.anthropic.com/v1/messages"),
            "api.anthropic.com"
        );
    }
}
