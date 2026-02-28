use crate::SKILLS_RUNTIME_PLUGIN_ID;
use async_trait::async_trait;
use tirea_contract::runtime::plugin::agent::{AgentBehavior, ReadOnlyContext};
use tirea_contract::runtime::plugin::phase::action::Action;

/// Placeholder plugin for activated skill state.
///
/// Skill instructions are delivered as user messages via `ToolExecutionEffect`
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
impl AgentBehavior for SkillRuntimePlugin {
    fn id(&self) -> &str {
        SKILLS_RUNTIME_PLUGIN_ID
    }

    async fn before_inference(&self, _ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        // No-op: skill content is delivered via ToolExecutionEffect user messages and tool results.
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::runtime::plugin::phase::Phase;
    use tirea_contract::RunConfig;
    use tirea_state::DocCell;

    #[tokio::test]
    async fn plugin_does_not_inject_system_context() {
        let state = json!({
            "skills": {
                "active": ["a"],
                "instructions": {"a": "Do X"},
                "references": {},
                "scripts": {}
            }
        });
        let p = SkillRuntimePlugin::new();
        let config = RunConfig::new();
        let doc = DocCell::new(state);
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        assert!(
            actions.is_empty(),
            "runtime plugin should not inject system context"
        );
    }
}
