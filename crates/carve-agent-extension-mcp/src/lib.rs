use async_trait::async_trait;
use carve_agent_contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agent_contract::AgentState;
use carve_agent_contract::ToolRegistry;
use mcp::transport::{McpServerConnectionConfig, McpTransport, McpTransportError, TransportTypeId};
use mcp::transport_factory::TransportFactory;
use mcp::McpToolDefinition;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

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
}

impl From<McpTransportError> for McpToolRegistryError {
    fn from(e: McpTransportError) -> Self {
        Self::Transport(e.to_string())
    }
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

        let mut d = ToolDescriptor::new(tool_id, name, desc)
            .with_parameters(def.input_schema.clone())
            .with_metadata("mcp.server", Value::String(server_name.clone()))
            .with_metadata("mcp.tool", Value::String(def.name.clone()))
            .with_metadata("mcp.transport", Value::String(transport_type.to_string()));

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

    async fn execute(&self, args: Value, _ctx: &AgentState) -> Result<ToolResult, ToolError> {
        let res = self
            .transport
            .call_tool(&self.tool_name, args)
            .await
            .map_err(map_mcp_error)?;

        Ok(ToolResult::success(self.descriptor.id.clone(), res)
            .with_metadata("mcp.server", Value::String(self.server_name.clone()))
            .with_metadata("mcp.tool", Value::String(self.tool_name.clone())))
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

struct McpRegistryState {
    servers: Vec<McpServerRuntime>,
    snapshot: RwLock<McpRegistrySnapshot>,
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
        f.debug_struct("McpToolRegistryManager")
            .field("servers", &self.state.servers.len())
            .field("tools", &snapshot.tools.len())
            .field("version", &snapshot.version)
            .finish()
    }
}

impl McpToolRegistryManager {
    pub async fn connect(
        configs: impl IntoIterator<Item = McpServerConnectionConfig>,
    ) -> Result<Self, McpToolRegistryError> {
        let mut entries: Vec<(McpServerConnectionConfig, Arc<dyn McpTransport>)> = Vec::new();
        for cfg in configs {
            if cfg.name.trim().is_empty() {
                return Err(McpToolRegistryError::EmptyServerName);
            }
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
            }),
        })
    }

    fn build_servers(
        entries: impl IntoIterator<Item = (McpServerConnectionConfig, Arc<dyn McpTransport>)>,
    ) -> Result<Vec<McpServerRuntime>, McpToolRegistryError> {
        let mut servers: Vec<McpServerRuntime> = Vec::new();
        let mut names: HashSet<String> = HashSet::new();

        for (cfg, transport) in entries {
            if cfg.name.trim().is_empty() {
                return Err(McpToolRegistryError::EmptyServerName);
            }
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
        let tools = discover_tools(&self.state.servers).await?;

        let mut snapshot = write_lock(&self.state.snapshot);
        let version = snapshot.version.saturating_add(1);
        *snapshot = McpRegistrySnapshot { version, tools };
        Ok(version)
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
        f.debug_struct("McpToolRegistry")
            .field("servers", &self.state.servers.len())
            .field("tools", &snapshot.tools.len())
            .field("version", &snapshot.version)
            .finish()
    }
}

impl McpToolRegistry {
    pub async fn connect(
        configs: impl IntoIterator<Item = McpServerConnectionConfig>,
    ) -> Result<Self, McpToolRegistryError> {
        Ok(McpToolRegistryManager::connect(configs).await?.registry())
    }

    pub async fn from_transports(
        entries: impl IntoIterator<Item = (McpServerConnectionConfig, Arc<dyn McpTransport>)>,
    ) -> Result<Self, McpToolRegistryError> {
        Ok(McpToolRegistryManager::from_transports(entries)
            .await?
            .registry())
    }

    /// Refresh MCP tools in place.
    pub async fn refresh(&self) -> Result<u64, McpToolRegistryError> {
        self.manager().refresh().await
    }

    /// Build a manager handle from this registry.
    pub fn manager(&self) -> McpToolRegistryManager {
        McpToolRegistryManager {
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
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    struct FakeTransport {
        tools: Arc<Mutex<Vec<McpToolDefinition>>>,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        fail_next_list: Arc<Mutex<Option<String>>>,
    }

    impl FakeTransport {
        fn new(tools: Vec<McpToolDefinition>) -> Self {
            Self {
                tools: Arc::new(Mutex::new(tools)),
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_next_list: Arc::new(Mutex::new(None)),
            }
        }

        fn set_tools(&self, tools: Vec<McpToolDefinition>) {
            *self.tools.lock().unwrap() = tools;
        }

        fn fail_next_list(&self, message: impl Into<String>) {
            *self.fail_next_list.lock().unwrap() = Some(message.into());
        }
    }

    #[async_trait]
    impl McpTransport for FakeTransport {
        async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
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

        let reg = McpToolRegistry::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();

        let id = reg.ids().into_iter().find(|x| x.contains("echo")).unwrap();
        let tool = reg.get(&id).unwrap();

        let desc = tool.descriptor();
        assert_eq!(desc.id, id);
        assert_eq!(desc.name, "Echo");
        assert!(desc.metadata.contains_key("mcp.server"));
        assert!(desc.metadata.contains_key("mcp.tool"));

        let res = tool
            .execute(
                serde_json::json!({"a": 1}),
                &carve_agent_contract::AgentState::new_transient(
                    &serde_json::json!({}),
                    "call",
                    "test",
                ),
            )
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

        let reg = McpToolRegistry::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();
        assert_eq!(reg.version(), 1);

        fake.set_tools(vec![
            McpToolDefinition::new("echo"),
            McpToolDefinition::new("sum"),
        ]);

        let version = reg.refresh().await.unwrap();
        assert_eq!(version, 2);
        assert!(reg.ids().into_iter().any(|id| id.contains("sum")));
    }

    #[tokio::test]
    async fn failed_refresh_keeps_last_good_snapshot() {
        let fake = Arc::new(FakeTransport::new(vec![McpToolDefinition::new("echo")]));
        let transport = fake.clone() as Arc<dyn McpTransport>;

        let reg = McpToolRegistry::from_transports([(cfg("s1"), transport.clone())])
            .await
            .unwrap();
        let initial_ids = reg.ids();

        fake.fail_next_list("temporary outage");

        let err = reg.refresh().await.err().unwrap();
        assert!(matches!(err, McpToolRegistryError::Transport(_)));
        assert_eq!(reg.version(), 1);
        assert_eq!(reg.ids(), initial_ids);
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

        let err = McpToolRegistry::from_transports([(cfg("s1"), transport)])
            .await
            .err()
            .unwrap();
        assert!(matches!(err, McpToolRegistryError::ToolIdConflict(_)));
    }
}
