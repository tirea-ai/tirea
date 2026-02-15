use crate::agent_os::ToolRegistry;
use crate::contracts::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use async_trait::async_trait;
use mcp::transport::{McpServerConnectionConfig, McpTransport, McpTransportError, TransportTypeId};
use mcp::transport_factory::TransportFactory;
use mcp::McpToolDefinition;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

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
            .with_metadata(
                "mcp.transport",
                Value::String(transport.transport_type().to_string()),
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

    async fn execute(
        &self,
        args: Value,
        _ctx: &carve_state::Context<'_>,
    ) -> Result<ToolResult, ToolError> {
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

/// A `ToolRegistry` built from MCP servers.
///
/// This is an immutable discovery result. To refresh tools, build a new instance.
#[derive(Clone)]
pub struct McpToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    servers: HashMap<String, TransportTypeId>,
}

impl std::fmt::Debug for McpToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpToolRegistry")
            .field("servers", &self.servers.len())
            .field("tools", &self.tools.len())
            .finish()
    }
}

impl McpToolRegistry {
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
        let mut servers: HashMap<String, TransportTypeId> = HashMap::new();
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

        for (cfg, transport) in entries {
            if cfg.name.trim().is_empty() {
                return Err(McpToolRegistryError::EmptyServerName);
            }
            if servers.contains_key(&cfg.name) {
                return Err(McpToolRegistryError::DuplicateServerName(cfg.name));
            }

            let defs = transport.list_tools().await?;
            let transport_type = transport.transport_type();
            servers.insert(cfg.name.clone(), transport_type);

            for def in defs {
                let tool_id = to_tool_id(&cfg.name, &def.name)?;
                if tools.contains_key(&tool_id) {
                    return Err(McpToolRegistryError::ToolIdConflict(tool_id));
                }
                tools.insert(
                    tool_id.clone(),
                    Arc::new(McpTool::new(
                        tool_id,
                        cfg.name.clone(),
                        def,
                        transport.clone(),
                    )) as Arc<dyn Tool>,
                );
            }
        }

        Ok(Self { tools, servers })
    }

    pub fn servers(&self) -> Vec<(String, TransportTypeId)> {
        let mut out: Vec<(String, TransportTypeId)> = self
            .servers
            .iter()
            .map(|(name, ty)| (name.clone(), *ty))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

impl ToolRegistry for McpToolRegistry {
    fn len(&self) -> usize {
        self.tools.len()
    }

    fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.tools.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn snapshot(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Debug, Clone)]
    struct FakeTransport {
        tools: Vec<McpToolDefinition>,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    #[async_trait]
    impl McpTransport for FakeTransport {
        async fn list_tools(&self) -> Result<Vec<McpToolDefinition>, McpTransportError> {
            Ok(self.tools.clone())
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
        let calls = Arc::new(Mutex::new(Vec::new()));
        let transport = Arc::new(FakeTransport {
            tools: vec![McpToolDefinition::new("echo").with_title("Echo")],
            calls: calls.clone(),
        }) as Arc<dyn McpTransport>;

        let reg = McpToolRegistry::from_transports([(cfg("s1"), transport)])
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
                &carve_state::Context::new(&serde_json::json!({}), "call", "test"),
            )
            .await
            .unwrap();
        assert!(res.is_success());

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "echo");
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
        let transport = Arc::new(FakeTransport {
            tools: vec![McpToolDefinition::new("a-b"), McpToolDefinition::new("a_b")],
            calls: Arc::new(Mutex::new(Vec::new())),
        }) as Arc<dyn McpTransport>;

        let err = McpToolRegistry::from_transports([(cfg("s1"), transport)])
            .await
            .err()
            .unwrap();
        assert!(matches!(err, McpToolRegistryError::ToolIdConflict(_)));
    }
}
