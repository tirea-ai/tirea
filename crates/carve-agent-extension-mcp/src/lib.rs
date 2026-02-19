use async_trait::async_trait;
use carve_agent_contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agent_contract::ToolCallContext;
use carve_agent_contract::ToolRegistry;
use mcp::transport::{McpServerConnectionConfig, McpTransport, McpTransportError, TransportTypeId};
use mcp::transport_factory::TransportFactory;
use mcp::McpToolDefinition;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

const MCP_META_SERVER: &str = "mcp.server";
const MCP_META_TOOL: &str = "mcp.tool";
const MCP_META_TRANSPORT: &str = "mcp.transport";

#[derive(Debug, thiserror::Error)]
pub enum McpToolRegistryError {
    #[error("server name must be non-empty")]
    EmptyServerName,

    #[error("duplicate server name: {0}")]
    DuplicateServerName(String),

    #[error("invalid tool id component after sanitization: {0}")]
    InvalidToolIdComponent(String),

    #[error("tool id already registered: {0}")]
    ToolIdConflict(String),

    #[error("mcp transport error: {0}")]
    Transport(String),

    #[error("periodic refresh interval must be > 0")]
    InvalidRefreshInterval,

    #[error("periodic refresh loop is already running")]
    PeriodicRefreshAlreadyRunning,

    #[error("tokio runtime is required to start periodic refresh")]
    RuntimeUnavailable,
}

impl From<McpTransportError> for McpToolRegistryError {
    fn from(e: McpTransportError) -> Self {
        Self::Transport(e.to_string())
    }
}

fn validate_server_name(name: &str) -> Result<(), McpToolRegistryError> {
    if name.trim().is_empty() {
        return Err(McpToolRegistryError::EmptyServerName);
    }
    Ok(())
}

fn with_mcp_descriptor_metadata(
    descriptor: ToolDescriptor,
    server_name: &str,
    tool_name: &str,
    transport_type: TransportTypeId,
) -> ToolDescriptor {
    descriptor
        .with_metadata(MCP_META_SERVER, Value::String(server_name.to_string()))
        .with_metadata(MCP_META_TOOL, Value::String(tool_name.to_string()))
        .with_metadata(
            MCP_META_TRANSPORT,
            Value::String(transport_type.to_string()),
        )
}

fn with_mcp_result_metadata(
    tool_result: ToolResult,
    server_name: &str,
    tool_name: &str,
) -> ToolResult {
    tool_result
        .with_metadata(MCP_META_SERVER, Value::String(server_name.to_string()))
        .with_metadata(MCP_META_TOOL, Value::String(tool_name.to_string()))
}

struct McpTool {
    descriptor: ToolDescriptor,
    server_name: String,
    tool_name: String,
    transport: Arc<dyn McpTransport>,
}

impl McpTool {
    fn new(
        tool_id: String,
        server_name: String,
        def: McpToolDefinition,
        transport: Arc<dyn McpTransport>,
        transport_type: TransportTypeId,
    ) -> Self {
        let name = def.title.clone().unwrap_or_else(|| def.name.clone());
        let desc = def
            .description
            .clone()
            .unwrap_or_else(|| format!("MCP tool {}", def.name));

        let mut d = with_mcp_descriptor_metadata(
            ToolDescriptor::new(tool_id, name, desc).with_parameters(def.input_schema.clone()),
            &server_name,
            &def.name,
            transport_type,
        );

        if let Some(group) = def.group.clone() {
            d = d.with_category(group);
        }

        Self {
            descriptor: d,
            server_name,
            tool_name: def.name,
            transport,
        }
    }

    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }
}

#[async_trait]
impl Tool for McpTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor()
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
        let res = self
            .transport
            .call_tool(&self.tool_name, args)
            .await
            .map_err(map_mcp_error)?;

        Ok(with_mcp_result_metadata(
            ToolResult::success(self.descriptor.id.clone(), res),
            &self.server_name,
            &self.tool_name,
        ))
    }
}

fn map_mcp_error(e: McpTransportError) -> ToolError {
    match e {
        McpTransportError::UnknownTool(name) => ToolError::NotFound(name),
        McpTransportError::Timeout(msg) => ToolError::ExecutionFailed(format!("timeout: {}", msg)),
        other => ToolError::ExecutionFailed(other.to_string()),
    }
}

fn sanitize_component(raw: &str) -> Result<String, McpToolRegistryError> {
    let mut out = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for ch in raw.chars() {
        let keep = ch.is_ascii_alphanumeric();
        let next = if keep { ch } else { '_' };
        if next == '_' {
            if prev_underscore {
                continue;
            }
            prev_underscore = true;
        } else {
            prev_underscore = false;
        }
        out.push(next);
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        return Err(McpToolRegistryError::InvalidToolIdComponent(
            raw.to_string(),
        ));
    }
    Ok(out)
}

fn to_tool_id(server_name: &str, tool_name: &str) -> Result<String, McpToolRegistryError> {
    let s = sanitize_component(server_name)?;
    let t = sanitize_component(tool_name)?;
    Ok(format!("mcp__{}__{}", s, t))
}

#[derive(Clone)]
struct McpServerRuntime {
    name: String,
    transport_type: TransportTypeId,
    transport: Arc<dyn McpTransport>,
}

#[derive(Clone, Default)]
struct McpRegistrySnapshot {
    version: u64,
    tools: HashMap<String, Arc<dyn Tool>>,
}

struct PeriodicRefreshRuntime {
    stop_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

struct McpRegistryState {
    servers: Vec<McpServerRuntime>,
    snapshot: RwLock<McpRegistrySnapshot>,
    periodic_refresh: Mutex<Option<PeriodicRefreshRuntime>>,
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn mutex_lock<T>(lock: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn is_periodic_refresh_running(state: &McpRegistryState) -> bool {
    let mut runtime = mutex_lock(&state.periodic_refresh);
    if runtime
        .as_ref()
        .is_some_and(|running| running.join.is_finished())
    {
        *runtime = None;
        return false;
    }
    runtime.is_some()
}

async fn discover_tools(
    servers: &[McpServerRuntime],
) -> Result<HashMap<String, Arc<dyn Tool>>, McpToolRegistryError> {
    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    for server in servers {
        let mut defs = server.transport.list_tools().await?;
        defs.sort_by(|a, b| a.name.cmp(&b.name));

        for def in defs {
            let tool_id = to_tool_id(&server.name, &def.name)?;
            if tools.contains_key(&tool_id) {
                return Err(McpToolRegistryError::ToolIdConflict(tool_id));
            }
            tools.insert(
                tool_id.clone(),
                Arc::new(McpTool::new(
                    tool_id,
                    server.name.clone(),
                    def,
                    server.transport.clone(),
                    server.transport_type,
                )) as Arc<dyn Tool>,
            );
        }
    }

    Ok(tools)
}

async fn refresh_state(state: &McpRegistryState) -> Result<u64, McpToolRegistryError> {
    let tools = discover_tools(&state.servers).await?;
    let mut snapshot = write_lock(&state.snapshot);
    let version = snapshot.version.saturating_add(1);
    *snapshot = McpRegistrySnapshot { version, tools };
    Ok(version)
}

async fn periodic_refresh_loop(
    state: Weak<McpRegistryState>,
    interval: Duration,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = &mut stop_rx => break,
            _ = ticker.tick() => {
                let Some(state) = state.upgrade() else {
                    break;
                };
                let _ = refresh_state(state.as_ref()).await;
            }
        }
    }
}

/// Dynamic MCP registry manager.
///
/// Keeps server transports alive and refreshes discovered tool definitions
/// into a shared snapshot consumed by [`McpToolRegistry`].
#[derive(Clone)]
pub struct McpToolRegistryManager {
    state: Arc<McpRegistryState>,
}

impl std::fmt::Debug for McpToolRegistryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snapshot = read_lock(&self.state.snapshot);
        let periodic_running = is_periodic_refresh_running(self.state.as_ref());
        f.debug_struct("McpToolRegistryManager")
            .field("servers", &self.state.servers.len())
            .field("tools", &snapshot.tools.len())
            .field("version", &snapshot.version)
            .field("periodic_refresh_running", &periodic_running)
            .finish()
    }
}

impl McpToolRegistryManager {
    pub async fn connect(
        configs: impl IntoIterator<Item = McpServerConnectionConfig>,
    ) -> Result<Self, McpToolRegistryError> {
        let mut entries: Vec<(McpServerConnectionConfig, Arc<dyn McpTransport>)> = Vec::new();
        for cfg in configs {
            validate_server_name(&cfg.name)?;
            let transport = TransportFactory::create(&cfg).await?;
            entries.push((cfg, transport));
        }
        Self::from_transports(entries).await
    }

    pub async fn from_transports(
        entries: impl IntoIterator<Item = (McpServerConnectionConfig, Arc<dyn McpTransport>)>,
    ) -> Result<Self, McpToolRegistryError> {
        let servers = Self::build_servers(entries)?;
        let tools = discover_tools(&servers).await?;

        let snapshot = McpRegistrySnapshot { version: 1, tools };
        Ok(Self {
            state: Arc::new(McpRegistryState {
                servers,
                snapshot: RwLock::new(snapshot),
                periodic_refresh: Mutex::new(None),
            }),
        })
    }

    fn build_servers(
        entries: impl IntoIterator<Item = (McpServerConnectionConfig, Arc<dyn McpTransport>)>,
    ) -> Result<Vec<McpServerRuntime>, McpToolRegistryError> {
        let mut servers: Vec<McpServerRuntime> = Vec::new();
        let mut names: HashSet<String> = HashSet::new();

        for (cfg, transport) in entries {
            validate_server_name(&cfg.name)?;
            if !names.insert(cfg.name.clone()) {
                return Err(McpToolRegistryError::DuplicateServerName(cfg.name));
            }

            servers.push(McpServerRuntime {
                name: cfg.name,
                transport_type: transport.transport_type(),
                transport,
            });
        }

        servers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(servers)
    }

    /// Refresh all MCP tool definitions atomically.
    ///
    /// On failure, the previously published snapshot is preserved.
    pub async fn refresh(&self) -> Result<u64, McpToolRegistryError> {
        refresh_state(self.state.as_ref()).await
    }

    /// Start background periodic refresh.
    ///
    /// The first refresh tick runs after `interval`.
    pub fn start_periodic_refresh(&self, interval: Duration) -> Result<(), McpToolRegistryError> {
        if interval.is_zero() {
            return Err(McpToolRegistryError::InvalidRefreshInterval);
        }

        let handle = Handle::try_current().map_err(|_| McpToolRegistryError::RuntimeUnavailable)?;
        let mut runtime = mutex_lock(&self.state.periodic_refresh);
        if runtime
            .as_ref()
            .is_some_and(|running| !running.join.is_finished())
        {
            return Err(McpToolRegistryError::PeriodicRefreshAlreadyRunning);
        }

        let (stop_tx, stop_rx) = oneshot::channel();
        let weak_state = Arc::downgrade(&self.state);
        let join = handle.spawn(periodic_refresh_loop(weak_state, interval, stop_rx));

        *runtime = Some(PeriodicRefreshRuntime {
            stop_tx: Some(stop_tx),
            join,
        });
        Ok(())
    }

    /// Stop the background periodic refresh loop.
    ///
    /// Returns `true` if a running loop existed.
    pub async fn stop_periodic_refresh(&self) -> bool {
        let runtime = {
            let mut guard = mutex_lock(&self.state.periodic_refresh);
            guard.take()
        };

        let Some(mut runtime) = runtime else {
            return false;
        };

        if let Some(stop_tx) = runtime.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        let _ = runtime.join.await;
        true
    }

    /// Whether periodic refresh loop is running.
    pub fn periodic_refresh_running(&self) -> bool {
        is_periodic_refresh_running(self.state.as_ref())
    }

    /// Get the tool-registry view backed by this manager.
    pub fn registry(&self) -> McpToolRegistry {
        McpToolRegistry {
            state: self.state.clone(),
        }
    }

    /// Current published snapshot version.
    pub fn version(&self) -> u64 {
        read_lock(&self.state.snapshot).version
    }

    pub fn servers(&self) -> Vec<(String, TransportTypeId)> {
        self.state
            .servers
            .iter()
            .map(|server| (server.name.clone(), server.transport_type))
            .collect()
    }
}

/// Dynamic `ToolRegistry` view backed by [`McpToolRegistryManager`].
#[derive(Clone)]
pub struct McpToolRegistry {
    state: Arc<McpRegistryState>,
}

impl std::fmt::Debug for McpToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snapshot = read_lock(&self.state.snapshot);
        let periodic_running = is_periodic_refresh_running(self.state.as_ref());
        f.debug_struct("McpToolRegistry")
            .field("servers", &self.state.servers.len())
            .field("tools", &snapshot.tools.len())
            .field("version", &snapshot.version)
            .field("periodic_refresh_running", &periodic_running)
            .finish()
    }
}

impl McpToolRegistry {
    /// Current published snapshot version.
    pub fn version(&self) -> u64 {
        read_lock(&self.state.snapshot).version
    }

    pub fn servers(&self) -> Vec<(String, TransportTypeId)> {
        self.state
            .servers
            .iter()
            .map(|server| (server.name.clone(), server.transport_type))
            .collect()
    }
}

impl ToolRegistry for McpToolRegistry {
    fn len(&self) -> usize {
        read_lock(&self.state.snapshot).tools.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        read_lock(&self.state.snapshot).tools.get(id).cloned()
    }

    fn ids(&self) -> Vec<String> {
        let snapshot = read_lock(&self.state.snapshot);
        let mut ids: Vec<String> = snapshot.tools.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>> {
        read_lock(&self.state.snapshot).tools.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Instant;

    #[derive(Debug, Clone)]
    struct FakeTransport {
        tools: Arc<Mutex<Vec<McpToolDefinition>>>,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        fail_next_list: Arc<Mutex<Option<String>>>,
        list_calls: Arc<AtomicUsize>,
    }

    impl FakeTransport {
        fn new(tools: Vec<McpToolDefinition>) -> Self {
            Self {
                tools: Arc::new(Mutex::new(tools)),
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_next_list: Arc::new(Mutex::new(None)),
                list_calls: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn set_tools(&self, tools: Vec<McpToolDefinition>) {
            *self.tools.lock().unwrap() = tools;
        }

        fn fail_next_list(&self, message: impl Into<String>) {
            *self.fail_next_list.lock().unwrap() = Some(message.into());
        }

        fn list_calls(&self) -> usize {
            self.list_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl McpTransport for FakeTransport {
        async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
            self.list_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(message) = self.fail_next_list.lock().unwrap().take() {
                return Err(McpTransportError::TransportError(message));
            }
            Ok(self.tools.lock().unwrap().clone())
        }

        async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpTransportError> {
            self.calls.lock().unwrap().push((name.to_string(), args));
            Ok(serde_json::json!({"ok": true}))
        }

        async fn shutdown(&self) -> Result<(), McpTransportError> {
            Ok(())
        }

        fn is_alive(&self) -> bool {
            true
        }

        fn transport_type(&self) -> TransportTypeId {
            TransportTypeId::Stdio
        }
    }

    fn cfg(name: &str) -> McpServerConnectionConfig {
        McpServerConnectionConfig::stdio(name, "node", vec!["server.js".to_string()])
    }

    #[tokio::test]
    async fn registry_discovers_tools_and_executes_calls() {
        let fake = Arc::new(FakeTransport::new(vec![
            McpToolDefinition::new("echo").with_title("Echo")
        ]));
        let transport = fake.clone() as Arc<dyn McpTransport>;

        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();
        let reg = manager.registry();

        let id = reg.ids().into_iter().find(|x| x.contains("echo")).unwrap();
        let tool = reg.get(&id).unwrap();

        let desc = tool.descriptor();
        assert_eq!(desc.id, id);
        assert_eq!(desc.name, "Echo");
        assert!(desc.metadata.contains_key("mcp.server"));
        assert!(desc.metadata.contains_key("mcp.tool"));

        let state = carve_agent_contract::AgentState::new_transient(
            &serde_json::json!({}),
            "call",
            "test",
        );
        let ctx = state.as_tool_call_context();
        let res = tool
            .execute(serde_json::json!({"a": 1}), &ctx)
            .await
            .unwrap();
        assert!(res.is_success());

        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "echo");
    }

    #[tokio::test]
    async fn registry_refresh_discovers_new_tools_without_rebuild() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;

        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();
        let reg = manager.registry();
        assert_eq!(manager.version(), 1);

        fake.set_tools(vec![
            McpToolDefinition::new("echo"),
            McpToolDefinition::new("sum"),
        ]);

        let version = manager.refresh().await.unwrap();
        assert_eq!(version, 2);
        assert!(reg.ids().into_iter().any(|id| id.contains("sum")));
    }

    #[tokio::test]
    async fn failed_refresh_keeps_last_good_snapshot() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;

        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();
        let reg = manager.registry();
        let initial_ids = reg.ids();

        fake.fail_next_list("temporary outage");

        let err = manager.refresh().await.err().unwrap();
        assert!(matches!(err, McpToolRegistryError::Transport(_)));
        assert_eq!(manager.version(), 1);
        assert_eq!(reg.ids(), initial_ids);
    }

    async fn wait_until(
        timeout: Duration,
        step: Duration,
        mut predicate: impl FnMut() -> bool,
    ) -> bool {
        let start = Instant::now();
        while start.elapsed() <= timeout {
            if predicate() {
                return true;
            }
            tokio::time::sleep(step).await;
        }
        predicate()
    }

    #[tokio::test]
    async fn periodic_refresh_updates_snapshot_and_can_stop() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;
        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport)])
            .await
            .unwrap();
        let reg = manager.registry();

        manager
            .start_periodic_refresh(Duration::from_millis(20))
            .expect("start periodic refresh");
        assert!(manager.periodic_refresh_running());

        fake.set_tools(vec![
            McpToolDefinition::new("echo"),
            McpToolDefinition::new("sum"),
        ]);

        let observed = wait_until(
            Duration::from_millis(400),
            Duration::from_millis(20),
            || manager.version() >= 2 && reg.ids().iter().any(|id| id.contains("sum")),
        )
        .await;
        assert!(observed, "periodic refresh should publish updated tools");
        assert!(
            fake.list_calls() >= 2,
            "list_tools should be called periodically"
        );

        assert!(manager.stop_periodic_refresh().await);
        assert!(!manager.periodic_refresh_running());

        let version_after_stop = manager.version();
        fake.set_tools(vec![
            McpToolDefinition::new("echo"),
            McpToolDefinition::new("sum"),
            McpToolDefinition::new("mul"),
        ]);
        tokio::time::sleep(Duration::from_millis(80)).await;

        assert_eq!(
            manager.version(),
            version_after_stop,
            "version should not change after periodic refresh stops"
        );
        assert!(
            !reg.ids().iter().any(|id| id.contains("mul")),
            "stopped periodic refresh should not publish new tools"
        );
    }

    #[tokio::test]
    async fn periodic_refresh_rejects_duplicate_start() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;
        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport)])
            .await
            .unwrap();

        manager
            .start_periodic_refresh(Duration::from_millis(100))
            .expect("start periodic refresh");
        let err = manager
            .start_periodic_refresh(Duration::from_millis(100))
            .err()
            .unwrap();
        assert!(matches!(
            err,
            McpToolRegistryError::PeriodicRefreshAlreadyRunning
        ));
        assert!(manager.stop_periodic_refresh().await);
    }

    #[tokio::test]
    async fn periodic_refresh_rejects_zero_interval() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;
        let manager = McpToolRegistryManager::from_transports([(cfg("s1"), transport)])
            .await
            .unwrap();

        let err = manager
            .start_periodic_refresh(Duration::from_millis(0))
            .err()
            .unwrap();
        assert!(matches!(err, McpToolRegistryError::InvalidRefreshInterval));
    }

    #[tokio::test]
    async fn sanitize_rejects_empty_component() {
        let err = to_tool_id("   ", "echo").err().unwrap();
        assert!(matches!(
            err,
            McpToolRegistryError::InvalidToolIdComponent(_)
        ));
    }

    #[tokio::test]
    async fn tool_id_conflict_is_an_error() {
        let transport = Arc::new(FakeTransport::new(vec![
            McpToolDefinition::new("a-b"),
            McpToolDefinition::new("a_b"),
        ])) as Arc<dyn McpTransport>;

        let err = McpToolRegistryManager::from_transports([(cfg("s1"), transport)])
            .await
            .err()
            .unwrap();
        assert!(matches!(err, McpToolRegistryError::ToolIdConflict(_)));
    }
}
