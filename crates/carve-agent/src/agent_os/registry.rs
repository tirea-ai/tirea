use crate::traits::tool::Tool;
use crate::AgentDefinition;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ToolRegistryError {
    #[error("tool id already registered: {0}")]
    ToolIdConflict(String),

    #[error("tool id mismatch: key={key} descriptor.id={descriptor_id}")]
    ToolIdMismatch { key: String, descriptor_id: String },
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("len", &self.tools.len())
            .finish()
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.tools.keys()
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolRegistryError> {
        let id = tool.descriptor().id;
        if self.tools.contains_key(&id) {
            return Err(ToolRegistryError::ToolIdConflict(id));
        }
        self.tools.insert(id, tool);
        Ok(())
    }

    pub fn register_named(
        &mut self,
        id: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Result<(), ToolRegistryError> {
        let key = id.into();
        let descriptor_id = tool.descriptor().id;
        if key != descriptor_id {
            return Err(ToolRegistryError::ToolIdMismatch { key, descriptor_id });
        }
        if self.tools.contains_key(&key) {
            return Err(ToolRegistryError::ToolIdConflict(key));
        }
        self.tools.insert(key, tool);
        Ok(())
    }

    pub fn extend_named(
        &mut self,
        tools: HashMap<String, Arc<dyn Tool>>,
    ) -> Result<(), ToolRegistryError> {
        for (key, tool) in tools {
            self.register_named(key, tool)?;
        }
        Ok(())
    }

    pub fn into_map(self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools
    }

    pub fn to_map(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.tools.clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentRegistryError {
    #[error("agent id already registered: {0}")]
    AgentIdConflict(String),
}

#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    agents: HashMap<String, AgentDefinition>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<AgentDefinition> {
        self.agents.get(id).cloned()
    }

    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.agents.keys()
    }

    pub fn register(
        &mut self,
        agent_id: impl Into<String>,
        mut def: AgentDefinition,
    ) -> Result<(), AgentRegistryError> {
        let agent_id = agent_id.into();
        if self.agents.contains_key(&agent_id) {
            return Err(AgentRegistryError::AgentIdConflict(agent_id));
        }
        // The registry key is canonical to avoid mismatches.
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
        Ok(())
    }

    pub fn upsert(&mut self, agent_id: impl Into<String>, mut def: AgentDefinition) {
        let agent_id = agent_id.into();
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
    }

    pub fn extend_upsert(&mut self, defs: HashMap<String, AgentDefinition>) {
        for (id, def) in defs {
            self.upsert(id, def);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use async_trait::async_trait;
    use serde_json::json;

    #[derive(Debug)]
    struct T(&'static str);

    #[async_trait]
    impl Tool for T {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(self.0, self.0, self.0)
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _ctx: &carve_state::Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success(self.0, json!({})))
        }
    }

    #[test]
    fn tool_registry_register_uses_descriptor_id_and_detects_conflict() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(T("a"))).unwrap();
        let err = reg.register(Arc::new(T("a"))).err().unwrap();
        assert!(matches!(err, ToolRegistryError::ToolIdConflict(ref id) if id == "a"));
    }

    #[test]
    fn tool_registry_register_named_detects_mismatch() {
        let mut reg = ToolRegistry::new();
        let err = reg
            .register_named("x", Arc::new(T("y")))
            .err()
            .unwrap();
        assert!(matches!(err, ToolRegistryError::ToolIdMismatch { .. }));
    }

    #[test]
    fn agent_registry_register_canonicalizes_id() {
        let mut reg = AgentRegistry::new();
        reg.register("a1", AgentDefinition::with_id("not-a1", "gpt-4o-mini"))
            .unwrap();
        let def = reg.get("a1").unwrap();
        assert_eq!(def.id, "a1");
    }
}

