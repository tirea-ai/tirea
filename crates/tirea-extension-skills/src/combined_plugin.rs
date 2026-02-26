use crate::{SkillDiscoveryPlugin, SkillRuntimePlugin, SKILLS_PLUGIN_ID};
use async_trait::async_trait;
use std::sync::Arc;
use tirea_contract::runtime::plugin::phase::{
    AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
    BeforeToolExecuteContext, RunEndContext, RunStartContext, StepEndContext, StepStartContext,
};
use tirea_contract::runtime::plugin::AgentPlugin;

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

    async fn run_start(&self, ctx: &mut RunStartContext<'_, '_>) {
        self.discovery.run_start(ctx).await;
        self.runtime.run_start(ctx).await;
    }

    async fn step_start(&self, ctx: &mut StepStartContext<'_, '_>) {
        self.discovery.step_start(ctx).await;
        self.runtime.step_start(ctx).await;
    }

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        // Keep ordering stable: catalog first (enables selection), then active skill content.
        self.discovery.before_inference(ctx).await;
        self.runtime.before_inference(ctx).await;
    }

    async fn after_inference(&self, ctx: &mut AfterInferenceContext<'_, '_>) {
        self.discovery.after_inference(ctx).await;
        self.runtime.after_inference(ctx).await;
    }

    async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
        self.discovery.before_tool_execute(ctx).await;
        self.runtime.before_tool_execute(ctx).await;
    }

    async fn after_tool_execute(&self, ctx: &mut AfterToolExecuteContext<'_, '_>) {
        self.discovery.after_tool_execute(ctx).await;
        self.runtime.after_tool_execute(ctx).await;
    }

    async fn step_end(&self, ctx: &mut StepEndContext<'_, '_>) {
        self.discovery.step_end(ctx).await;
        self.runtime.step_end(ctx).await;
    }

    async fn run_end(&self, ctx: &mut RunEndContext<'_, '_>) {
        self.discovery.run_end(ctx).await;
        self.runtime.run_end(ctx).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FsSkill, InMemorySkillRegistry, Skill, SkillRegistry};
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;
    use tirea_contract::testing::TestFixture;
    use tirea_contract::runtime::tool_call::ToolDescriptor;

    #[tokio::test]
    async fn combined_plugin_injects_catalog_only() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nDo X\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        let skills: Vec<Arc<dyn Skill>> = FsSkill::into_arc_skills(result.skills);
        let registry: Arc<dyn SkillRegistry> = Arc::new(InMemorySkillRegistry::from_skills(skills));
        let discovery = SkillDiscoveryPlugin::new(registry);
        let plugin = SkillPlugin::new(discovery);

        let fixture = TestFixture::new_with_state(json!({
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
        }));
        let mut step = fixture.step(vec![ToolDescriptor::new("t", "t", "t")]);
        let mut before = tirea_contract::runtime::plugin::phase::BeforeInferenceContext::new(&mut step);
        plugin.before_inference(&mut before).await;

        // Only discovery catalog is injected; runtime plugin no longer injects system context.
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<available_skills>"));
    }
}
