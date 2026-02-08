use crate::plugin::AgentPlugin;
use crate::skills::{
    LoadSkillReferenceTool, SkillActivateTool, SkillDiscoveryPlugin, SkillPlugin, SkillRegistry,
    SkillRuntimePlugin, SkillScriptTool,
};
use crate::traits::tool::Tool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Errors returned when wiring the skills subsystem into an agent.
#[derive(Debug, thiserror::Error)]
pub enum SkillSubsystemError {
    #[error("tool id already registered: {0}")]
    ToolIdConflict(String),
}

/// High-level facade for wiring skills into an agent.
///
/// Callers should prefer this over manually instantiating the tools/plugins so:
/// - tool ids stay consistent
/// - plugin ordering is stable (discovery first, runtime second)
#[derive(Debug, Clone)]
pub struct SkillSubsystem {
    registry: Arc<SkillRegistry>,
}

impl SkillSubsystem {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self { registry }
    }

    pub fn from_roots(roots: Vec<PathBuf>) -> Self {
        Self::new(Arc::new(SkillRegistry::new(roots)))
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self::new(Arc::new(SkillRegistry::from_root(root)))
    }

    pub fn registry(&self) -> Arc<SkillRegistry> {
        self.registry.clone()
    }

    /// Build the combined skills plugin (discovery + runtime).
    pub fn plugin(&self) -> Arc<dyn AgentPlugin> {
        let discovery = SkillDiscoveryPlugin::new(self.registry.clone());
        SkillPlugin::new(discovery).boxed()
    }

    /// Build only the runtime plugin (inject activated skills).
    pub fn runtime_plugin(&self) -> Arc<dyn AgentPlugin> {
        Arc::new(SkillRuntimePlugin::new())
    }

    /// Construct the skills tools map.
    ///
    /// Tool ids:
    /// - `skill`
    /// - `load_skill_reference`
    /// - `skill_script`
    pub fn tools(&self) -> HashMap<String, Arc<dyn Tool>> {
        let mut out: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        // These inserts cannot conflict inside an empty map.
        let _ = self.extend_tools(&mut out);
        out
    }

    /// Add skills tools to an existing tool map.
    ///
    /// Returns an error if any tool id is already present.
    pub fn extend_tools(
        &self,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<(), SkillSubsystemError> {
        let reg = self.registry.clone();
        let tool_defs: Vec<Arc<dyn Tool>> = vec![
            Arc::new(SkillActivateTool::new(reg.clone())),
            Arc::new(LoadSkillReferenceTool::new(reg.clone())),
            Arc::new(SkillScriptTool::new(reg)),
        ];

        for t in tool_defs {
            let id = t.descriptor().id.clone();
            if tools.contains_key(&id) {
                return Err(SkillSubsystemError::ToolIdConflict(id));
            }
            tools.insert(id, t);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::execute::execute_single_tool;
    use crate::phase::{Phase, StepContext};
    use crate::session::Session;
    use crate::types::{Message, ToolCall};
    use crate::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use serde_json::json;
    use serde_json::Value;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn descriptor(&self) -> crate::traits::tool::ToolDescriptor {
            crate::traits::tool::ToolDescriptor::new("skill", "x", "x").with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &carve_state::Context<'_>,
        ) -> Result<crate::traits::tool::ToolResult, crate::traits::tool::ToolError> {
            Ok(crate::traits::tool::ToolResult::success("skill", json!({})))
        }
    }

    #[test]
    fn subsystem_extend_tools_detects_conflict() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(root.join("s1").join("SKILL.md"), "Body").unwrap();

        let sys = SkillSubsystem::from_root(root);
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        tools.insert("skill".to_string(), Arc::new(DummyTool));
        let err = sys.extend_tools(&mut tools).unwrap_err();
        assert!(err.to_string().contains("tool id already registered"));
    }

    #[test]
    fn subsystem_tools_returns_expected_ids() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(root.join("s1").join("SKILL.md"), "Body").unwrap();

        let sys = SkillSubsystem::from_root(root);
        let tools = sys.tools();
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn subsystem_extend_tools_inserts_tools_into_existing_map() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(root.join("s1").join("SKILL.md"), "Body").unwrap();

        let sys = SkillSubsystem::from_root(root);
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        tools.insert("other".to_string(), Arc::new(DummyOtherTool));
        sys.extend_tools(&mut tools).unwrap();
        assert!(tools.contains_key("other"));
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));
        assert_eq!(tools.len(), 4);
    }

    #[derive(Debug)]
    struct DummyOtherTool;

    #[async_trait]
    impl Tool for DummyOtherTool {
        fn descriptor(&self) -> crate::traits::tool::ToolDescriptor {
            crate::traits::tool::ToolDescriptor::new("other", "x", "x").with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &carve_state::Context<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("other", json!({})))
        }
    }

    #[tokio::test]
    async fn subsystem_plugin_injects_catalog_and_activated_skill() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("docx").join("references")).unwrap();

        let mut f = fs::File::create(root.join("docx").join("SKILL.md")).unwrap();
        writeln!(
            f,
            "{}",
            r#"---
name: DOCX Processing
description: DOCX guidance
---
Use docx-js for new documents.
"#
        )
        .unwrap();

        let sys = SkillSubsystem::from_root(root);
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        sys.extend_tools(&mut tools).unwrap();

        // Activate the skill via the registered "skill" tool.
        let session = Session::with_initial_state("s", json!({})).with_message(Message::user("hi"));
        let state = session.rebuild_state().unwrap();
        let call = ToolCall::new("call_1", "skill", json!({"skill": "docx"}));
        let activate_tool = tools.get("skill").unwrap().as_ref();
        let exec = execute_single_tool(Some(activate_tool), &call, &state).await;
        assert!(exec.result.is_success());
        let session = session.with_patch(exec.patch.unwrap());

        // Run the subsystem plugin and verify both discovery and runtime injections exist.
        let plugin = sys.plugin();
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill id=\"docx\">"));
        assert!(step.system_context[1].contains("Use docx-js for new documents."));
    }
}
