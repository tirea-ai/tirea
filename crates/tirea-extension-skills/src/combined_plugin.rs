use crate::{SkillDiscoveryPlugin, SkillRuntimePlugin, SKILLS_PLUGIN_ID};
use async_trait::async_trait;
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::plugin::phase::{Phase, StepContext};
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        // Keep ordering stable: catalog first (enables selection), then active skill content.
        self.discovery.on_phase(phase, step).await;
        self.runtime.on_phase(phase, step).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FsSkill, InMemorySkillRegistry, Skill, SkillRegistry};
    use tirea_contract::testing::TestFixture;
    use tirea_contract::tool::ToolDescriptor;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

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
        plugin
            .on_phase(Phase::BeforeInference, &mut step)
            .await;

        // Only discovery catalog is injected; runtime plugin no longer injects system context.
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<available_skills>"));
    }
}
