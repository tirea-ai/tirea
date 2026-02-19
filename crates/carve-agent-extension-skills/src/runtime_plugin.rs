use crate::SKILLS_RUNTIME_PLUGIN_ID;
use async_trait::async_trait;
use carve_agent_contract::plugin::AgentPlugin;
use carve_agent_contract::runtime::phase::Phase;
use carve_agent_contract::runtime::phase::StepContext;

/// Placeholder plugin for activated skill state.
///
/// Skill instructions are injected via `append_user_messages` (single injection path)
/// and tool results for references/scripts/assets are already visible in conversation
/// history. This plugin no longer injects system context to avoid token waste from
/// duplicate injection.
#[derive(Debug, Default, Clone)]
pub struct SkillRuntimePlugin;

impl SkillRuntimePlugin {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl AgentPlugin for SkillRuntimePlugin {
    fn id(&self) -> &str {
        SKILLS_RUNTIME_PLUGIN_ID
    }

    async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {
        // No-op: skill content is delivered via append_user_messages and tool results.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agent_contract::state::AgentState;
    use carve_agent_contract::tool::ToolDescriptor;
    use carve_agent_contract::AgentState as ContextAgentState;
    use serde_json::json;

    #[tokio::test]
    async fn plugin_does_not_inject_system_context() {
        let doc = json!({});
        let ctx = ContextAgentState::new_transient(&doc, "test", "test");
        let thread = AgentState::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["a"],
                    "instructions": {"a": "Do X"},
                    "references": {},
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        let p = SkillRuntimePlugin::new();
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        assert!(
            step.system_context.is_empty(),
            "runtime plugin should not inject system context"
        );
    }
}
