//! MCP Stdio transport: line-delimited JSON-RPC over stdin/stdout.
//!
//! Reads JSON-RPC messages from stdin, dispatches them through [`McpServer`],
//! and writes responses to stdout. Follows the same pattern as ACP stdio.

use std::sync::Arc;

use mcp::protocol::{
    ClientInbound, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ServerOutbound,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use awaken_runtime::AgentRuntime;

/// Run the MCP stdio server on actual stdin/stdout.
pub async fn serve_stdio(runtime: Arc<AgentRuntime>) {
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    serve_stdio_io(runtime, stdin, stdout).await;
}

/// Run the MCP stdio server with injectable I/O (for testing).
pub async fn serve_stdio_io<R, W>(runtime: Arc<AgentRuntime>, input: R, mut output: W)
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let (_server, mut channels) = super::create_mcp_server(&runtime);
    let mut lines = input.lines();
    let mut pending_requests: usize = 0;
    let mut input_done = false;

    loop {
        if input_done && pending_requests == 0 {
            break;
        }

        tokio::select! {
            // Only read input if we haven't reached EOF.
            line_result = lines.next_line(), if !input_done => {
                match line_result {
                    Ok(Some(line)) => {
                        let line = line.trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        match handle_line(&line, &channels.inbound_tx).await {
                            Ok(is_request) => {
                                if is_request {
                                    pending_requests += 1;
                                }
                            }
                            Err(e) => {
                                let resp = make_error_response(None, -32700, &e);
                                let _ = write_line(&mut output, &resp).await;
                            }
                        }
                    }
                    Ok(None) => {
                        input_done = true;
                    }
                    Err(_) => {
                        input_done = true;
                    }
                }
            }
            outbound = channels.outbound_rx.recv() => {
                match outbound {
                    Some(msg) => {
                        let line = match &msg {
                            ServerOutbound::Response(resp) => {
                                pending_requests = pending_requests.saturating_sub(1);
                                serde_json::to_string(resp).ok()
                            }
                            ServerOutbound::Notification(notif) => {
                                serde_json::to_string(notif).ok()
                            }
                            ServerOutbound::Request(req) => {
                                serde_json::to_string(req).ok()
                            }
                        };
                        if let Some(line) = line {
                            let _ = write_line(&mut output, &line).await;
                        }
                    }
                    None => break, // Server shut down.
                }
            }
        }
    }
}

/// Parse and dispatch a single JSON-RPC line to the MCP server.
///
/// Returns `Ok(true)` if a request was sent (expecting a response),
/// `Ok(false)` for notifications/responses.
async fn handle_line(
    line: &str,
    inbound_tx: &tokio::sync::mpsc::Sender<ClientInbound>,
) -> Result<bool, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| format!("parse error: {e}"))?;

    let method = value
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    if method.is_empty() {
        // Could be a response to a server-initiated request.
        if value.get("result").is_some() || value.get("error").is_some() {
            let resp: JsonRpcResponse =
                serde_json::from_value(value).map_err(|e| format!("invalid response: {e}"))?;
            inbound_tx
                .send(ClientInbound::Response(resp))
                .await
                .map_err(|_| "channel closed".to_string())?;
            return Ok(false);
        }
        return Err("missing 'method' field".into());
    }

    let has_id = value.get("id").is_some_and(|v| !v.is_null());

    if has_id {
        let request: JsonRpcRequest =
            serde_json::from_value(value).map_err(|e| format!("invalid request: {e}"))?;
        inbound_tx
            .send(ClientInbound::Request(request))
            .await
            .map_err(|_| "channel closed".to_string())?;
        Ok(true)
    } else {
        let notification: JsonRpcNotification =
            serde_json::from_value(value).map_err(|e| format!("invalid notification: {e}"))?;
        inbound_tx
            .send(ClientInbound::Notification(notification))
            .await
            .map_err(|_| "channel closed".to_string())?;
        Ok(false)
    }
}

fn make_error_response(id: Option<Value>, code: i64, message: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": code,
            "message": message
        },
        "id": id
    })
    .to_string()
}

async fn write_line<W: AsyncWriteExt + Unpin>(writer: &mut W, line: &str) -> std::io::Result<()> {
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_runtime::{AgentResolver, AgentRuntime, ResolvedAgent, RuntimeError};

    struct StubResolver;
    impl AgentResolver for StubResolver {
        fn resolve(&self, agent_id: &str) -> Result<ResolvedAgent, RuntimeError> {
            Err(RuntimeError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })
        }
        fn agent_ids(&self) -> Vec<String> {
            vec!["test-agent".into()]
        }
    }

    fn test_runtime() -> Arc<AgentRuntime> {
        Arc::new(AgentRuntime::new(Arc::new(StubResolver)))
    }

    /// Helper: run stdio with given input, return output string.
    async fn run_stdio(input: &[u8]) -> String {
        let runtime = test_runtime();
        let mut output = Vec::new();
        serve_stdio_io(runtime, input, &mut output).await;
        String::from_utf8(output).unwrap()
    }

    /// Parse first response from output.
    fn first_response(output: &str) -> Value {
        let line = output.lines().next().expect("no output");
        serde_json::from_str(line).expect("invalid JSON")
    }

    #[tokio::test]
    async fn stdio_initialize() {
        let output = run_stdio(b"{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"id\":1}\n").await;

        let resp = first_response(&output);
        assert!(resp["result"]["protocolVersion"].is_string());
        assert_eq!(resp["id"], 1);
    }

    #[tokio::test]
    async fn stdio_tools_list() {
        let output = run_stdio(b"{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"id\":2}\n").await;

        let resp = first_response(&output);
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "test-agent");
    }

    #[tokio::test]
    async fn stdio_unknown_method() {
        let output =
            run_stdio(b"{\"jsonrpc\":\"2.0\",\"method\":\"unknown/foo\",\"id\":3}\n").await;

        let resp = first_response(&output);
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn stdio_parse_error() {
        let output = run_stdio(b"not json\n").await;
        let resp = first_response(&output);
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn stdio_empty_lines_skipped() {
        let output =
            run_stdio(b"\n  \n{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"id\":1}\n\n").await;

        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines.len(), 1);
    }

    #[tokio::test]
    async fn stdio_multiple_requests() {
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialize\",\"id\":1}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"tools/list\",\"id\":2}\n",
        );
        let output = run_stdio(input.as_bytes()).await;

        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines.len(), 2);

        let resp1: Value = serde_json::from_str(lines[0]).unwrap();
        assert!(resp1["result"]["protocolVersion"].is_string());

        let resp2: Value = serde_json::from_str(lines[1]).unwrap();
        assert!(resp2["result"]["tools"].is_array());
    }

    #[tokio::test]
    async fn stdio_notification_no_response() {
        let output =
            run_stdio(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await;

        // Notifications don't produce a response.
        assert!(output.trim().is_empty());
    }

    #[tokio::test]
    async fn stdio_ping() {
        let output = run_stdio(b"{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":99}\n").await;

        let resp = first_response(&output);
        assert!(resp["result"].is_object());
        assert_eq!(resp["id"], 99);
    }
}
