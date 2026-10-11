//! Making a picture from a description.
//!
//! Two kinds of server are spoken to: anything with an OpenAI-style
//! `images/generations` endpoint (OpenAI itself, LocalAI and others), and the
//! Stable Diffusion web UI's own API (AUTOMATIC1111 and Forge started with
//! `--api`), which is what most people run on their own computer.

use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{openai, Request};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageProtocol {
    OpenAi,
    WebUi,
}

/// Where to ask for pictures, and which model to ask for.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEndpoint {
    pub protocol: ImageProtocol,
    /// For example `https://api.openai.com/v1` or `http://127.0.0.1:7860`.
    pub base_url: String,
    /// May be empty for a local server, which then uses whatever it has loaded.
    pub model: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shape {
    Square,
    Wide,
    Tall,
}

/// What a server sent back: the picture itself, or where to collect it.
#[derive(Debug, PartialEq)]
pub enum Generated {
    Bytes(Vec<u8>),
    Url(String),
}

/// Asks the server for one picture of `prompt`.
#[cfg(feature = "http")]
pub fn generate_image(
    endpoint: &ImageEndpoint,
    key: Option<&str>,
    prompt: &str,
    shape: Shape,
) -> Result<Generated, String> {
    let reply = crate::http::send(&image_request(endpoint, key, prompt, shape))?;
    read_image(endpoint, &reply)
}

/// The request that asks the server for one picture of `prompt`.
pub fn image_request(
    endpoint: &ImageEndpoint,
    key: Option<&str>,
    prompt: &str,
    shape: Shape,
) -> Request {
    let base = endpoint.base_url.trim_end_matches('/');
    let model = endpoint.model.trim();
    match endpoint.protocol {
        ImageProtocol::OpenAi => Request {
            url: format!("{base}/images/generations"),
            headers: openai::headers(key),
            body: Some(openai_body(model, prompt, shape)),
        },
        ImageProtocol::WebUi => Request {
            url: format!("{base}/sdapi/v1/txt2img"),
            headers: Vec::new(),
            body: Some(web_ui_body(model, prompt, shape)),
        },
    }
}

/// Reads the server's reply to [`image_request`].
pub fn read_image(endpoint: &ImageEndpoint, reply: &Value) -> Result<Generated, String> {
    match endpoint.protocol {
        ImageProtocol::OpenAi => parse_openai(reply),
        ImageProtocol::WebUi => parse_web_ui(reply),
    }
}

/// Each family of model accepts its own set of sizes.
fn openai_size(model: &str, shape: Shape) -> &'static str {
    let sizes = if model.starts_with("gpt-image") {
        ["1024x1024", "1536x1024", "1024x1536"]
    } else if model.starts_with("dall-e-3") {
        ["1024x1024", "1792x1024", "1024x1792"]
    } else {
        ["1024x1024", "1216x832", "832x1216"]
    };
    sizes[shape as usize]
}

fn openai_body(model: &str, prompt: &str, shape: Shape) -> Value {
    let mut body = json!({"prompt": prompt, "n": 1, "size": openai_size(model, shape)});
    if !model.is_empty() {
        body["model"] = json!(model);
    }
    if model.starts_with("gpt-image") {
        // These always reply with the picture itself, and reject the field
        // that asks for it. Medium quality keeps the cost of a picture modest.
        body["quality"] = json!("medium");
    } else {
        body["response_format"] = json!("b64_json");
    }
    body
}

fn parse_openai(reply: &Value) -> Result<Generated, String> {
    let first = &reply["data"][0];
    if let Some(encoded) = first["b64_json"].as_str() {
        return decode(encoded);
    }
    if let Some(url) = first["url"].as_str() {
        return Ok(Generated::Url(url.to_owned()));
    }
    Err("The image service's reply did not contain a picture.".into())
}

fn web_ui_body(model: &str, prompt: &str, shape: Shape) -> Value {
    let (width, height) = [(1024, 1024), (1216, 832), (832, 1216)][shape as usize];
    let mut body = json!({
        "prompt": prompt,
        "negative_prompt": "blurry, low quality, watermark, signature, text",
        "width": width,
        "height": height,
        "steps": 25,
    });
    if !model.is_empty() {
        body["override_settings"] = json!({"sd_model_checkpoint": model});
    }
    body
}

fn parse_web_ui(reply: &Value) -> Result<Generated, String> {
    let encoded = reply["images"][0]
        .as_str()
        .ok_or("The image server's reply did not contain a picture.")?;
    decode(encoded)
}

fn decode(encoded: &str) -> Result<Generated, String> {
    // Some servers send a data URL rather than bare base64.
    let bare = encoded
        .rsplit_once("base64,")
        .map_or(encoded, |(_, rest)| rest);
    base64::engine::general_purpose::STANDARD
        .decode(bare.trim())
        .map(Generated::Bytes)
        .map_err(|e| format!("The picture that came back could not be read: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_openai_models_get_their_own_sizes_and_fields() {
        let body = openai_body("gpt-image-1", "a hawk", Shape::Wide);
        assert_eq!(body["size"], "1536x1024");
        assert_eq!(body["quality"], "medium");
        assert!(body.get("response_format").is_none());

        let body = openai_body("dall-e-3", "a hawk", Shape::Tall);
        assert_eq!(body["size"], "1024x1792");
        assert_eq!(body["response_format"], "b64_json");
        assert!(body.get("quality").is_none());
    }

    #[test]
    fn a_local_server_may_be_asked_without_naming_a_model() {
        let body = openai_body("", "a hawk", Shape::Square);
        assert!(body.get("model").is_none());
        assert_eq!(body["size"], "1024x1024");
        assert_eq!(body["response_format"], "b64_json");

        let body = web_ui_body("", "a hawk", Shape::Wide);
        assert_eq!(
            (body["width"].as_i64(), body["height"].as_i64()),
            (Some(1216), Some(832))
        );
        assert!(body.get("override_settings").is_none());
        let body = web_ui_body("sdxl.safetensors", "a hawk", Shape::Square);
        assert_eq!(
            body["override_settings"]["sd_model_checkpoint"],
            "sdxl.safetensors"
        );
    }

    #[test]
    fn pictures_are_read_from_either_kind_of_reply() {
        // "hello" in base64.
        let picture = Generated::Bytes(b"hello".to_vec());
        assert_eq!(
            parse_openai(&json!({"data": [{"b64_json": "aGVsbG8="}]})),
            Ok(Generated::Bytes(b"hello".to_vec()))
        );
        assert_eq!(
            parse_openai(&json!({"data": [{"url": "https://example.com/a.png"}]})),
            Ok(Generated::Url("https://example.com/a.png".into()))
        );
        assert_eq!(parse_web_ui(&json!({"images": ["aGVsbG8="]})), Ok(picture));
        assert_eq!(
            parse_web_ui(&json!({"images": ["data:image/png;base64,aGVsbG8="]})),
            Ok(Generated::Bytes(b"hello".to_vec()))
        );
    }

    #[test]
    fn replies_without_a_picture_are_errors() {
        assert!(parse_openai(&json!({"data": []})).is_err());
        assert!(parse_web_ui(&json!({"images": []})).is_err());
        assert!(parse_web_ui(&json!({"images": ["not base64!"]})).is_err());
    }
}
