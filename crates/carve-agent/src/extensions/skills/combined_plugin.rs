use crate::contracts::agent_plugin::AgentPlugin;
use crate::contracts::context::AgentState as ContextAgentState;
use crate::contracts::phase::{Phase, StepContext};
use crate::extensions::skills::{SkillDiscoveryPlugin, SkillRuntimePlugin, SKILLS_PLUGIN_ID};
use async_trait::async_trait;
use std::sync::Arc;

/// Single plugin wrapper that injects both:
/// - the skills catalog (discovery)
/// - activated skill instructions/materials (runtime)
///
/// This is a convenience so callers can register one plugin instead of two.
#[derive(Debug, Clone)]
pub struct SkillPlugin {
    discovery: SkillDiscoveryPlugin,
    runtime: SkillRuntimePlugin,
}

impl SkillPlugin {
    pub fn new(discovery: SkillDiscoveryPlugin) -> Self {
        Self {
            discovery,
            runtime: SkillRuntimePlugin::new(),
        }
    }

    pub fn with_runtime(mut self, runtime: SkillRuntimePlugin) -> Self {
        self.runtime = runtime;
        self
    }

    pub fn boxed(self) -> Arc<dyn AgentPlugin> {
        Arc::new(self)
    }
}

#[async_trait]
impl AgentPlugin for SkillPlugin {
    fn id(&self) -> &str {
        SKILLS_PLUGIN_ID
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &ContextAgentState<'_>) {
        // Keep ordering stable: catalog first (enables selection), then active skill content.
        self.discovery.on_phase(phase, step, ctx).await;
        self.runtime.on_phase(phase, step, ctx).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::conversation::AgentState;
    use crate::contracts::traits::tool::ToolDescriptor;
    use crate::extensions::skills::{FsSkillRegistry, SkillRegistry};
    use crate::contracts::context::AgentState as ContextAgentState;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn combined_plugin_injects_catalog_and_active_skills() {
        let doc = json!({});
        let ctx = ContextAgentState::new(&doc, "test", "test");
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nDo X\n",
        )
        .unwrap();

        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        let discovery = SkillDiscoveryPlugin::new(reg);
        let plugin = SkillPlugin::new(discovery);

        let thread = AgentState::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["s1"],
                    "instructions": {"s1": "Do X"},
                    "references": {
                        "s1:references/a.md": {
                            "skill":"s1",
                            "path":"references/a.md",
                            "sha256":"x",
                            "truncated":false,
                            "content":"A",
                            "bytes":1
                        }
                    },
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        plugin
            .on_phase(Phase::BeforeInference, &mut step, &ctx)
            .await;

        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill_reference"));
    }
}
