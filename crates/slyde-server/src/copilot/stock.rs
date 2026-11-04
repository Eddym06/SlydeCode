//! Stock image search for decks that have no image generator.
//!
//! This is the fallback the copilot advertises when no image model is
//! configured. It returns direct, already-hosted picture URLs and the
//! attribution each service requires; the model then places one with
//! `insert_image`, which downloads and stores it through the SSRF-safe intake.
//!
//! Keys are stored next to the provider keys (OS credential store or the
//! owner-only config file) and can also come from the environment:
//!
//! - Unsplash: `UNSPLASH_ACCESS_KEY` (`Client-ID`)
//! - Pexels: `PEXELS_API_KEY` (`Authorization`)
//! - Vecteezy: `VECTEEZY_API_KEY` + `VECTEEZY_ACCOUNT_ID` (bearer + path account)
//!
//! A missing key is not an error by itself: the tool reports which services are
//! available and the model falls back to a URL the user supplied.

/// Stable ids used in the config map and the credential store.
pub const UNSPLASH_KEY: &str = "stock:unsplash";
pub const PEXELS_KEY: &str = "stock:pexels";
pub const VECTEEZY_KEY: &str = "stock:vecteezy";
pub const VECTEEZY_ACCOUNT: &str = "stock:vecteezy_account";
pub const SECRET_IDS: &[&str] = &[UNSPLASH_KEY, PEXELS_KEY, VECTEEZY_KEY, VECTEEZY_ACCOUNT];

use std::time::Duration;

use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use super::error::CopilotError;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_RESULTS: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StockHit {
    pub provider: &'static str,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
    pub attribution: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockKind {
    Photo,
    Vector,
}

impl StockKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "photo" | "image" | "" => Some(Self::Photo),
            "vector" | "illustration" | "svg" => Some(Self::Vector),
            _ => None,
        }
    }
}

/// Stock keys after merging stored settings with the environment. Stored
/// values win, so the settings screen can override a deploy-time env var.
#[derive(Debug, Clone, Default)]
pub struct StockKeys {
    unsplash: Option<String>,
    pexels: Option<String>,
    vecteezy: Option<String>,
    vecteezy_account: Option<String>,
}

impl StockKeys {
    fn from_env() -> Self {
        let read = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        Self {
            unsplash: read("UNSPLASH_ACCESS_KEY"),
            pexels: read("PEXELS_API_KEY"),
            vecteezy: read("VECTEEZY_API_KEY"),
            vecteezy_account: read("VECTEEZY_ACCOUNT_ID"),
        }
    }

    /// Builds the effective key set from a lookup over the stored config.
    /// `lookup` receives the stable ids in `SECRET_IDS`.
    pub fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        let mut keys = Self::from_env();
        let stored = |value: Option<String>| {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        if let Some(key) = stored(lookup(UNSPLASH_KEY)) {
            keys.unsplash = Some(key);
        }
        if let Some(key) = stored(lookup(PEXELS_KEY)) {
            keys.pexels = Some(key);
        }
        if let Some(key) = stored(lookup(VECTEEZY_KEY)) {
            keys.vecteezy = Some(key);
        }
        if let Some(account) = stored(lookup(VECTEEZY_ACCOUNT)) {
            keys.vecteezy_account = Some(account);
        }
        keys
    }

    fn has_vectors(&self) -> bool {
        self.vecteezy.is_some() && self.vecteezy_account.is_some()
    }

    /// Human-readable configuration summary for the tool result and the prompt.
    pub fn providers(&self) -> Vec<&'static str> {
        let mut providers = Vec::new();
        if self.unsplash.is_some() {
            providers.push("unsplash");
        }
        if self.pexels.is_some() {
            providers.push("pexels");
        }
        if self.has_vectors() {
            providers.push("vecteezy");
        }
        providers
    }
}

pub async fn search(
    client: &Client,
    keys: &StockKeys,
    query: &str,
    kind: StockKind,
    count: usize,
    orientation: Option<&str>,
) -> Result<Vec<StockHit>, CopilotError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(CopilotError::InvalidRequest(
            "a stock image search needs a query".into(),
        ));
    }
    if query.chars().count() > 160 {
        return Err(CopilotError::InvalidRequest(
            "the stock image query is too long".into(),
        ));
    }
    let count = count.clamp(1, MAX_RESULTS);

    let mut errors = Vec::new();
    match kind {
        StockKind::Photo => {
            if let Some(key) = &keys.unsplash {
                match unsplash(client, key, query, count, orientation).await {
                    Ok(hits) if !hits.is_empty() => return Ok(hits),
                    Ok(_) => errors.push("Unsplash returned no pictures".to_string()),
                    Err(error) => errors.push(format!("Unsplash: {error}")),
                }
            }
            if let Some(key) = &keys.pexels {
                match pexels(client, key, query, count, orientation).await {
                    Ok(hits) if !hits.is_empty() => return Ok(hits),
                    Ok(_) => errors.push("Pexels returned no pictures".to_string()),
                    Err(error) => errors.push(format!("Pexels: {error}")),
                }
            }
            if errors.is_empty() {
                Err(CopilotError::InvalidRequest(
                    "no stock photo service is configured; set UNSPLASH_ACCESS_KEY or PEXELS_API_KEY"
                        .into(),
                ))
            } else {
                Err(CopilotError::Upstream(errors.join("; ")))
            }
        }
        StockKind::Vector => {
            let (Some(key), Some(account)) = (&keys.vecteezy, &keys.vecteezy_account) else {
                return Err(CopilotError::InvalidRequest(
                    "no stock vector service is configured; set VECTEEZY_API_KEY and VECTEEZY_ACCOUNT_ID"
                        .into(),
                ));
            };
            vecteezy(client, key, account, query, count, orientation).await
        }
    }
}

async fn get_json(request: reqwest::RequestBuilder, provider: &str) -> Result<Value, CopilotError> {
    let response = request
        .timeout(TIMEOUT)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|error| CopilotError::Network(format!("{provider}: {error}")))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CopilotError::InvalidKey);
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(CopilotError::RateLimited);
    }
    if !status.is_success() {
        let detail = response.text().await.unwrap_or_default();
        let detail = detail.chars().take(240).collect::<String>();
        return Err(CopilotError::Upstream(format!(
            "{provider} answered {status}: {detail}"
        )));
    }
    response
        .json::<Value>()
        .await
        .map_err(|error| CopilotError::Upstream(format!("{provider}: {error}")))
}

async fn unsplash(
    client: &Client,
    key: &str,
    query: &str,
    count: usize,
    orientation: Option<&str>,
) -> Result<Vec<StockHit>, CopilotError> {
    let mut request = client
        .get("https://api.unsplash.com/search/photos")
        .query(&[("query", query), ("per_page", &count.to_string())])
        .header("Authorization", format!("Client-ID {key}"))
        .header("Accept-Version", "v1");
    if let Some(orientation) = orientation.map(unsplash_orientation) {
        request = request.query(&[("orientation", orientation)]);
    }
    let payload = get_json(request, "Unsplash").await?;
    Ok(parse_unsplash(&payload))
}

async fn pexels(
    client: &Client,
    key: &str,
    query: &str,
    count: usize,
    orientation: Option<&str>,
) -> Result<Vec<StockHit>, CopilotError> {
    let mut request = client
        .get("https://api.pexels.com/v1/search")
        .query(&[("query", query), ("per_page", &count.to_string())])
        .header("Authorization", key);
    if let Some(orientation) = orientation {
        request = request.query(&[("orientation", orientation)]);
    }
    let payload = get_json(request, "Pexels").await?;
    Ok(parse_pexels(&payload))
}

async fn vecteezy(
    client: &Client,
    key: &str,
    account: &str,
    query: &str,
    count: usize,
    orientation: Option<&str>,
) -> Result<Vec<StockHit>, CopilotError> {
    // The search endpoint lives under /v2/account/{id}/resources. The shorter
    // /v2/{id}/resources path fetches a known resource by id and would ignore
    // the query, so it cannot power a search.
    let url = vecteezy_search_url(account);
    let mut request = client
        .get(&url)
        .query(&[
            ("term", query),
            ("content_type", "vector"),
            ("per_page", &count.to_string()),
        ])
        .bearer_auth(key);
    if let Some(orientation) = orientation.map(vecteezy_orientation) {
        request = request.query(&[("orientation", orientation)]);
    }
    let payload = get_json(request, "Vecteezy").await?;
    Ok(parse_vecteezy(&payload))
}

fn parse_unsplash(payload: &Value) -> Vec<StockHit> {
    payload
        .get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item
                        .pointer("/urls/regular")
                        .and_then(Value::as_str)?
                        .to_string();
                    Some(StockHit {
                        provider: "unsplash",
                        url,
                        width: item.get("width").and_then(Value::as_u64),
                        height: item.get("height").and_then(Value::as_u64),
                        author: item
                            .pointer("/user/name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        page_url: item
                            .pointer("/links/html")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        attribution: "Photo from Unsplash".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_pexels(payload: &Value) -> Vec<StockHit> {
    payload
        .get("photos")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let url = item
                        .pointer("/src/large2x")
                        .or_else(|| item.pointer("/src/large"))
                        .and_then(Value::as_str)?
                        .to_string();
                    Some(StockHit {
                        provider: "pexels",
                        url,
                        width: item.get("width").and_then(Value::as_u64),
                        height: item.get("height").and_then(Value::as_u64),
                        author: item
                            .get("photographer")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        page_url: item.get("url").and_then(Value::as_str).map(str::to_string),
                        attribution: "Photo from Pexels".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_vecteezy(payload: &Value) -> Vec<StockHit> {
    payload
        .get("resources")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let preview = item
                        .get("preview_url")
                        .and_then(Value::as_str)
                        .filter(|url| !is_svg_url(url));
                    let thumbnail = item
                        .get("thumbnail_url")
                        .and_then(Value::as_str)
                        .filter(|url| !is_svg_url(url));
                    let url = preview.or(thumbnail)?.to_string();
                    let id = item
                        .get("id")
                        .and_then(|id| {
                            id.as_u64()
                                .map(|value| value.to_string())
                                .or_else(|| id.as_str().map(str::to_string))
                        })
                        .unwrap_or_default();
                    // The search response nests the source dimensions under
                    // `dimensions`; older payloads put them at the top level.
                    let width = item
                        .get("width")
                        .and_then(Value::as_u64)
                        .or_else(|| item.pointer("/dimensions/width").and_then(Value::as_u64));
                    let height = item
                        .get("height")
                        .and_then(Value::as_u64)
                        .or_else(|| item.pointer("/dimensions/height").and_then(Value::as_u64));
                    Some(StockHit {
                        provider: "vecteezy",
                        url,
                        width,
                        height,
                        author: item
                            .get("author")
                            .or_else(|| item.get("byline"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        page_url: if id.is_empty() {
                            None
                        } else {
                            Some(format!("https://www.vecteezy.com/vector-art/{id}"))
                        },
                        attribution: "Vector from Vecteezy".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The asset store deliberately refuses SVG (it is an active document), so a
/// vector search must hand the model a raster preview instead of the source
/// file. Ignore obvious SVG links and let the next candidate fill the slot.
fn is_svg_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_ascii_lowercase().ends_with(".svg")
}

/// Unsplash spells the square crop "squarish"; its API ignores "square".
fn unsplash_orientation(value: &str) -> &str {
    match value {
        "square" => "squarish",
        other => other,
    }
}

/// Vecteezy names the same axis horizontal/vertical.
fn vecteezy_orientation(value: &str) -> &str {
    match value {
        "landscape" => "horizontal",
        "portrait" => "vertical",
        other => other,
    }
}

fn vecteezy_search_url(account: &str) -> String {
    format!(
        "https://api.vecteezy.com/v2/account/{}/resources",
        encode_path(account)
    )
}

fn encode_path(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsplash_results_become_direct_urls_with_credit() {
        let hits = parse_unsplash(&serde_json::json!({
            "results": [{
                "width": 1600,
                "height": 900,
                "urls": { "regular": "https://images.unsplash.com/photo-1?w=1080" },
                "links": { "html": "https://unsplash.com/photos/1" },
                "user": { "name": "Ada" }
            }]
        }));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].provider, "unsplash");
        assert_eq!(hits[0].url, "https://images.unsplash.com/photo-1?w=1080");
        assert_eq!(hits[0].author.as_deref(), Some("Ada"));
        assert!(hits[0].attribution.contains("Unsplash"));
    }

    #[test]
    fn pexels_results_prefer_the_large_variant() {
        let hits = parse_pexels(&serde_json::json!({
            "photos": [{
                "width": 1200,
                "height": 800,
                "photographer": "Grace",
                "url": "https://www.pexels.com/photo/1",
                "src": {
                    "large": "https://images.pexels.com/photo-1-large.jpg",
                    "large2x": "https://images.pexels.com/photo-1-2x.jpg"
                }
            }]
        }));
        assert_eq!(hits[0].url, "https://images.pexels.com/photo-1-2x.jpg");
        assert_eq!(hits[0].author.as_deref(), Some("Grace"));
    }

    #[test]
    fn vecteezy_results_are_parsed_and_vectors_are_supported() {
        let hits = parse_vecteezy(&serde_json::json!({
            "resources": [{
                "id": 42,
                "dimensions": { "width": 800, "height": 600 },
                "preview_url": "https://cdn.vecteezy.com/42.jpg",
                "thumbnail_url": "https://cdn.vecteezy.com/42-thumb.jpg",
                "byline": "Lin"
            }]
        }));
        assert_eq!(hits[0].provider, "vecteezy");
        assert_eq!(hits[0].url, "https://cdn.vecteezy.com/42.jpg");
        assert_eq!(hits[0].width, Some(800));
        assert_eq!(hits[0].height, Some(600));
        assert_eq!(hits[0].author.as_deref(), Some("Lin"));
        assert_eq!(
            hits[0].page_url.as_deref(),
            Some("https://www.vecteezy.com/vector-art/42")
        );
        assert!(hits[0].attribution.contains("Vecteezy"));
        assert_eq!(StockKind::parse("vector"), Some(StockKind::Vector));
        assert_eq!(StockKind::parse("nonsense"), None);
    }

    #[test]
    fn vecteezy_skips_the_svg_source_and_uses_a_raster_preview() {
        let hits = parse_vecteezy(&serde_json::json!({
            "resources": [
                {
                    "id": 1,
                    "preview_url": "https://cdn.vecteezy.com/1.svg",
                    "thumbnail_url": "https://cdn.vecteezy.com/1-thumb.jpg"
                },
                {
                    "id": 2,
                    "preview_url": "https://cdn.vecteezy.com/2.svg?v=3"
                }
            ]
        }));
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].page_url.as_deref(),
            Some("https://www.vecteezy.com/vector-art/1")
        );
        assert!(hits[0].url.ends_with(".jpg"));
    }

    #[test]
    fn the_search_endpoints_and_orientation_vocabulary_match_each_vendor() {
        assert_eq!(
            vecteezy_search_url("123/../abc"),
            "https://api.vecteezy.com/v2/account/123abc/resources"
        );
        assert_eq!(unsplash_orientation("square"), "squarish");
        assert_eq!(unsplash_orientation("landscape"), "landscape");
        assert_eq!(vecteezy_orientation("landscape"), "horizontal");
        assert_eq!(vecteezy_orientation("portrait"), "vertical");
        assert_eq!(vecteezy_orientation("square"), "square");
    }

    #[test]
    fn stored_keys_override_the_environment_and_need_both_vecteezy_fields() {
        let keys = StockKeys::from_lookup(|id| match id {
            UNSPLASH_KEY => Some("unsplash-stored".into()),
            VECTEEZY_KEY => Some("vecteezy-stored".into()),
            _ => None,
        });
        let providers = keys.providers();
        assert!(providers.contains(&"unsplash"));
        assert!(
            !providers.contains(&"vecteezy"),
            "a Vecteezy key without its account id is not usable"
        );
        assert_eq!(keys.unsplash.as_deref(), Some("unsplash-stored"));

        let keys = StockKeys::from_lookup(|id| match id {
            VECTEEZY_KEY => Some("  vecteezy-stored  ".into()),
            VECTEEZY_ACCOUNT => Some(" acct_9 ".into()),
            _ => Some("".into()),
        });
        assert!(keys.providers().contains(&"vecteezy"));
        assert_eq!(keys.vecteezy.as_deref(), Some("vecteezy-stored"));
        assert_eq!(keys.vecteezy_account.as_deref(), Some("acct_9"));
    }

    #[test]
    fn path_encoding_never_lets_an_account_id_escape_the_route() {
        assert_eq!(encode_path("123/../abc"), "123abc");
        assert_eq!(encode_path("acct_9"), "acct_9");
    }
}
