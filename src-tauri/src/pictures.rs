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
const COMMONS_URL: &str = "https://commons.wikimedia.org/w/api.php";
/// Commons serves scaled copies at a fixed set of widths, and refuses
/// programs it does not know the full-size originals. These are the widths
/// worth working from, largest first, and the one used for thumbnails.
const COMMONS_WIDTHS: [u32; 4] = [3840, 1920, 1280, 960];
const COMMONS_ASKED_WIDTH: u32 = 960;
const COMMONS_THUMBNAIL_WIDTH: u32 = 330;
const RESULTS: usize = 8;
/// Larger downloads are refused rather than read into memory.
const MAX_PICTURE_BYTES: u64 = 40 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Says who is asking, with where to find the project: sites such as
/// Wikimedia ask programs to identify themselves this way.
fn user_agent() -> &'static str {
    concat!(
        "Kerf/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/engineer983/Kerf)"
    )
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

#[derive(Serialize)]
pub struct SearchResults {
    pub pictures: Vec<FoundPicture>,
    /// How many pictures matched in all, of which these are the first few.
    /// It tells a caller trying several wordings which one suits best.
    pub total: u64,
}

/// Searches for pictures that may be adapted and used commercially, in two
/// collections at once. One failing does not lose the other's results.
#[tauri::command]
pub async fn search_pictures(
    query: String,
    public_domain_only: bool,
) -> Result<SearchResults, String> {
    blocking(move || {
        let words = query.trim().to_owned();
        let commons = {
            let words = words.clone();
            std::thread::spawn(move || search_commons(&words, public_domain_only))
        };
        let openverse = search_openverse(&words, public_domain_only);
        let commons = commons
            .join()
            .unwrap_or_else(|_| Err("The picture search failed.".into()));
        match (commons, openverse) {
            (Err(problem), Err(_)) => Err(problem),
            (commons, openverse) => {
                let (commons, commons_total) = commons.unwrap_or_default();
                let (openverse, openverse_total) = openverse.unwrap_or_default();
                Ok(SearchResults {
                    pictures: interleave(commons, openverse),
                    total: commons_total + openverse_total,
                })
            }
        }
    })
    .await
}

/// Takes results from each collection in turn, so both are represented.
fn interleave(first: Vec<FoundPicture>, second: Vec<FoundPicture>) -> Vec<FoundPicture> {
    let (mut first, mut second) = (first.into_iter(), second.into_iter());
    let mut mixed = Vec::new();
    loop {
        let before = mixed.len();
        mixed.extend(first.next());
        mixed.extend(second.next());
        if mixed.len() == before || mixed.len() >= RESULTS {
            break;
        }
    }
    mixed.truncate(RESULTS);
    mixed
}

fn search_failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(429, _) => {
            "The picture search is limiting requests; wait a minute and try again.".to_owned()
        }
        ureq::Error::Status(code, _) => format!("The picture search answered with error {code}."),
        other => format!("Could not reach the picture search: {other}"),
    }
}

fn search_openverse(
    query: &str,
    public_domain_only: bool,
) -> Result<(Vec<FoundPicture>, u64), String> {
    let reply: Value = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build()
        .get(SEARCH_URL)
        .set("User-Agent", user_agent())
        .query("q", query)
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
        .map_err(search_failure)?
        .into_json()
        .map_err(|e| format!("The picture search's reply could not be read: {e}"))?;
    Ok((
        parse_results(&reply),
        reply["result_count"].as_u64().unwrap_or(0),
    ))
}

/// Wikimedia Commons holds only freely licensed and public-domain files, and
/// is strong on historical illustrations, emblems and flags.
fn search_commons(
    query: &str,
    public_domain_only: bool,
) -> Result<(Vec<FoundPicture>, u64), String> {
    let reply: Value = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build()
        .get(COMMONS_URL)
        .set("User-Agent", user_agent())
        .query("action", "query")
        .query("format", "json")
        .query("generator", "search")
        .query("gsrnamespace", "6")
        .query("gsrinfo", "totalhits")
        .query("gsrlimit", &(RESULTS * 2).to_string())
        .query("gsrsearch", &format!("filetype:bitmap {query}"))
        .query("prop", "imageinfo")
        .query("iiprop", "url|size|mime|extmetadata")
        .query("iiurlwidth", &COMMONS_ASKED_WIDTH.to_string())
        .query(
            "iiextmetadatafilter",
            "LicenseShortName|Artist|AttributionRequired",
        )
        .call()
        .map_err(search_failure)?
        .into_json()
        .map_err(|e| format!("The picture search's reply could not be read: {e}"))?;
    Ok((
        parse_commons(&reply, public_domain_only),
        reply["query"]["searchinfo"]["totalhits"]
            .as_u64()
            .unwrap_or(0),
    ))
}

fn parse_commons(reply: &Value, public_domain_only: bool) -> Vec<FoundPicture> {
    let Some(pages) = reply["query"]["pages"].as_object() else {
        return Vec::new();
    };
    // Pages arrive keyed by id; `index` is their place in the results.
    let mut pages: Vec<&Value> = pages.values().collect();
    pages.sort_by_key(|page| page["index"].as_u64().unwrap_or(u64::MAX));
    pages
        .into_iter()
        .filter_map(|page| {
            let info = &page["imageinfo"][0];
            if !matches!(
                info["mime"].as_str()?,
                "image/jpeg" | "image/png" | "image/webp" | "image/gif"
            ) {
                return None;
            }
            // Only scaled copies can be downloaded, so a picture too small to
            // have one is left out. The address of one size gives the others.
            let asked = format!("/{COMMONS_ASKED_WIDTH}px-");
            let scaled = info["thumburl"].as_str().filter(|u| u.contains(&asked))?;
            let full_width = info["width"].as_u64().unwrap_or(0) as u32;
            let sized = |width: u32| scaled.replace(&asked, &format!("/{width}px-"));
            let url = COMMONS_WIDTHS
                .iter()
                .find(|width| **width < full_width)
                .map_or_else(|| scaled.to_owned(), |width| sized(*width));
            let meta = |key: &str| info["extmetadata"][key]["value"].as_str().unwrap_or("");
            let licence = meta("LicenseShortName").trim();
            let lower = licence.to_ascii_lowercase();
            let public_domain =
                lower.contains("public domain") || lower.starts_with("cc0") || lower == "pd";
            if licence.is_empty() || (public_domain_only && !public_domain) {
                return None;
            }
            // The artist is given as a fragment of HTML.
            let creator = crate::web::html_to_text(meta("Artist"))
                .text
                .replace('\n', " ");
            let name = page["title"].as_str().unwrap_or("Untitled");
            let name = name.strip_prefix("File:").unwrap_or(name);
            let title = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            let needs_credit = meta("AttributionRequired") == "true";
            let by = if creator.is_empty() {
                String::new()
            } else {
                format!(" by {creator}")
            };
            Some(FoundPicture {
                title: title.to_owned(),
                attribution: format!("\"{title}\"{by}, {licence}, via Wikimedia Commons."),
                creator,
                licence: licence.to_owned(),
                needs_credit,
                page: info["descriptionurl"].as_str().unwrap_or("").to_owned(),
                thumbnail: sized(COMMONS_THUMBNAIL_WIDTH),
                url,
                width: full_width,
                height: info["height"].as_u64().unwrap_or(0) as u32,
            })
        })
        .take(RESULTS)
        .collect()
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
            ureq::Error::Status(code @ (403 | 429), _) => format!(
                "The site holding the picture refuses downloads by programs (error {code}). Open the picture in your web browser, right-click it and choose Copy image, then press Ctrl+V in Kerf."
            ),
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
    fn commons_results_are_read_in_order_with_scaled_addresses() {
        let reply = json!({"query": {"pages": {
            "9": {"index": 2, "title": "File:Scan.tif", "imageinfo": [{"mime": "image/tiff", "width": 4000,
                "thumburl": "https://thumb.wikimedia.org/x/thumb/a/Scan.tif/960px-Scan.tif.jpg",
                "extmetadata": {"LicenseShortName": {"value": "Public domain"}}}]},
            "7": {"index": 3, "title": "File:Chainsaw Eagle.jpg", "imageinfo": [{"mime": "image/jpeg", "width": 2736, "height": 3648,
                "thumburl": "https://thumb.wikimedia.org/x/thumb/a/Eagle.jpg/960px-Eagle.jpg?utm=1",
                "descriptionurl": "https://commons.wikimedia.org/wiki/File:Chainsaw_Eagle.jpg",
                "extmetadata": {"LicenseShortName": {"value": "CC BY 2.0"}, "AttributionRequired": {"value": "true"},
                    "Artist": {"value": "<a href=\"//commons.wikimedia.org/wiki/User:Ann\" title=\"User:Ann\">Ann</a>"}}}]},
            "5": {"index": 4, "title": "File:Tiny.jpg", "imageinfo": [{"mime": "image/jpeg", "width": 399, "height": 280,
                "thumburl": "https://upload.wikimedia.org/a/Tiny.jpg?utm=unscaled",
                "extmetadata": {"LicenseShortName": {"value": "Public domain"}}}]},
            "3": {"index": 1, "title": "File:Bald Eagle and American Flag (1904).png", "imageinfo": [{"mime": "image/png", "width": 1066, "height": 1594,
                "thumburl": "https://thumb.wikimedia.org/x/thumb/a/Flag.png/960px-Flag.png", "url": "https://upload.wikimedia.org/a/Flag.png",
                "extmetadata": {"LicenseShortName": {"value": "Public domain"}, "AttributionRequired": {"value": "false"}, "Artist": {"value": "Perrysburg journal"}}}]}
        }}});
        let found = parse_commons(&reply, false);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].title, "Bald Eagle and American Flag (1904)");
        assert!(!found[0].needs_credit);
        // The largest standard size smaller than the original is the one used.
        assert_eq!(
            found[0].url,
            "https://thumb.wikimedia.org/x/thumb/a/Flag.png/960px-Flag.png"
        );
        assert_eq!(
            found[0].thumbnail,
            "https://thumb.wikimedia.org/x/thumb/a/Flag.png/330px-Flag.png"
        );
        assert_eq!(found[1].creator, "Ann");
        assert_eq!(found[1].licence, "CC BY 2.0");
        assert!(found[1].needs_credit);
        assert_eq!(
            found[1].url,
            "https://thumb.wikimedia.org/x/thumb/a/Eagle.jpg/1920px-Eagle.jpg?utm=1"
        );
        assert_eq!(
            found[1].thumbnail,
            "https://thumb.wikimedia.org/x/thumb/a/Eagle.jpg/330px-Eagle.jpg?utm=1"
        );
        assert_eq!((found[1].width, found[1].height), (2736, 3648));
        assert_eq!(
            found[1].attribution,
            "\"Chainsaw Eagle\" by Ann, CC BY 2.0, via Wikimedia Commons."
        );

        assert_eq!(parse_commons(&reply, true).len(), 1);
        assert!(parse_commons(&json!({"batchcomplete": ""}), false).is_empty());
    }
    #[test]
    fn both_collections_share_the_results() {
        let picture = |title: &str| FoundPicture {
            title: title.into(),
            creator: String::new(),
            licence: "Public domain".into(),
            needs_credit: false,
            attribution: String::new(),
            page: String::new(),
            url: String::new(),
            thumbnail: String::new(),
            width: 0,
            height: 0,
        };
        let titles = |found: Vec<FoundPicture>| {
            found
                .into_iter()
                .map(|p| p.title)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            titles(interleave(
                vec![picture("a"), picture("b")],
                vec![picture("1"), picture("2"), picture("3")]
            )),
            "a 1 b 2 3"
        );
        assert_eq!(titles(interleave(Vec::new(), vec![picture("1")])), "1");
        let many = |prefix: &str| {
            (0..8)
                .map(|i| picture(&format!("{prefix}{i}")))
                .collect::<Vec<_>>()
        };
        assert_eq!(interleave(many("a"), many("b")).len(), RESULTS);
    }
    #[test]
    fn only_web_addresses_are_downloaded() {
        assert!(download("file:///C:/Windows/win.ini").is_err());
        assert!(download("C:\\Windows\\win.ini").is_err());
    }
}
