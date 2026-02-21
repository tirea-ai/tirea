use crate::SKILLS_RUNTIME_PLUGIN_ID;
use async_trait::async_trait;
use tirea_contract::plugin::phase::Phase;
use tirea_contract::plugin::phase::StepContext;
use tirea_contract::plugin::AgentPlugin;

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
    use serde_json::json;
    use tirea_contract::testing::TestFixture;
    use tirea_contract::tool::ToolDescriptor;

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
