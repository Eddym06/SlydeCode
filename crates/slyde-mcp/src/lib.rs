//! SlydeCode MCP server over stdio.
//!
//! JSON-RPC 2.0, newline-delimited, on stdin/stdout. The server is deliberately
//! thin: every tool call is forwarded to the running studio over loopback HTTP,
//! so the MCP process and the UI can never disagree about the deck.
//!
//! If no studio is running, the MCP server launches one (which opens Chrome)
//! and waits for it to become healthy.

pub mod tools;

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use slyde_server::{RuntimeInfo, probe_health, read_runtime};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

/// Protocol revisions this server can speak, newest first.
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const STARTUP_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Error)]
pub enum McpError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not start the studio: {0}")]
    Startup(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn run_stdio() -> Result<(), McpError> {
    let runtime = ensure_runtime().await?;
    tracing::info!(port = runtime.port, "MCP bridge attached to the studio");

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = BufWriter::new(tokio::io::stdout());

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&runtime, &line).await {
            let encoded = serde_json::to_vec(&response)?;
            stdout.write_all(&encoded).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    Ok(())
}

async fn handle_line(runtime: &RuntimeInfo, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(err) => {
            return Some(error_response(
                Value::Null,
                -32700,
                format!("parse error: {err}"),
            ));
        }
    };

    let id = message.get("id").cloned();
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        // A response from the client to a server-initiated request. We never
        // send those, so there is nothing to do.
        return None;
    };

    // Notifications (no id) never get a response.
    let is_notification = id.is_none();

    match method {
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(SUPPORTED_PROTOCOLS[0]);
            let version = if SUPPORTED_PROTOCOLS.contains(&requested) {
                requested
            } else {
                SUPPORTED_PROTOCOLS[0]
            };
            Some(success(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": "slydecode",
                        "title": "SlydeCode",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "instructions": "SlydeCode writes real slides. Call create_presentation once, then upsert_slide per slide. Slide code is a self-contained HTML fragment rendered at 1280x720 (16:9). Inline <style> is allowed; no network requests."
                }),
            ))
        }

        "notifications/initialized" | "initialized" => None,

        "ping" => Some(success(id, json!({}))),

        "tools/list" => Some(success(id, json!({ "tools": tools::definitions() }))),

        "tools/call" => {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error_response(
                    id.unwrap_or(Value::Null),
                    -32602,
                    "invalid params: missing tool name".into(),
                ));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            Some(match tools::call(runtime, name, &arguments).await {
                Ok(text) => success(id, tool_content(&text, false)),
                Err(tools::ToolFailure(message)) => success(id, tool_content(&message, true)),
            })
        }

        other => {
            if is_notification {
                None
            } else {
                Some(error_response(
                    id.unwrap_or(Value::Null),
                    -32601,
                    format!("method not found: {other}"),
                ))
            }
        }
    }
}

fn tool_content(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn success(id: Option<Value>, result: Value) -> Value {
    match id {
        Some(id) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        None => json!({ "jsonrpc": "2.0", "result": result }),
    }
}

fn error_response(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

/// Returns a healthy runtime, starting the studio if necessary.
async fn ensure_runtime() -> Result<RuntimeInfo, McpError> {
    if let Some(info) = read_runtime() {
        if probe_health(info.port, Duration::from_millis(600)).await {
            return Ok(info);
        }
    }

    let exe = std::env::current_exe()
        .map_err(|err| McpError::Startup(format!("cannot locate the slyde binary: {err}")))?;

    let mut command = tokio::process::Command::new(exe);
    command
        .arg("app")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let _child = command
        .spawn()
        .map_err(|err| McpError::Startup(err.to_string()))?;

    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if let Some(info) = read_runtime() {
            if probe_health(info.port, Duration::from_millis(400)).await {
                return Ok(info);
            }
        }
    }

    Err(McpError::Startup(
        "the studio did not become healthy in time".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> RuntimeInfo {
        RuntimeInfo {
            port: 1,
            sandbox_port: 2,
            token: "t".into(),
            pid: 0,
            started_at_ms: 0,
        }
    }

    #[tokio::test]
    async fn initialize_negotiates_a_supported_protocol() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(response["result"]["serverInfo"]["name"], "slydecode");
        assert_eq!(response["result"]["capabilities"]["tools"]["listChanged"], false);
    }

    #[tokio::test]
    async fn unknown_protocol_falls_back_to_the_newest_supported() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
    }

    #[tokio::test]
    async fn initialized_notification_gets_no_response() {
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_line(&runtime(), line).await.is_none());
    }

    #[tokio::test]
    async fn tools_list_exposes_the_seven_tools() {
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 7);
    }

    #[tokio::test]
    async fn parse_errors_use_the_json_rpc_code() {
        let response = handle_line(&runtime(), "{not json").await.unwrap();
        assert_eq!(response["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn unknown_methods_report_minus_32601() {
        let line = r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn tool_failures_are_results_with_is_error() {
        let line = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown tool"));
    }
}
