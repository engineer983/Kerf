//! Getting a picture from somewhere other than the person's own files: made
//! by an image model, or found on the web among pictures free to reuse.

use std::io::Read;
use std::time::Duration;

use kerf_ai::{Generated, ImageEndpoint, Shape};
use serde::Serialize;
use serde_json::Value;
use tauri::ipc::Response;

use crate::assistant::saved_key;
use crate::blocking;

/// Openverse indexes openly licensed pictures from Flickr, Wikimedia Commons,
/// museums and others, and needs no key for occasional use.
const SEARCH_URL: &str = "https://api.openverse.org/v1/images/";
/// Licences that allow both adapting a picture and selling what is made from
/// it. Attribution may still be required.
const REUSABLE: &str = "cc0,pdm,by,by-sa";
const PUBLIC_DOMAIN: &str = "cc0,pdm";
const RESULTS: usize = 8;
/// Larger downloads are refused rather than read into memory.
const MAX_PICTURE_BYTES: u64 = 40 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

fn user_agent() -> &'static str {
    concat!("Kerf/", env!("CARGO_PKG_VERSION"))
}

/// Makes a picture of `prompt` and returns the image file's bytes.
#[tauri::command]
pub async fn generate_picture(
    endpoint: ImageEndpoint,
    key_id: String,
    prompt: String,
    shape: Shape,
) -> Result<Response, String> {
    blocking(move || {
        let key = saved_key(&key_id)?;
        match kerf_ai::generate_image(&endpoint, key.as_deref(), &prompt, shape)? {
            Generated::Bytes(bytes) => Ok(Response::new(bytes)),
            Generated::Url(url) => download(&url).map(Response::new),
        }
    })
    .await
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundPicture {
    pub title: String,
    pub creator: String,
    /// For example "CC BY 2.0" or "Public domain".
    pub licence: String,
    /// Whether the licence asks for the creator to be credited.
    pub needs_credit: bool,
    /// The credit line to give, as the source words it.
    pub attribution: String,
    /// The page the picture comes from.
    pub page: String,
    pub url: String,
    pub thumbnail: String,
    pub width: u32,
    pub height: u32,
}

/// Searches for pictures that may be adapted and used commercially.
#[tauri::command]
pub async fn search_pictures(
    query: String,
    public_domain_only: bool,
) -> Result<Vec<FoundPicture>, String> {
    blocking(move || {
        let reply: Value = ureq::AgentBuilder::new()
            .timeout(TIMEOUT)
            .build()
            .get(SEARCH_URL)
            .set("User-Agent", user_agent())
            .query("q", query.trim())
            .query(
                "license",
                if public_domain_only {
                    PUBLIC_DOMAIN
                } else {
                    REUSABLE
                },
            )
            // A few spare, since some results are in formats that cannot be used.
            .query("page_size", &(RESULTS * 2).to_string())
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(429, _) => {
                    "The picture search is limiting requests; wait a minute and try again."
                        .to_owned()
                }
                ureq::Error::Status(code, _) => {
                    format!("The picture search answered with error {code}.")
                }
                other => format!("Could not reach the picture search: {other}"),
            })?
            .into_json()
            .map_err(|e| format!("The picture search's reply could not be read: {e}"))?;
        Ok(parse_results(&reply))
    })
    .await
}

fn parse_results(reply: &Value) -> Vec<FoundPicture> {
    let Some(results) = reply["results"].as_array() else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str()?;
            let lower = url.to_ascii_lowercase();
            // Only ordinary web pictures can be loaded.
            let usable = url.starts_with("https://")
                && ![".svg", ".tif", ".tiff", ".djvu", ".pdf"]
                    .iter()
                    .any(|ext| lower.ends_with(ext));
            if !usable {
                return None;
            }
            let code = r["license"].as_str()?;
            let public_domain = matches!(code, "cc0" | "pdm");
            let licence = match code {
                "cc0" => "Public domain (CC0)".to_owned(),
                "pdm" => "Public domain".to_owned(),
                other => format!(
                    "CC {} {}",
                    other.to_ascii_uppercase(),
                    r["license_version"].as_str().unwrap_or("")
                )
                .trim()
                .to_owned(),
            };
            let text = |key: &str| r[key].as_str().unwrap_or("").trim().to_owned();
            Some(FoundPicture {
                title: Some(text("title"))
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "Untitled".into()),
                creator: text("creator"),
                licence,
                needs_credit: !public_domain,
                attribution: text("attribution"),
                page: text("foreign_landing_url"),
                url: url.to_owned(),
                thumbnail: text("thumbnail"),
                width: r["width"].as_u64().unwrap_or(0) as u32,
                height: r["height"].as_u64().unwrap_or(0) as u32,
            })
        })
        .take(RESULTS)
        .collect()
}

/// Downloads a picture found by a search, or its thumbnail.
#[tauri::command]
pub async fn fetch_picture(url: String) -> Result<Response, String> {
    blocking(move || download(&url).map(Response::new)).await
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("The picture's address is not a web address.".into());
    }
    let response = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build()
        .get(url)
        .set("User-Agent", user_agent())
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => {
                format!("The site holding the picture answered with error {code}.")
            }
            other => format!("Could not download the picture: {other}"),
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_PICTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Could not download the picture: {e}"))?;
    if bytes.len() as u64 > MAX_PICTURE_BYTES {
        return Err("The picture is too large to download.".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn results_carry_the_licence_and_credit() {
        let found = parse_results(&json!({"results": [
            {
                "title": "Bald Eagle", "creator": "Pen Waggener", "license": "by", "license_version": "2.0",
                "url": "https://live.staticflickr.com/127/a_b.jpg",
                "thumbnail": "https://api.openverse.org/v1/images/e3/thumb/",
                "foreign_landing_url": "https://www.flickr.com/photos/1/2",
                "attribution": "\"Bald Eagle\" by Pen Waggener is licensed under CC BY 2.0.",
                "width": 1024, "height": 684
            },
            {"title": "", "creator": null, "license": "cc0", "license_version": "1.0", "url": "https://example.org/eagle.png"}
        ]}));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].licence, "CC BY 2.0");
        assert!(found[0].needs_credit);
        assert_eq!((found[0].width, found[0].height), (1024, 684));
        assert_eq!(found[1].title, "Untitled");
        assert_eq!(found[1].licence, "Public domain (CC0)");
        assert!(!found[1].needs_credit);
    }

    #[test]
    fn pictures_that_cannot_be_loaded_are_left_out() {
        let found = parse_results(&json!({"results": [
            {"title": "Drawing", "license": "cc0", "url": "https://example.org/eagle.SVG"},
            {"title": "Scan", "license": "pdm", "url": "https://example.org/eagle.tif"},
            {"title": "Plain", "license": "pdm", "url": "http://example.org/eagle.jpg"},
            {"title": "No licence", "url": "https://example.org/eagle.jpg"},
            {"title": "Good", "license": "pdm", "url": "https://example.org/eagle.jpg"}
        ]}));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Good");
        assert_eq!(found[0].licence, "Public domain");
        assert!(parse_results(&json!({"detail": "error"})).is_empty());
    }

    #[test]
    fn only_web_addresses_are_downloaded() {
        assert!(download("file:///C:/Windows/win.ini").is_err());
        assert!(download("C:\\Windows\\win.ini").is_err());
    }
}
