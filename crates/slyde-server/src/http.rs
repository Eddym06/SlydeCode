//! Tiny HTTP/1.1 client for `127.0.0.1`.
//!
//! Used by the MCP bridge and the export pipeline to talk to the running
//! studio. A full HTTP stack (reqwest + TLS) is not worth the dependency for
//! loopback JSON requests.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("http {status}: {body}")]
    Status { status: u16, body: String },
}

pub async fn get_json(url: &str, timeout: Duration) -> Result<serde_json::Value, HttpError> {
    request("GET", url, None, timeout).await
}

pub async fn post_json(
    url: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, HttpError> {
    let encoded =
        serde_json::to_vec(body).map_err(|e| HttpError::InvalidResponse(e.to_string()))?;
    request("POST", url, Some(encoded), timeout).await
}

/// POSTs with an empty body and returns the response bytes verbatim.
///
/// The export endpoint answers with a file rather than JSON, so the CLI cannot
/// go through `post_json`. A non-2xx still carries JSON, so the error keeps the
/// server's message.
pub async fn post_bytes(url: &str, timeout: Duration) -> Result<Vec<u8>, HttpError> {
    let (status, body) = raw_request("POST", url, None, timeout).await?;
    if !(200..300).contains(&status) {
        return Err(HttpError::Status {
            status,
            body: String::from_utf8_lossy(&body).to_string(),
        });
    }
    Ok(body)
}

async fn request(
    method: &str,
    url: &str,
    body: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<serde_json::Value, HttpError> {
    let (status, body) = raw_request(method, url, body, timeout).await?;
    if !(200..300).contains(&status) {
        return Err(HttpError::Status {
            status,
            body: String::from_utf8_lossy(&body).to_string(),
        });
    }
    serde_json::from_slice(&body).map_err(|e| HttpError::InvalidResponse(e.to_string()))
}

/// Performs one HTTP/1.1 request and returns the status and the body bytes.
async fn raw_request(
    method: &str,
    url: &str,
    body: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), HttpError> {
    let parsed = Url::parse(url)?;

    let attempt = async {
        let mut stream = TcpStream::connect((parsed.host.as_str(), parsed.port))
            .await
            .map_err(|e| HttpError::Network(e.to_string()))?;

        let mut request = format!(
            "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept: application/json\r\n",
            parsed.path, parsed.host, parsed.port
        );
        if let Some(payload) = &body {
            request.push_str("Content-Type: application/json\r\n");
            request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
        }
        request.push_str("\r\n");

        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| HttpError::Network(e.to_string()))?;
        if let Some(payload) = &body {
            stream
                .write_all(payload)
                .await
                .map_err(|e| HttpError::Network(e.to_string()))?;
        }
        stream.flush().await.ok();

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .await
            .map_err(|e| HttpError::Network(e.to_string()))?;

        split_response(&raw)
    };

    match tokio::time::timeout(timeout, attempt).await {
        Ok(result) => result,
        Err(_) => Err(HttpError::Network("request timed out".into())),
    }
}

/// Splits a raw HTTP response into its status code and body bytes.
fn split_response(raw: &[u8]) -> Result<(u16, Vec<u8>), HttpError> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| HttpError::InvalidResponse("missing header terminator".into()))?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = raw[split + 4..].to_vec();

    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| HttpError::InvalidResponse("missing status line".into()))?;

    Ok((status, body))
}

struct Url {
    host: String,
    port: u16,
    path: String,
}

impl Url {
    fn parse(raw: &str) -> Result<Self, HttpError> {
        let rest = raw
            .strip_prefix("http://")
            .ok_or_else(|| HttpError::InvalidUrl(raw.to_string()))?;
        let (authority, path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse::<u16>()
                    .map_err(|_| HttpError::InvalidUrl(raw.to_string()))?,
            ),
            None => (authority.to_string(), 80),
        };
        if host.is_empty() {
            return Err(HttpError::InvalidUrl(raw.to_string()));
        }
        Ok(Self {
            host,
            port,
            path: path.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_parsing_handles_ports_and_paths() {
        let url = Url::parse("http://127.0.0.1:4321/api/state?t=abc").unwrap();
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 4321);
        assert_eq!(url.path, "/api/state?t=abc");
    }

    #[test]
    fn url_parsing_rejects_https() {
        assert!(matches!(
            Url::parse("https://example.com"),
            Err(HttpError::InvalidUrl(_))
        ));
    }

    #[test]
    fn response_parsing_splits_headers_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}";
        let (status, body) = split_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, b"{\"ok\":true}");
    }

    #[test]
    fn binary_bodies_survive_untouched() {
        // A PPTX starts with the ZIP signature; the client must not decode it.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/vnd.openxmlformats\r\n\r\nPK\x03\x04binary";
        let (status, body) = split_response(raw).unwrap();
        assert_eq!(status, 200);
        assert_eq!(&body[..4], b"PK\x03\x04");
    }

    #[test]
    fn a_missing_status_line_is_an_error() {
        assert!(matches!(
            split_response(b"garbage\r\n\r\n"),
            Err(HttpError::InvalidResponse(_))
        ));
    }
}
