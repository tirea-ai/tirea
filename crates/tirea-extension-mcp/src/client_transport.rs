use async_trait::async_trait;
use mcp::transport::{McpServerConnectionConfig, McpTransportError, TransportTypeId};
use mcp::{
    CallToolParams, CallToolResult, InitializeParams, JsonRpcId, JsonRpcMessage,
    JsonRpcNotification, JsonRpcPayload, JsonRpcRequest, JsonRpcResponse, ListToolsResult,
    McpToolDefinition, ProgressNotificationParams, ProgressToken,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Clone)]
pub struct McpProgressUpdate {
    pub progress: f64,
    pub total: Option<f64>,
    pub message: Option<String>,
}

#[async_trait]
pub trait McpToolTransport: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError>;
    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        progress_tx: Option<mpsc::UnboundedSender<McpProgressUpdate>>,
    ) -> Result<Value, McpTransportError>;
    fn transport_type(&self) -> TransportTypeId;
}

pub(crate) async fn connect_transport(
    config: &McpServerConnectionConfig,
) -> Result<Arc<dyn McpToolTransport>, McpTransportError> {
    match config.transport {
        TransportTypeId::Stdio => {
            let transport = ProgressAwareStdioTransport::connect(config).await?;
            Ok(Arc::new(transport))
        }
        TransportTypeId::Http => {
            let transport = ProgressAwareHttpTransport::connect(config)?;
            Ok(Arc::new(transport))
        }
    }
}

struct WriteRequest {
    line: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum ProgressTokenKey {
    String(String),
    Number(i64),
}

impl From<&ProgressToken> for ProgressTokenKey {
    fn from(token: &ProgressToken) -> Self {
        match token {
            ProgressToken::String(v) => ProgressTokenKey::String(v.clone()),
            ProgressToken::Number(v) => ProgressTokenKey::Number(*v),
        }
    }
}

pub(crate) struct ProgressAwareHttpTransport {
    endpoint: String,
    client: reqwest::Client,
    next_id: AtomicI64,
    next_progress_token: AtomicI64,
}

impl ProgressAwareHttpTransport {
    pub(crate) fn connect(config: &McpServerConnectionConfig) -> Result<Self, McpTransportError> {
        let endpoint = config.url.as_ref().ok_or_else(|| {
            McpTransportError::TransportError("HTTP transport requires URL".to_string())
        })?;
        let timeout = Duration::from_secs(config.timeout_secs);
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| {
                McpTransportError::TransportError(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self {
            endpoint: endpoint.clone(),
            client,
            next_id: AtomicI64::new(1),
            next_progress_token: AtomicI64::new(1),
        })
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
        progress_registration: Option<(ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>)>,
    ) -> Result<Value, McpTransportError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(JsonRpcId::Number(id), method.to_string(), params);

        let response = self
            .client
            .post(&self.endpoint)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                McpTransportError::TransportError(format!("HTTP request failed: {}", e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(McpTransportError::TransportError(format!(
                "HTTP error: {} - {}",
                status, body
            )));
        }

        let body: Value = response.json().await.map_err(|e| {
            McpTransportError::TransportError(format!("Failed to parse JSON response: {}", e))
        })?;
        decode_http_response_payload(body, id, progress_registration)
    }
}

pub(crate) struct ProgressAwareStdioTransport {
    write_tx: mpsc::Sender<WriteRequest>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, McpTransportError>>>>>,
    progress_subscribers:
        Arc<Mutex<HashMap<ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>>>>,
    next_id: AtomicI64,
    next_progress_token: AtomicI64,
    alive: Arc<AtomicBool>,
    _child: Arc<tokio::sync::Mutex<Child>>,
    timeout: Duration,
}

impl ProgressAwareStdioTransport {
    pub(crate) async fn connect(
        config: &McpServerConnectionConfig,
    ) -> Result<Self, McpTransportError> {
        let command = config.command.as_ref().ok_or_else(|| {
            McpTransportError::TransportError("Stdio transport requires command".to_string())
        })?;

        let mut cmd = Command::new(command);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            McpTransportError::TransportError(format!(
                "Failed to spawn process '{}': {}",
                command, e
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpTransportError::TransportError("Failed to get stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpTransportError::TransportError("Failed to get stdout".to_string()))?;

        let alive = Arc::new(AtomicBool::new(true));
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, McpTransportError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let progress_subscribers: Arc<
            Mutex<HashMap<ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>>>,
        > = Arc::new(Mutex::new(HashMap::new()));

        let (write_tx, mut write_rx) = mpsc::channel::<WriteRequest>(256);
        let alive_writer = Arc::clone(&alive);
        let mut stdin = stdin;
        tokio::spawn(async move {
            while let Some(req) = write_rx.recv().await {
                if !alive_writer.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(e) = stdin.write_all(req.line.as_bytes()).await {
                    eprintln!("MCP stdio write error: {}", e);
                    alive_writer.store(false, Ordering::SeqCst);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    eprintln!("MCP stdio flush error: {}", e);
                    alive_writer.store(false, Ordering::SeqCst);
                    break;
                }
            }
        });

        let pending_reader = Arc::clone(&pending);
        let progress_reader = Arc::clone(&progress_subscribers);
        let alive_reader = Arc::clone(&alive);
        let mut reader = BufReader::new(stdout);
        tokio::spawn(async move {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => {
                        alive_reader.store(false, Ordering::SeqCst);
                        break;
                    }
                    Ok(_) => match serde_json::from_str::<JsonRpcMessage>(&line) {
                        Ok(JsonRpcMessage::Response(response)) => {
                            if let JsonRpcId::Number(id) = response.id {
                                let tx = pending_reader.lock().unwrap().remove(&id);
                                if let Some(tx) = tx {
                                    let result = map_response_payload(response.payload);
                                    let _ = tx.send(result);
                                }
                            }
                        }
                        Ok(JsonRpcMessage::Notification(notification)) => {
                            handle_progress_notification(&progress_reader, notification);
                        }
                        Ok(JsonRpcMessage::Request(_)) => {}
                        Err(e) => {
                            eprintln!("Failed to parse MCP message: {} - {}", e, line.trim());
                        }
                    },
                    Err(e) => {
                        eprintln!("MCP stdio read error: {}", e);
                        alive_reader.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }

            pending_reader.lock().unwrap().clear();
            progress_reader.lock().unwrap().clear();
        });

        let transport = Self {
            write_tx,
            pending,
            progress_subscribers,
            next_id: AtomicI64::new(1),
            next_progress_token: AtomicI64::new(1),
            alive,
            _child: Arc::new(tokio::sync::Mutex::new(child)),
            timeout: Duration::from_secs(config.timeout_secs),
        };

        let init_params = InitializeParams::new(Some(config.config.clone()));
        transport
            .send_request(
                "initialize",
                Some(serde_json::to_value(&init_params)?),
                None,
            )
            .await?;
        let _ = transport
            .send_notification("notifications/initialized", Some(json!({})))
            .await;

        Ok(transport)
    }

    async fn send_notification(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), McpTransportError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(McpTransportError::ConnectionClosed);
        }
        let notification = JsonRpcNotification::new(method, params);
        let line = format!("{}\n", serde_json::to_string(&notification)?);
        self.write_tx
            .send(WriteRequest { line })
            .await
            .map_err(|_| McpTransportError::ConnectionClosed)?;
        Ok(())
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
        progress_registration: Option<(ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>)>,
    ) -> Result<Value, McpTransportError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(McpTransportError::ConnectionClosed);
        }

        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(JsonRpcId::Number(id), method.to_string(), params);
        let line = format!("{}\n", serde_json::to_string(&request)?);

        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let progress_key = progress_registration.as_ref().map(|(key, _)| key.clone());
        if let Some((key, sender)) = progress_registration {
            self.progress_subscribers
                .lock()
                .unwrap()
                .insert(key, sender);
        }

        if self.write_tx.send(WriteRequest { line }).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            if let Some(key) = progress_key {
                self.progress_subscribers.lock().unwrap().remove(&key);
            }
            return Err(McpTransportError::ConnectionClosed);
        }

        let response = tokio::time::timeout(self.timeout, rx).await;
        if let Some(key) = progress_key {
            self.progress_subscribers.lock().unwrap().remove(&key);
        }

        match response {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.pending.lock().unwrap().remove(&id);
                Err(McpTransportError::ConnectionClosed)
            }
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(McpTransportError::Timeout(format!(
                    "Request timed out after {:?}",
                    self.timeout
                )))
            }
        }
    }
}

fn handle_progress_notification(
    subscribers: &Arc<Mutex<HashMap<ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>>>>,
    notification: JsonRpcNotification,
) {
    let Some((key, update)) = decode_progress_notification(notification) else {
        return;
    };
    let sender = subscribers.lock().unwrap().get(&key).cloned();
    if let Some(sender) = sender {
        if sender.send(update).is_err() {
            subscribers.lock().unwrap().remove(&key);
        }
    }
}

fn decode_progress_notification(
    notification: JsonRpcNotification,
) -> Option<(ProgressTokenKey, McpProgressUpdate)> {
    if notification.method != "notifications/progress" {
        return None;
    }
    let params = notification.params?;
    let params = serde_json::from_value::<ProgressNotificationParams>(params).ok()?;
    let key = ProgressTokenKey::from(&params.progress_token);
    let update = McpProgressUpdate {
        progress: params.progress,
        total: params.total,
        message: params.message,
    };
    Some((key, update))
}

fn map_response_payload(payload: JsonRpcPayload) -> Result<Value, McpTransportError> {
    match payload {
        JsonRpcPayload::Success { result } => Ok(result),
        JsonRpcPayload::Error { error } => Err(McpTransportError::ServerError(format!(
            "MCP Error: {}",
            error
        ))),
    }
}

fn parse_json_rpc_message(value: Value) -> Result<JsonRpcMessage, McpTransportError> {
    match serde_json::from_value::<JsonRpcMessage>(value.clone()) {
        Ok(message) => Ok(message),
        Err(_) => serde_json::from_value::<JsonRpcResponse>(value)
            .map(JsonRpcMessage::Response)
            .map_err(McpTransportError::from),
    }
}

fn decode_http_response_payload(
    body: Value,
    request_id: i64,
    progress_registration: Option<(ProgressTokenKey, mpsc::UnboundedSender<McpProgressUpdate>)>,
) -> Result<Value, McpTransportError> {
    let progress_key = progress_registration.as_ref().map(|(key, _)| key.clone());
    let progress_tx = progress_registration
        .as_ref()
        .map(|(_, sender)| sender.clone());
    let mut matched_response: Option<Result<Value, McpTransportError>> = None;

    let mut process_message = |message: JsonRpcMessage| match message {
        JsonRpcMessage::Response(response) => {
            if matches!(response.id, JsonRpcId::Number(id) if id == request_id) {
                matched_response = Some(map_response_payload(response.payload));
            }
        }
        JsonRpcMessage::Notification(notification) => {
            let Some(expected_key) = progress_key.as_ref() else {
                return;
            };
            let Some(sender) = progress_tx.as_ref() else {
                return;
            };
            let Some((key, update)) = decode_progress_notification(notification) else {
                return;
            };
            if key == *expected_key {
                let _ = sender.send(update);
            }
        }
        JsonRpcMessage::Request(_) => {}
    };

    match body {
        Value::Array(items) => {
            for item in items {
                let message = parse_json_rpc_message(item)?;
                process_message(message);
            }
        }
        other => {
            let message = parse_json_rpc_message(other)?;
            process_message(message);
        }
    }

    matched_response.unwrap_or_else(|| {
        Err(McpTransportError::ProtocolError(format!(
            "Missing response for request id {}",
            request_id
        )))
    })
}

#[async_trait]
impl McpToolTransport for ProgressAwareStdioTransport {
    async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
        let result = self
            .send_request("tools/list", Some(json!({})), None)
            .await?;
        let list_result: ListToolsResult = serde_json::from_value(result)?;
        Ok(list_result.tools)
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        progress_tx: Option<mpsc::UnboundedSender<McpProgressUpdate>>,
    ) -> Result<Value, McpTransportError> {
        let progress_registration = progress_tx.map(|sender| {
            let token =
                ProgressToken::Number(self.next_progress_token.fetch_add(1, Ordering::SeqCst));
            let key = ProgressTokenKey::from(&token);
            (token, key, sender)
        });

        let (meta, progress_sender) = if let Some((token, key, sender)) = progress_registration {
            let mut map = Map::new();
            map.insert("progressToken".to_string(), serde_json::to_value(token)?);
            (Some(Value::Object(map)), Some((key, sender)))
        } else {
            (None, None)
        };

        let params = CallToolParams {
            name: name.to_string(),
            arguments: Some(args),
            task: None,
            meta,
        };

        let result = self
            .send_request(
                "tools/call",
                Some(serde_json::to_value(&params)?),
                progress_sender,
            )
            .await?;
        let call_result: CallToolResult = serde_json::from_value(result)?;

        if call_result.is_error == Some(true) {
            let error_text = call_result
                .content
                .first()
                .and_then(|c| c.as_text())
                .unwrap_or("Unknown error");
            return Err(McpTransportError::ServerError(error_text.to_string()));
        }

        let text = call_result
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Value::String(text))
    }

    fn transport_type(&self) -> TransportTypeId {
        TransportTypeId::Stdio
    }
}

#[async_trait]
impl McpToolTransport for ProgressAwareHttpTransport {
    async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
        let result = self
            .send_request("tools/list", Some(json!({})), None)
            .await?;
        let list_result: ListToolsResult = serde_json::from_value(result)?;
        Ok(list_result.tools)
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        progress_tx: Option<mpsc::UnboundedSender<McpProgressUpdate>>,
    ) -> Result<Value, McpTransportError> {
        let progress_registration = progress_tx.map(|sender| {
            let token =
                ProgressToken::Number(self.next_progress_token.fetch_add(1, Ordering::SeqCst));
            let key = ProgressTokenKey::from(&token);
            (token, key, sender)
        });

        let (meta, progress_sender) = if let Some((token, key, sender)) = progress_registration {
            let mut map = Map::new();
            map.insert("progressToken".to_string(), serde_json::to_value(token)?);
            (Some(Value::Object(map)), Some((key, sender)))
        } else {
            (None, None)
        };

        let params = CallToolParams {
            name: name.to_string(),
            arguments: Some(args),
            task: None,
            meta,
        };

        let result = self
            .send_request(
                "tools/call",
                Some(serde_json::to_value(&params)?),
                progress_sender,
            )
            .await?;
        let call_result: CallToolResult = serde_json::from_value(result)?;

        if call_result.is_error == Some(true) {
            let error_text = call_result
                .content
                .first()
                .and_then(|c| c.as_text())
                .unwrap_or("Unknown error");
            return Err(McpTransportError::ServerError(error_text.to_string()));
        }

        let text = call_result
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Value::String(text))
    }

    fn transport_type(&self) -> TransportTypeId {
        TransportTypeId::Http
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decode_http_batch_routes_progress_and_returns_matching_response() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let body = json!([
            {
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": 7,
                    "progress": 2.0,
                    "total": 4.0,
                    "message": "phase"
                }
            },
            {
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "content": [{"type": "text", "text": "ok"}]
                }
            }
        ]);

        let result = decode_http_response_payload(body, 1, Some((ProgressTokenKey::Number(7), tx)))
            .expect("decode response");

        let update = rx.try_recv().expect("progress update");
        assert_eq!(update.progress, 2.0);
        assert_eq!(update.total, Some(4.0));
        assert_eq!(update.message.as_deref(), Some("phase"));
        assert_eq!(result["content"][0]["text"], json!("ok"));
    }

    #[test]
    fn decode_http_batch_ignores_progress_with_other_token() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let body = json!([
            {
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": 9,
                    "progress": 1.0
                }
            },
            {
                "jsonrpc": "2.0",
                "id": 3,
                "result": {
                    "content": [{"type": "text", "text": "ok"}]
                }
            }
        ]);

        let _ = decode_http_response_payload(body, 3, Some((ProgressTokenKey::Number(7), tx)))
            .expect("decode response");
        assert!(rx.try_recv().is_err(), "unexpected progress update");
    }

    #[test]
    fn decode_http_response_requires_matching_response_id() {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {"content": [{"type": "text", "text": "ok"}]}
        });
        let err = decode_http_response_payload(body, 1, None)
            .err()
            .expect("error");
        assert!(matches!(err, McpTransportError::ProtocolError(_)));
    }
}
