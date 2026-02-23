use async_trait::async_trait;
use mcp::transport::{McpServerConnectionConfig, McpTransport, McpTransportError, TransportTypeId};
use mcp::transport_factory::TransportFactory;
use mcp::{
    CallToolParams, CallToolResult, InitializeParams, JsonRpcId, JsonRpcMessage,
    JsonRpcNotification, JsonRpcPayload, JsonRpcRequest, ListToolsResult, McpToolDefinition,
    ProgressNotificationParams, ProgressToken,
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
pub(crate) struct McpProgressUpdate {
    pub(crate) progress: f64,
    pub(crate) total: Option<f64>,
    pub(crate) message: Option<String>,
}

#[async_trait]
pub(crate) trait McpToolTransport: Send + Sync {
    async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError>;
    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        progress_tx: Option<mpsc::UnboundedSender<McpProgressUpdate>>,
    ) -> Result<Value, McpTransportError>;
    fn transport_type(&self) -> TransportTypeId;
}

pub(crate) struct LegacyMcpTransportAdapter {
    inner: Arc<dyn McpTransport>,
}

impl LegacyMcpTransportAdapter {
    pub(crate) fn new(inner: Arc<dyn McpTransport>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl McpToolTransport for LegacyMcpTransportAdapter {
    async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
        self.inner.list_tools().await
    }

    async fn call_tool(
        &self,
        name: &str,
        args: Value,
        _progress_tx: Option<mpsc::UnboundedSender<McpProgressUpdate>>,
    ) -> Result<Value, McpTransportError> {
        self.inner.call_tool(name, args).await
    }

    fn transport_type(&self) -> TransportTypeId {
        self.inner.transport_type()
    }
}

pub(crate) async fn connect_transport(
    config: &McpServerConnectionConfig,
) -> Result<Arc<dyn McpToolTransport>, McpTransportError> {
    match config.transport {
        TransportTypeId::Stdio => {
            let transport = ProgressAwareStdioTransport::connect(config).await?;
            Ok(Arc::new(transport))
        }
        _ => {
            let transport = TransportFactory::create(config).await?;
            Ok(Arc::new(LegacyMcpTransportAdapter::new(transport)))
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
                                    let result = match response.payload {
                                        JsonRpcPayload::Success { result } => Ok(result),
                                        JsonRpcPayload::Error { error } => {
                                            Err(McpTransportError::ServerError(format!(
                                                "MCP Error: {}",
                                                error
                                            )))
                                        }
                                    };
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
    if notification.method != "notifications/progress" {
        return;
    }
    let Some(params) = notification.params else {
        return;
    };
    let Ok(params) = serde_json::from_value::<ProgressNotificationParams>(params) else {
        return;
    };

    let key = ProgressTokenKey::from(&params.progress_token);
    let sender = subscribers.lock().unwrap().get(&key).cloned();
    if let Some(sender) = sender {
        let update = McpProgressUpdate {
            progress: params.progress,
            total: params.total,
            message: params.message,
        };
        if sender.send(update).is_err() {
            subscribers.lock().unwrap().remove(&key);
        }
    }
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
