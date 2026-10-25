//! SlydeCode MCP server over stdio.
//!
//! JSON-RPC 2.0, newline-delimited, on stdin/stdout. The server is deliberately
//! thin: every tool call is forwarded to the running studio over loopback HTTP,
//! so the MCP process and the UI can never disagree about the deck.
//!
//! If no studio is running, the MCP server launches one (which opens Chrome)
//! and waits for it to become healthy.
//!
//! Requests are dispatched concurrently: the reader never awaits a handler, so a
//! slow `tools/call` cannot stall a `ping` behind it. Responses are matched by
//! `id`, so they may leave out of order, but a single writer task owns stdout and
//! emits exactly one line per response.

pub mod tools;

use std::future::Future;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use slyde_server::{RuntimeInfo, probe_health, read_runtime};
use thiserror::Error;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

/// Protocol revisions this server can speak, newest first.
const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const STARTUP_TIMEOUT: Duration = Duration::from_secs(25);
/// Upper bound on requests processed at the same time. The reader waits for a
/// permit when the limit is reached, so a client cannot make the server spawn
/// unbounded work; requests already in flight still finish.
const MAX_IN_FLIGHT_REQUESTS: usize = 8;
/// Bound for each shutdown phase: draining in-flight requests, then letting the
/// writer flush and exit.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

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

    let stdin = BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    run_reader(
        stdin,
        stdout,
        MAX_IN_FLIGHT_REQUESTS,
        move |line: String| {
            let runtime = runtime.clone();
            async move { handle_line(&runtime, &line).await }
        },
    )
    .await
}

/// Runs the JSON-RPC loop over an arbitrary reader/writer pair.
///
/// Every non-empty line is handed to `handler` on its own task, so a slow
/// request never blocks reading the next line. `handler` returns the response
/// to send, or `None` for notifications and client responses, which are never
/// answered. Responses are matched to requests by `id`, not by order.
///
/// `max_in_flight` bounds concurrent work: at the limit the reader waits for a
/// permit instead of piling up tasks, while everything in flight keeps running.
/// Responses are funneled to a single writer task, so bytes are never
/// interleaved and every response is written as one encoded line plus `\n`.
/// When the reader ends, no new work is accepted, in-flight tasks are drained
/// (bounded by [`SHUTDOWN_TIMEOUT`]), and only then does stdout close.
async fn run_reader<R, W, H, Fut>(
    reader: R,
    writer: W,
    max_in_flight: usize,
    handler: H,
) -> Result<(), McpError>
where
    R: AsyncBufRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
    H: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Option<Value>> + Send + 'static,
{
    let capacity = max_in_flight.max(1);
    let (responses_tx, responses_rx) = mpsc::channel::<Vec<u8>>(capacity);
    let mut writer_task = tokio::spawn(write_responses(writer, responses_rx));

    let permits = Arc::new(Semaphore::new(capacity));
    let mut tasks: JoinSet<()> = JoinSet::new();
    let mut lines = reader.lines();
    let mut read_error = None;

    loop {
        // Reap finished tasks as we go so a long-lived session cannot accumulate
        // task slots, and surface a handler panic instead of losing it.
        while let Some(joined) = tasks.try_join_next() {
            if let Err(err) = joined {
                tracing::error!(%err, "an MCP request task failed");
            }
        }

        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                read_error = Some(McpError::Io(err));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        // Waiting for a permit here is also the ordering guarantee: we never
        // spawn request N+1 before request N has been dispatched, even when the
        // bounded pool is saturated.
        let Ok(permit) = permits.clone().acquire_owned().await else {
            break;
        };

        let handler = handler.clone();
        let responses_tx = responses_tx.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let Some(response) = handler(line).await else {
                return;
            };
            let encoded = match serde_json::to_vec(&response) {
                Ok(encoded) => encoded,
                Err(err) => {
                    tracing::error!(%err, "could not encode an MCP response");
                    return;
                }
            };
            if responses_tx.send(encoded).await.is_err() {
                tracing::warn!("the MCP writer stopped before a response could be queued");
            }
        });
    }

    // Stop accepting work: once the last in-flight task drops its sender, the
    // writer sees the channel close, flushes and exits.
    drop(responses_tx);

    let drain = async {
        while let Some(joined) = tasks.join_next().await {
            if let Err(err) = joined {
                tracing::error!(%err, "an MCP request task failed");
            }
        }
    };
    if tokio::time::timeout(SHUTDOWN_TIMEOUT, drain).await.is_err() {
        tracing::warn!(
            timeout_secs = SHUTDOWN_TIMEOUT.as_secs(),
            "in-flight MCP requests did not finish in time; aborting them"
        );
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }

    match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut writer_task).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(err))) => return Err(McpError::Io(err)),
        Ok(Err(err)) => {
            return Err(McpError::Io(std::io::Error::other(err.to_string())));
        }
        Err(_) => {
            writer_task.abort();
            return Err(McpError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the MCP response writer did not finish in time",
            )));
        }
    }

    match read_error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// Single owner of the output stream. It receives fully encoded responses and
/// writes each as one line plus `\n`, flushing every time so a client never
/// waits on a buffered response.
async fn write_responses<W>(
    writer: W,
    mut responses: mpsc::Receiver<Vec<u8>>,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut writer = BufWriter::new(writer);
    while let Some(response) = responses.recv().await {
        writer.write_all(&response).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
    writer.flush().await?;
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
    use std::collections::HashMap;

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

    fn request(id: u64, method: &str) -> String {
        json!({ "jsonrpc": "2.0", "id": id, "method": method }).to_string()
    }

    /// Feeds `requests` through the real loop with an injected handler over an
    /// in-memory pipe, then returns every response line in write order.
    async fn drive<H, Fut>(handler: H, max_in_flight: usize, requests: Vec<String>) -> Vec<String>
    where
        H: Fn(String) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = Option<Value>> + Send + 'static,
    {
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (server_reader, server_writer) = tokio::io::split(server);
        let loop_task = tokio::spawn(run_reader(
            BufReader::new(server_reader),
            server_writer,
            max_in_flight,
            handler,
        ));

        for line in requests {
            client.write_all(line.as_bytes()).await.unwrap();
            client.write_all(b"\n").await.unwrap();
        }
        client.flush().await.unwrap();
        // Closing our write side is what the real process sees as stdin EOF.
        client.shutdown().await.unwrap();

        let mut reader = BufReader::new(client);
        let mut responses = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await.unwrap() {
                0 => break,
                _ => responses.push(line.trim_end_matches(&['\n', '\r'][..]).to_string()),
            }
        }

        loop_task.await.unwrap().unwrap();
        responses
    }

    #[tokio::test]
    async fn initialize_negotiates_a_supported_protocol() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(response["result"]["serverInfo"]["name"], "slydecode");
        assert_eq!(
            response["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
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
    async fn tools_list_exposes_the_tools() {
        let line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let response = handle_line(&runtime(), line).await.unwrap();
        assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 11);
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
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("unknown tool")
        );
    }

    /// The core promise of the concurrent loop: a request that takes 300 ms
    /// must not delay a request that is ready immediately, even though the slow
    /// one was dispatched first.
    #[tokio::test]
    async fn a_fast_request_overtakes_a_slow_one() {
        let handler = |line: String| async move {
            let message: Value = match serde_json::from_str(&line) {
                Ok(message) => message,
                Err(_) => return None,
            };
            let method = message.get("method").and_then(Value::as_str)?.to_string();
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            if method == "slow" {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "method": method }
            }))
        };

        let responses = drive(
            handler,
            MAX_IN_FLIGHT_REQUESTS,
            vec![request(1, "slow"), request(2, "fast")],
        )
        .await;

        assert_eq!(responses.len(), 2);
        for response in &responses {
            let value: Value =
                serde_json::from_str(response).expect("every response line must be valid JSON");
            assert_eq!(value["jsonrpc"], "2.0");
            assert!(
                value["id"].is_number(),
                "every response keeps its request id"
            );
        }

        let first: Value = serde_json::from_str(&responses[0]).unwrap();
        let second: Value = serde_json::from_str(&responses[1]).unwrap();
        assert_eq!(
            first["id"], 2,
            "the fast response must be written before the slow one"
        );
        assert_eq!(first["result"]["method"], "fast");
        assert_eq!(second["id"], 1);
        assert_eq!(second["result"]["method"], "slow");
    }

    /// Twenty pipelined requests, with the in-flight limit deliberately smaller
    /// than the batch: the reader has to wait for permits, and every response
    /// still comes back as its own valid JSON line carrying the right id.
    #[tokio::test]
    async fn twenty_interleaved_requests_get_one_json_line_each() {
        let handler = |line: String| async move {
            let message: Value = match serde_json::from_str(&line) {
                Ok(message) => message,
                Err(_) => return None,
            };
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            // Yield so several handlers are genuinely in flight at once.
            tokio::task::yield_now().await;
            Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "ok": true }
            }))
        };

        let requests: Vec<String> = (0..20).map(|id| request(id, "ping")).collect();
        let responses = drive(handler, 4, requests).await;

        assert_eq!(responses.len(), 20);
        let mut seen = Vec::new();
        for response in &responses {
            assert!(
                !response.contains('\n'),
                "a response must be exactly one line: {response:?}"
            );
            let value: Value =
                serde_json::from_str(response).expect("every response line must be valid JSON");
            assert_eq!(value["jsonrpc"], "2.0");
            seen.push(
                value["id"]
                    .as_u64()
                    .expect("every response must carry the request id"),
            );
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..20).collect::<Vec<u64>>());
    }

    /// End to end over the loop with the real dispatcher: no response for
    /// notifications, parse errors keep -32700, unknown methods keep -32601,
    /// and stdout carries nothing but response lines.
    #[tokio::test]
    async fn the_loop_keeps_the_documented_dispatch_behaviour() {
        let handler = move |line: String| {
            let runtime = runtime();
            async move { handle_line(&runtime, &line).await }
        };

        let responses = drive(
            handler,
            MAX_IN_FLIGHT_REQUESTS,
            vec![
                r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_string(),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string(),
                "{not json".to_string(),
                r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#.to_string(),
            ],
        )
        .await;

        assert_eq!(
            responses.len(),
            3,
            "the notification must not produce a response line"
        );

        let mut by_id: HashMap<String, Value> = HashMap::new();
        for response in &responses {
            let value: Value =
                serde_json::from_str(response).expect("every response line must be valid JSON");
            by_id.insert(value["id"].to_string(), value);
        }
        assert_eq!(by_id["1"]["result"], json!({}));
        assert_eq!(by_id["2"]["error"]["code"], -32601);
        assert_eq!(by_id["null"]["error"]["code"], -32700);
    }
}
