use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::skills::{SkillDiscoveryPlugin, SkillRuntimePlugin};
use async_trait::async_trait;
use serde_json::Value;
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
        "skills"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        // Keep ordering stable: catalog first (enables selection), then active skill content.
        self.discovery.on_phase(phase, step).await;
        self.runtime.on_phase(phase, step).await;
    }

    fn initial_data(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::skills::SkillRegistry;
    use crate::traits::tool::ToolDescriptor;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn combined_plugin_injects_catalog_and_active_skills() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nDo X\n",
        )
        .unwrap();

        let reg = Arc::new(SkillRegistry::from_root(root));
        let discovery = SkillDiscoveryPlugin::new(reg);
        let plugin = SkillPlugin::new(discovery);

        let session = Session::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["s1"],
                    "instructions": {"s1": "Do X"},
                    "references": {},
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        plugin.on_phase(Phase::BeforeInference, &mut step).await;

        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill id=\"s1\">"));
    }
}
