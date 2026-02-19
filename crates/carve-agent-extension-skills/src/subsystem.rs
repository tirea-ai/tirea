use crate::{
    LoadSkillResourceTool, Skill, SkillActivateTool, SkillDiscoveryPlugin, SkillPlugin,
    SkillRuntimePlugin, SkillScriptTool,
};
use carve_agent_contract::plugin::AgentPlugin;
use carve_agent_contract::tool::Tool;
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
/// ```ignore
/// use carve_agent::contracts::tool::Tool;
/// use carve_agent::extensions::skills::{FsSkill, SkillSubsystem};
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// // 1) Discover skills and wire into subsystem.
/// let result = FsSkill::discover("skills").unwrap();
/// let skills = SkillSubsystem::new(FsSkill::into_arc_skills(result.skills));
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
    skills: HashMap<String, Arc<dyn Skill>>,
}

impl SkillSubsystem {
    pub fn new(skills: Vec<Arc<dyn Skill>>) -> Self {
        let mut map: HashMap<String, Arc<dyn Skill>> = HashMap::new();
        for skill in skills {
            map.insert(skill.meta().id.clone(), skill);
        }
        Self { skills: map }
    }

    pub fn from_map(skills: HashMap<String, Arc<dyn Skill>>) -> Self {
        Self { skills }
    }

    pub fn skills(&self) -> &HashMap<String, Arc<dyn Skill>> {
        &self.skills
    }

    fn skills_list(&self) -> Vec<Arc<dyn Skill>> {
        let mut list: Vec<_> = self.skills.values().cloned().collect();
        list.sort_by(|a, b| a.meta().id.cmp(&b.meta().id));
        list
    }

    /// Build the combined skills plugin (discovery + runtime).
    pub fn plugin(&self) -> Arc<dyn AgentPlugin> {
        let discovery = SkillDiscoveryPlugin::new(self.skills_list());
        SkillPlugin::new(discovery).boxed()
    }

    /// Build only the runtime plugin (inject activated skills).
    pub fn runtime_plugin(&self) -> Arc<dyn AgentPlugin> {
        Arc::new(SkillRuntimePlugin::new())
    }

    /// Build only the discovery plugin.
    pub fn discovery_plugin(&self) -> SkillDiscoveryPlugin {
        SkillDiscoveryPlugin::new(self.skills_list())
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
    pub fn extend_tools(
        &self,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<(), SkillSubsystemError> {
        let skills = self.skills.clone();
        let tool_defs: Vec<Arc<dyn Tool>> = vec![
            Arc::new(SkillActivateTool::new(skills.clone())),
            Arc::new(LoadSkillResourceTool::new(skills.clone())),
            Arc::new(SkillScriptTool::new(skills)),
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
    use crate::{FsSkill, SKILL_ACTIVATE_TOOL_ID, SKILL_LOAD_RESOURCE_TOOL_ID, SKILL_SCRIPT_TOOL_ID};
    use async_trait::async_trait;
    use carve_agent_contract::runtime::phase::{Phase, StepContext};
    use carve_agent_contract::state::AgentState;
    use carve_agent_contract::state::{Message, ToolCall};
    use carve_agent_contract::tool::{ToolDescriptor, ToolError, ToolResult};
    use carve_agent_contract::AgentState as ContextAgentState;
    use carve_state::TrackedPatch;
    use serde_json::json;
    use serde_json::Value;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    struct LocalToolExecution {
        result: ToolResult,
        patch: Option<TrackedPatch>,
    }

    async fn execute_single_tool(
        tool: Option<&dyn Tool>,
        call: &ToolCall,
        state: &Value,
    ) -> LocalToolExecution {
        let Some(tool) = tool else {
            return LocalToolExecution {
                result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
                patch: None,
            };
        };

        let ctx = ContextAgentState::new_transient(state, &call.id, format!("tool:{}", call.name));
        let result = match tool.execute(call.arguments.clone(), &ctx.as_tool_call_context()).await {
            Ok(r) => r,
            Err(e) => ToolResult::error(&call.name, e.to_string()),
        };
        let patch = ctx.take_patch();
        let patch = if patch.patch().is_empty() {
            None
        } else {
            Some(patch)
        };
        LocalToolExecution { result, patch }
    }

    #[derive(Debug)]
    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn descriptor(&self) -> carve_agent_contract::tool::ToolDescriptor {
            carve_agent_contract::tool::ToolDescriptor::new(SKILL_ACTIVATE_TOOL_ID, "x", "x")
                .with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &carve_agent_contract::context::ToolCallContext<'_>,
        ) -> Result<carve_agent_contract::tool::ToolResult, carve_agent_contract::tool::ToolError>
        {
            Ok(carve_agent_contract::tool::ToolResult::success(
                SKILL_ACTIVATE_TOOL_ID,
                json!({}),
            ))
        }
    }

    fn make_subsystem() -> (TempDir, SkillSubsystem) {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        let sys = SkillSubsystem::new(FsSkill::into_arc_skills(result.skills));
        (td, sys)
    }

    #[test]
    fn subsystem_extend_tools_detects_conflict() {
        let (_td, sys) = make_subsystem();
        let mut tools = HashMap::<String, Arc<dyn Tool>>::new();
        tools.insert(SKILL_ACTIVATE_TOOL_ID.to_string(), Arc::new(DummyTool));
        let err = sys.extend_tools(&mut tools).unwrap_err();
        assert!(err.to_string().contains("tool id already registered"));
    }

    #[test]
    fn subsystem_tools_returns_expected_ids() {
        let (_td, sys) = make_subsystem();
        let tools = sys.tools();
        assert!(tools.contains_key(SKILL_ACTIVATE_TOOL_ID));
        assert!(tools.contains_key(SKILL_LOAD_RESOURCE_TOOL_ID));
        assert!(tools.contains_key(SKILL_SCRIPT_TOOL_ID));
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn subsystem_extend_tools_inserts_tools_into_existing_map() {
        let (_td, sys) = make_subsystem();
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
        fn descriptor(&self) -> carve_agent_contract::tool::ToolDescriptor {
            carve_agent_contract::tool::ToolDescriptor::new("other", "x", "x")
                .with_parameters(json!({}))
        }

        async fn execute(
            &self,
            _args: Value,
            _ctx: &carve_agent_contract::context::ToolCallContext<'_>,
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

        let result = FsSkill::discover(root).unwrap();
        let sys = SkillSubsystem::new(FsSkill::into_arc_skills(result.skills));
        let tools = sys.tools();

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

        // Run the subsystem plugin and verify discovery catalog is injected.
        let plugin = sys.plugin();
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        let doc = json!({});
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;

        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<available_skills>"));
    }
}
