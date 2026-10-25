//! Commands for the assistant: talking to the chosen model, and keeping API
//! keys in the operating system's credential store.
//!
//! Keys are written and used on this side only. The UI can ask whether one is
//! saved, but can never read it back.

use kerf_ai::{Endpoint, ModelInfo, ToolSpec, Turn};
use serde_json::Value;

use crate::blocking;

/// The name the keys are filed under in the credential store.
const KEYRING_SERVICE: &str = "dev.kerf.app";

fn entry(key_id: &str) -> Result<keyring::Entry, String> {
    let safe = !key_id.is_empty()
        && key_id.len() <= 64
        && key_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !safe {
        return Err("That is not a valid provider id.".into());
    }
    keyring::Entry::new(KEYRING_SERVICE, key_id)
        .map_err(|e| format!("Could not open the system's credential store: {e}"))
}

/// The saved key for a provider, or `None` if there is none.
pub(crate) fn saved_key(key_id: &str) -> Result<Option<String>, String> {
    match entry(key_id)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the saved API key: {e}")),
    }
}

#[tauri::command]
pub async fn set_api_key(key_id: String, key: String) -> Result<(), String> {
    blocking(move || {
        entry(&key_id)?
            .set_password(key.trim())
            .map_err(|e| format!("Could not save the API key: {e}"))
    })
    .await
}

#[tauri::command]
pub async fn has_api_key(key_id: String) -> Result<bool, String> {
    blocking(move || Ok(saved_key(&key_id)?.is_some())).await
}

#[tauri::command]
pub async fn delete_api_key(key_id: String) -> Result<(), String> {
    blocking(move || match entry(&key_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("Could not remove the API key: {e}")),
    })
    .await
}

/// Sends the conversation to the model and returns its next turn. The tools
/// are only described here; the UI runs whichever ones the model calls.
#[tauri::command]
pub async fn assistant_send(
    endpoint: Endpoint,
    key_id: String,
    system: String,
    messages: Vec<Value>,
    tools: Vec<ToolSpec>,
) -> Result<Turn, String> {
    blocking(move || {
        let key = saved_key(&key_id)?;
        kerf_ai::send(&endpoint, key.as_deref(), &system, &messages, &tools)
    })
    .await
}

#[tauri::command]
pub async fn assistant_models(
    endpoint: Endpoint,
    key_id: String,
) -> Result<Vec<ModelInfo>, String> {
    blocking(move || {
        let key = saved_key(&key_id)?;
        kerf_ai::list_models(&endpoint, key.as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    /// The assistant's tools, as both versions send them with `assistant_send`.
    const TOOLS: &str = include_str!("../../src/lib/assistantTools.ts");

    /// Every tool name the assistant is given.
    fn tool_names() -> Vec<&'static str> {
        TOOLS
            .split("name: \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect()
    }

    #[test]
    fn the_assistant_has_no_tool_that_reaches_the_machine() {
        let names = tool_names();
        assert!(names.contains(&"generate_gcode"), "{names:?}");
        let forbidden = [
            "machine",
            "connect",
            "jog",
            "home",
            "probe",
            "continue",
            "start",
            "stop",
            "spindle",
            "export",
            "save_file",
            "send",
            "zero",
            "frame",
            "camera",
        ];
        for name in &names {
            let lower = name.to_ascii_lowercase();
            for word in forbidden {
                assert!(
                    !lower.contains(word),
                    "assistant tool {name} looks like a machine, probe, continue or export action"
                );
            }
        }
        // Nor does any tool call the backend for one directly.
        for call in [
            "machineAction",
            "connectMachine",
            "disconnectMachine",
            "exportGcode",
            "exportSvg",
            "useMachine",
            "invoke(",
        ] {
            assert!(!TOOLS.contains(call), "assistantTools.ts uses {call}");
        }
    }
}
