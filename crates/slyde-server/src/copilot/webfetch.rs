//! Reads a web page or an image from a link the user pastes into the copilot.
//!
//! The server fetches it, not the browser, so the studio page never makes a
//! cross-origin request and the result can be bounded. A link is untrusted input
//! that names a machine to connect to, so every hop is checked before a socket is
//! opened: only `http`/`https`, only the standard ports, and never an address
//! that points back at this computer or its network (loopback, private ranges,
//! link-local, carrier-grade NAT, multicast). The validated address is pinned for
//! the request, so a name that resolves differently a second later cannot slip
//! through, and redirects are followed by hand so each hop is validated too.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::Url;
use serde::Serialize;

const TIMEOUT: Duration = Duration::from_secs(12);
const MAX_HOPS: usize = 4;
const MAX_PAGE_BYTES: usize = 3 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// Text handed to the model: enough for an article, never a whole book.
const MAX_TEXT_CHARS: usize = 48_000;
const USER_AGENT: &str =
    "Mozilla/5.0 (compatible; SlydeCode/0.1; +https://github.com/Eddym06/SlydeCode)";

/// What a link turned out to be.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fetched {
    /// A picture, as base64 so it travels like an uploaded image.
    Image {
        mime: String,
        name: String,
        data: String,
    },
    /// A readable page, reduced to plain text.
    Page {
        title: String,
        text: String,
        url: String,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FetchError {
    #[error("that is not a valid web link")]
    InvalidUrl,
    #[error("only http and https links on the standard ports can be read")]
    Blocked,
    #[error("the page could not be reached")]
    Unreachable,
    #[error("the page redirected too many times")]
    TooManyRedirects,
    #[error("the page is larger than the copilot can read")]
    TooLarge,
    #[error("only web pages and PNG, JPEG or WebP images can be read")]
    Unsupported,
}

/// Fetches `link`, following at most [`MAX_HOPS`] validated redirects.
pub async fn fetch(link: &str) -> Result<Fetched, FetchError> {
    let mut url = Url::parse(link.trim()).map_err(|_| FetchError::InvalidUrl)?;
    for _ in 0..=MAX_HOPS {
        let address = validate(&url).await?;
        let host = url.host_str().ok_or(FetchError::InvalidUrl)?.to_string();
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, address)
            .build()
            .map_err(|_| FetchError::Unreachable)?;
        let response = client
            .get(url.clone())
            .header(
                reqwest::header::ACCEPT,
                "text/html,text/plain,image/png,image/jpeg,image/webp;q=0.9,*/*;q=0.1",
            )
            .send()
            .await
            .map_err(|_| FetchError::Unreachable)?;

        if response.status().is_redirection() {
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| url.join(value).ok())
                .ok_or(FetchError::Unreachable)?;
            url = next;
            continue;
        }
        if !response.status().is_success() {
            return Err(FetchError::Unreachable);
        }
        return read_body(url, response).await;
    }
    Err(FetchError::TooManyRedirects)
}

async fn read_body(url: Url, mut response: reqwest::Response) -> Result<Fetched, FetchError> {
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or("").trim().to_lowercase())
        .unwrap_or_default();
    let is_image = matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/webp");
    let is_text = mime.is_empty() || mime.starts_with("text/") || mime == "application/xhtml+xml";
    if !is_image && !is_text {
        return Err(FetchError::Unsupported);
    }

    let limit = if is_image {
        MAX_IMAGE_BYTES
    } else {
        MAX_PAGE_BYTES
    };
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| FetchError::Unreachable)?
    {
        if body.len() + chunk.len() > limit {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    if is_image {
        let name = url
            .path_segments()
            .and_then(|mut parts| parts.next_back())
            .filter(|name| !name.is_empty())
            .unwrap_or("image")
            .to_string();
        return Ok(Fetched::Image {
            mime,
            name,
            data: STANDARD.encode(&body),
        });
    }

    let html = String::from_utf8_lossy(&body);
    let (title, text) = readable(&html);
    let title = if title.is_empty() {
        url.host_str().unwrap_or("page").to_string()
    } else {
        title
    };
    Ok(Fetched::Page {
        title,
        text,
        url: url.to_string(),
    })
}

/// Checks a URL and returns the one address the request may connect to.
async fn validate(url: &Url) -> Result<SocketAddr, FetchError> {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() {
        return Err(FetchError::Blocked);
    }
    let port = url.port_or_known_default().ok_or(FetchError::Blocked)?;
    if !matches!(port, 80 | 443) {
        return Err(FetchError::Blocked);
    }
    let host = url.host_str().ok_or(FetchError::InvalidUrl)?;
    let candidates: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| FetchError::Unreachable)?
        .collect();
    if candidates.is_empty() {
        return Err(FetchError::Unreachable);
    }
    // One private answer taints the name: refuse rather than pick around it.
    if candidates.iter().any(|addr| !is_public(addr.ip())) {
        return Err(FetchError::Blocked);
    }
    Ok(candidates[0])
}

/// True for an address that belongs to the public internet.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_v4(mapped);
            }
            is_public_v6(v6)
        }
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 100.64.0.0/10 carrier-grade NAT, 0.0.0.0/8, 192.0.0.0/24, 198.18.0.0/15, 240.0.0.0/4
        || (a == 100 && (64..128).contains(&b))
        || a == 0
        || (a == 192 && b == 0)
        || (a == 198 && (18..20).contains(&b))
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (first & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (first == 0x2001 && ip.segments()[1] == 0x0db8)) // documentation
}

/// Title and visible text of an HTML document, bounded.
fn readable(html: &str) -> (String, String) {
    let title = tag_text(html, "title");
    let mut text = String::with_capacity(html.len().min(MAX_TEXT_CHARS * 2));
    let mut rest = html;
    loop {
        let Some(open) = rest.find('<') else {
            text.push_str(rest);
            break;
        };
        text.push_str(&rest[..open]);
        rest = &rest[open..];
        let lower = rest.get(..9).unwrap_or(rest).to_ascii_lowercase();
        let skipped = ["script", "style", "noscript", "svg", "template", "head"]
            .into_iter()
            .find(|name| lower.starts_with(&format!("<{name}")));
        if let Some(name) = skipped {
            let close = format!("</{name}");
            let after = rest.to_ascii_lowercase().find(&close);
            match after.and_then(|at| rest[at..].find('>').map(|end| at + end + 1)) {
                Some(end) => rest = &rest[end..],
                None => break,
            }
            continue;
        }
        match rest.find('>') {
            Some(end) => {
                // Block-level boundaries become line breaks so paragraphs survive.
                let tag = rest[1..end].trim_start_matches('/').to_ascii_lowercase();
                let name = tag.split(|c: char| c.is_whitespace() || c == '/').next();
                if matches!(
                    name,
                    Some(
                        "p" | "br"
                            | "div"
                            | "li"
                            | "tr"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "section"
                            | "article"
                            | "blockquote"
                    )
                ) {
                    text.push('\n');
                }
                rest = &rest[end + 1..];
            }
            None => break,
        }
    }
    (decode(&title), tidy(&decode(&text)))
}

fn tag_text(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(open) = lower.find(&format!("<{tag}")) else {
        return String::new();
    };
    let Some(start) = lower[open..].find('>').map(|at| open + at + 1) else {
        return String::new();
    };
    let Some(end) = lower[start..]
        .find(&format!("</{tag}"))
        .map(|at| start + at)
    else {
        return String::new();
    };
    html[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Collapses runs of spaces and blank lines, and applies the text budget.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(&line);
        if out.chars().count() > MAX_TEXT_CHARS {
            break;
        }
    }
    out.chars().take(MAX_TEXT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn addresses_that_point_back_at_this_machine_or_its_network_are_refused() {
        for blocked in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.9",
            "192.168.1.20",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public(ip(blocked)), "{blocked} must be refused");
        }
    }

    #[test]
    fn ordinary_internet_addresses_are_allowed() {
        for allowed in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(is_public(ip(allowed)), "{allowed} must be allowed");
        }
    }

    #[tokio::test]
    async fn only_plain_web_links_on_standard_ports_are_fetched() {
        for (link, error) in [
            ("file:///etc/passwd", FetchError::Blocked),
            ("ftp://example.com/a", FetchError::Blocked),
            ("http://127.0.0.1/", FetchError::Blocked),
            ("http://localhost/", FetchError::Blocked),
            (
                "http://169.254.169.254/latest/meta-data",
                FetchError::Blocked,
            ),
            ("http://[::1]/", FetchError::Blocked),
            ("https://user:pass@example.com/", FetchError::Blocked),
            ("http://example.com:22/", FetchError::Blocked),
            ("not a url", FetchError::InvalidUrl),
        ] {
            assert_eq!(fetch(link).await.unwrap_err(), error, "{link}");
        }
    }

    #[test]
    fn a_page_becomes_its_visible_text_without_scripts_or_styles() {
        let html = "<html><head><title> My &amp; Page </title><style>p{color:red}</style></head>\
            <body><script>alert(1)</script><h1>Hello</h1><p>First   paragraph</p>\
            <p>Second &lt;one&gt;</p><noscript>enable js</noscript></body></html>";
        let (title, text) = readable(html);
        assert_eq!(title, "My & Page");
        assert!(text.contains("Hello"));
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Second <one>"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("color:red"));
        assert!(!text.contains("enable js"));
    }

    #[test]
    fn the_text_handed_to_the_model_is_bounded() {
        let html = format!("<p>{}</p>", "word ".repeat(MAX_TEXT_CHARS));
        let (_, text) = readable(&html);
        assert!(text.chars().count() <= MAX_TEXT_CHARS);
        assert!(!text.is_empty());
    }
}
