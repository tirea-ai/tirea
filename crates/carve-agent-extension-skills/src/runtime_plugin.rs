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
    use carve_agent_contract::context::ToolCallContext;
    use carve_agent_contract::state::Message;
    use carve_agent_contract::tool::ToolDescriptor;
    use carve_state::{DocCell, Op, ScopeState};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    struct TestFixture {
        doc: DocCell,
        ops: Mutex<Vec<Op>>,
        overlay: Arc<Mutex<Vec<Op>>>,
        scope: ScopeState,
        pending_messages: Mutex<Vec<Arc<Message>>>,
        messages: Vec<Arc<Message>>,
    }

    impl TestFixture {
        fn new_with_state(state: serde_json::Value) -> Self {
            Self {
                doc: DocCell::new(state),
                ops: Mutex::new(Vec::new()),
                overlay: Arc::new(Mutex::new(Vec::new())),
                scope: ScopeState::default(),
                pending_messages: Mutex::new(Vec::new()),
                messages: Vec::new(),
            }
        }

        fn ctx(&self) -> ToolCallContext<'_> {
            ToolCallContext::new(
                &self.doc,
                &self.ops,
                self.overlay.clone(),
                "test",
                "test",
                &self.scope,
                &self.pending_messages,
                None,
            )
        }

        fn step<'a>(&'a self, tools: Vec<ToolDescriptor>) -> StepContext<'a> {
            StepContext::new(self.ctx(), "test-thread", &self.messages, tools)
        }
    }

    #[tokio::test]
    async fn plugin_does_not_inject_system_context() {
        let fixture = TestFixture::new_with_state(json!({
            "skills": {
                "active": ["a"],
                "instructions": {"a": "Do X"},
                "references": {},
                "scripts": {}
            }
        }));
        let mut step = fixture.step(vec![ToolDescriptor::new("t", "t", "t")]);
        let p = SkillRuntimePlugin::new();
        p.on_phase(Phase::BeforeInference, &mut step).await;
        assert!(
            step.system_context.is_empty(),
            "runtime plugin should not inject system context"
        );
    }
}
