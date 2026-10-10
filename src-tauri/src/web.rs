//! Fetching a product page as plain text, for filling in a tool's details.

use std::io::Read;
use std::time::Duration;

use serde::Serialize;

use crate::blocking;

/// Pages larger than this are cut off; product details are near the top.
const MAX_PAGE_BYTES: u64 = 3 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Serialize)]
pub struct PageText {
    pub title: String,
    pub text: String,
}

/// Downloads `url` and returns its visible text. Only the page itself is
/// fetched: no scripts run, so pages that build their content in the browser,
/// or that refuse automated requests, come back empty or with an error.
#[tauri::command]
pub async fn fetch_page_text(url: String) -> Result<PageText, String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Enter a web address starting with https://".into());
    }
    blocking(move || {
        let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
        let response = agent
            .get(&url)
            .set("User-Agent", concat!("Kerf/", env!("CARGO_PKG_VERSION")))
            .set("Accept", "text/html")
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(code, _) => {
                    format!("The site answered with error {code}. Copy the product details from your browser and paste them instead.")
                }
                other => format!("Could not reach the page: {other}"),
            })?;
        let mut html = String::new();
        response
            .into_reader()
            .take(MAX_PAGE_BYTES)
            .read_to_string(&mut html)
            .map_err(|e| format!("Could not read the page: {e}"))?;
        Ok(html_to_text(&html))
    })
    .await
}

/// Strips tags, scripts and styles, leaving the text a reader would see.
fn html_to_text(html: &str) -> PageText {
    let lower = html.to_ascii_lowercase();
    let title = lower
        .find("<title")
        .and_then(|start| lower[start..].find('>').map(|close| start + close + 1))
        .and_then(|start| {
            lower[start..]
                .find("</title")
                .map(|end| &html[start..start + end])
        })
        .map(|t| decode_entities(t.trim()))
        .unwrap_or_default();

    let mut text = String::new();
    let mut rest = 0;
    while rest < html.len() {
        let Some(open) = html[rest..].find('<').map(|i| rest + i) else {
            text.push_str(&html[rest..]);
            break;
        };
        text.push_str(&html[rest..open]);
        // Skip the whole of elements whose content is never shown.
        let hidden = ["script", "style", "noscript", "svg", "head"]
            .iter()
            .find(|tag| {
                lower[open + 1..].starts_with(**tag)
                    && !lower[open + 1 + tag.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric())
            });
        let after = match hidden {
            Some(tag) => {
                let closing = format!("</{tag}");
                lower[open..]
                    .find(&closing)
                    .map(|i| open + i + closing.len())
            }
            None => Some(open),
        };
        let Some(after) = after else { break };
        let Some(close) = html[after..].find('>').map(|i| after + i + 1) else {
            break;
        };
        // Tags separate words and lines.
        text.push('\n');
        rest = close;
    }

    let text = decode_entities(&text)
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    PageText { title, text }
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&deg;", "°")
        .replace("&frac12;", "1/2")
        .replace("&frac14;", "1/4")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_visible_text_and_drops_scripts_and_styles() {
        let page = html_to_text(
            "<html><head><title> 1/8&quot; Ball Nose &amp; More </title><style>p{color:red}</style></head>\
             <body><script>var x = '<b>no</b>';</script><h1>Ball Nose</h1>\
             <table><tr><td>Cutting Diameter</td><td>1/8&quot;</td></tr></table>\
             <p>2&nbsp;flutes,   carbide</p><svg><text>icon</text></svg></body></html>",
        );
        assert_eq!(page.title, "1/8\" Ball Nose & More");
        assert_eq!(
            page.text,
            "Ball Nose\nCutting Diameter\n1/8\"\n2 flutes, carbide"
        );
    }

    #[test]
    fn similar_tag_names_are_not_hidden() {
        // <header> must not be treated as <head>.
        let page = html_to_text("<header>Shop</header><p>End mill</p>");
        assert_eq!(page.text, "Shop\nEnd mill");
    }

    #[test]
    fn broken_html_does_not_panic() {
        assert_eq!(html_to_text("plain text").text, "plain text");
        assert_eq!(html_to_text("<p>open <b").text, "open");
        assert_eq!(html_to_text("<script>never closed").text, "");
    }
}
