use crate::contracts::extension::plugin::AgentPlugin;
use crate::contracts::extension::traits::tool::Tool;
use crate::extensions::skills::{
    LoadSkillResourceTool, SkillActivateTool, SkillDiscoveryPlugin, SkillPlugin, SkillRegistry,
    SkillRuntimePlugin, SkillScriptTool,
};
use std::collections::HashMap;
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
///
/// # Example
///
/// ```no_run
/// use carve_agent::contracts::extension::traits::tool::Tool;
/// use carve_agent::extensions::skills::{FsSkillRegistry, SkillSubsystem};
/// use carve_agent::runtime::loop_runner::AgentConfig;
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// // 1) Build a registry and wire it into the subsystem.
/// let reg = FsSkillRegistry::discover_root("skills").unwrap();
/// let skills = SkillSubsystem::new(Arc::new(reg));
///
/// // 2) Register tools (skill activation + reference/script utilities).
/// let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
/// skills.extend_tools(&mut tools).unwrap();
///
/// // 3) Register the combined plugin: discovery catalog + runtime injection.
/// let config = AgentConfig::new("gpt-4o-mini").with_plugin(skills.plugin());
/// # let _ = config;
/// # let _ = tools;
/// ```
#[derive(Debug, Clone)]
pub struct SkillSubsystem {
    registry: Arc<dyn SkillRegistry>,
}

impl SkillSubsystem {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> Arc<dyn SkillRegistry> {
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
    /// - `SKILL_ACTIVATE_TOOL_ID`
    /// - `SKILL_LOAD_RESOURCE_TOOL_ID`
    /// - `SKILL_SCRIPT_TOOL_ID`
    pub fn tools(&self) -> HashMap<String, Arc<dyn Tool>> {
        let mut out: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        // These inserts cannot conflict inside an empty map.
        let _ = self.extend_tools(&mut out);
        out
    }

    /// Add skills tools to an existing tool map.
    ///
    /// Returns an error if any tool id is already present.
    ///
    /// ```no_run
    /// use carve_agent::contracts::extension::traits::tool::Tool;
    /// use carve_agent::extensions::skills::{FsSkillRegistry, SkillSubsystem};
    /// use std::collections::HashMap;
    /// use std::sync::Arc;
    ///
    /// let reg = FsSkillRegistry::discover_root("skills").unwrap();
    /// let skills = SkillSubsystem::new(Arc::new(reg));
    ///
    /// // Conflict example: `tools()` already contains the skill tool ids.
    /// let mut tools: HashMap<String, Arc<dyn Tool>> = skills.tools();
    /// let err = skills.extend_tools(&mut tools).unwrap_err();
    /// assert!(err.to_string().contains("tool id already registered"));
    /// ```
    pub fn extend_tools(
        &self,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<(), SkillSubsystemError> {
        let reg = self.registry.clone();
        let tool_defs: Vec<Arc<dyn Tool>> = vec![
            Arc::new(SkillActivateTool::new(reg.clone())),
            Arc::new(LoadSkillResourceTool::new(reg.clone())),
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
    use crate::contracts::conversation::AgentState;
    use crate::contracts::conversation::{Message, ToolCall};
    use crate::contracts::extension::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use crate::contracts::runtime::phase::{Phase, StepContext};
    use crate::contracts::AgentState as ContextAgentState;
    use crate::engine::tool_execution::execute_single_tool;
    use crate::extensions::skills::{
        FsSkillRegistry, SKILL_ACTIVATE_TOOL_ID, SKILL_LOAD_RESOURCE_TOOL_ID, SKILL_SCRIPT_TOOL_ID,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use serde_json::Value;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn descriptor(&self) -> crate::contracts::extension::traits::tool::ToolDescriptor {
            crate::contracts::extension::traits::tool::ToolDescriptor::new(
                SKILL_ACTIVATE_TOOL_ID,
                "x",
                "x",
            )
            .with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &ContextAgentState,
        ) -> Result<
            crate::contracts::extension::traits::tool::ToolResult,
            crate::contracts::extension::traits::tool::ToolError,
        > {
            Ok(
                crate::contracts::extension::traits::tool::ToolResult::success(
                    SKILL_ACTIVATE_TOOL_ID,
                    json!({}),
                ),
            )
        }
    }

    #[test]
    fn subsystem_extend_tools_detects_conflict() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let sys = SkillSubsystem::new(Arc::new(FsSkillRegistry::discover_root(root).unwrap()));
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        tools.insert(SKILL_ACTIVATE_TOOL_ID.to_string(), Arc::new(DummyTool));
        let err = sys.extend_tools(&mut tools).unwrap_err();
        assert!(err.to_string().contains("tool id already registered"));
    }

    #[test]
    fn subsystem_tools_returns_expected_ids() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let sys = SkillSubsystem::new(Arc::new(FsSkillRegistry::discover_root(root).unwrap()));
        let tools = sys.tools();
        assert!(tools.contains_key(SKILL_ACTIVATE_TOOL_ID));
        assert!(tools.contains_key(SKILL_LOAD_RESOURCE_TOOL_ID));
        assert!(tools.contains_key(SKILL_SCRIPT_TOOL_ID));
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn subsystem_extend_tools_inserts_tools_into_existing_map() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let sys = SkillSubsystem::new(Arc::new(FsSkillRegistry::discover_root(root).unwrap()));
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        tools.insert("other".to_string(), Arc::new(DummyOtherTool));
        sys.extend_tools(&mut tools).unwrap();
        assert!(tools.contains_key("other"));
        assert!(tools.contains_key(SKILL_ACTIVATE_TOOL_ID));
        assert!(tools.contains_key(SKILL_LOAD_RESOURCE_TOOL_ID));
        assert!(tools.contains_key(SKILL_SCRIPT_TOOL_ID));
        assert_eq!(tools.len(), 4);
    }

    #[derive(Debug)]
    struct DummyOtherTool;

    #[async_trait]
    impl Tool for DummyOtherTool {
        fn descriptor(&self) -> crate::contracts::extension::traits::tool::ToolDescriptor {
            crate::contracts::extension::traits::tool::ToolDescriptor::new("other", "x", "x")
                .with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &ContextAgentState,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("other", json!({})))
        }
    }

    #[tokio::test]
    async fn subsystem_plugin_injects_catalog_and_activated_skill() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("docx").join("references")).unwrap();
        fs::write(
            root.join("docx").join("references").join("DOCX-JS.md"),
            "Use docx-js for new documents.",
        )
        .unwrap();

        let mut f = fs::File::create(root.join("docx").join("SKILL.md")).unwrap();
        f.write_all(
            b"---\nname: docx\ndescription: DOCX guidance\n---\nUse docx-js for new documents.\n\n",
        )
        .unwrap();

        let sys = SkillSubsystem::new(Arc::new(FsSkillRegistry::discover_root(root).unwrap()));
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        sys.extend_tools(&mut tools).unwrap();

        // Activate the skill via the registered "skill" tool.
        let thread =
            AgentState::with_initial_state("s", json!({})).with_message(Message::user("hi"));
        let state = thread.rebuild_state().unwrap();
        let call = ToolCall::new("call_1", SKILL_ACTIVATE_TOOL_ID, json!({"skill": "docx"}));
        let activate_tool = tools.get(SKILL_ACTIVATE_TOOL_ID).unwrap().as_ref();
        let exec = execute_single_tool(Some(activate_tool), &call, &state).await;
        assert!(exec.result.is_success());
        let thread = thread.with_patch(exec.patch.unwrap());

        let state = thread.rebuild_state().unwrap();
        let call = ToolCall::new(
            "call_2",
            SKILL_LOAD_RESOURCE_TOOL_ID,
            json!({"skill": "docx", "path": "references/DOCX-JS.md"}),
        );
        let load_resource_tool = tools.get(SKILL_LOAD_RESOURCE_TOOL_ID).unwrap().as_ref();
        let exec = execute_single_tool(Some(load_resource_tool), &call, &state).await;
        assert!(exec.result.is_success());
        let thread = thread.with_patch(exec.patch.unwrap());

        // Run the subsystem plugin and verify both discovery and runtime injections exist.
        let plugin = sys.plugin();
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        let doc = json!({});
        let ctx = ContextAgentState::new_runtime(&doc, "test", "test");
        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;

        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill_reference"));
        assert!(step.system_context[1].contains("path=\"references/DOCX-JS.md\""));
    }
}
