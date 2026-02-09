use crate::plugin::AgentPlugin;
use crate::skills::{
    SkillDiscoveryPlugin, SkillPlugin, SkillRegistry, SkillRuntimePlugin, SkillSubsystem,
};
use crate::traits::tool::Tool;
use crate::AgentConfig;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillsMode {
    Disabled,
    DiscoveryAndRuntime,
    DiscoveryOnly,
    RuntimeOnly,
}

#[derive(Debug, Clone)]
pub struct SkillsConfig {
    pub mode: SkillsMode,
    pub discovery_max_entries: usize,
    pub discovery_max_chars: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            mode: SkillsMode::DiscoveryAndRuntime,
            discovery_max_entries: 32,
            discovery_max_chars: 16 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsWiringError {
    #[error("skills tool id already registered: {0}")]
    SkillsToolIdConflict(String),

    #[error("skills plugin already installed: {0}")]
    SkillsPluginAlreadyInstalled(String),
}

#[derive(Debug, Clone)]
pub struct AgentOs {
    skills_registry: Option<Arc<SkillRegistry>>,
    skills: SkillsConfig,
}

#[derive(Debug, Clone)]
pub struct AgentOsBuilder {
    skills_roots: Vec<PathBuf>,
    skills: SkillsConfig,
}

impl AgentOsBuilder {
    pub fn new() -> Self {
        Self {
            skills_roots: Vec::new(),
            skills: SkillsConfig::default(),
        }
    }

    pub fn with_skills_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.skills_roots.push(root.into());
        self
    }

    pub fn with_skills_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.skills_roots = roots;
        self
    }

    pub fn with_skills_config(mut self, cfg: SkillsConfig) -> Self {
        self.skills = cfg;
        self
    }

    pub fn build(self) -> AgentOs {
        let skills_registry = if self.skills_roots.is_empty() {
            None
        } else {
            Some(Arc::new(SkillRegistry::new(self.skills_roots)))
        };

        AgentOs {
            skills_registry,
            skills: self.skills,
        }
    }
}

impl Default for AgentOsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentOs {
    pub fn builder() -> AgentOsBuilder {
        AgentOsBuilder::new()
    }

    pub fn skill_registry(&self) -> Option<Arc<SkillRegistry>> {
        self.skills_registry.clone()
    }

    pub fn wire_skills_into(
        &self,
        mut config: AgentConfig,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
    ) -> Result<AgentConfig, AgentOsWiringError> {
        let Some(reg) = self.skills_registry.clone() else {
            return Ok(config);
        };

        if self.skills.mode == SkillsMode::Disabled {
            return Ok(config);
        }

        // Prevent duplicate plugin installation.
        let reserved_plugin_ids = ["skills", "skills_discovery", "skills_runtime"];
        if let Some(existing) = config
            .plugins
            .iter()
            .map(|p| p.id())
            .find(|id| reserved_plugin_ids.contains(id))
        {
            return Err(AgentOsWiringError::SkillsPluginAlreadyInstalled(
                existing.to_string(),
            ));
        }

        // Register skills tools.
        let skills = SkillSubsystem::new(reg.clone());
        skills
            .extend_tools(tools)
            .map_err(|e| AgentOsWiringError::SkillsToolIdConflict(e.to_string()))?;

        // Register skills plugins.
        let mut plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
        match self.skills.mode {
            SkillsMode::Disabled => {}
            SkillsMode::DiscoveryAndRuntime => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                plugins.push(SkillPlugin::new(discovery).boxed());
            }
            SkillsMode::DiscoveryOnly => {
                let discovery = SkillDiscoveryPlugin::new(reg).with_limits(
                    self.skills.discovery_max_entries,
                    self.skills.discovery_max_chars,
                );
                plugins.push(Arc::new(discovery));
            }
            SkillsMode::RuntimeOnly => {
                plugins.push(Arc::new(SkillRuntimePlugin::new()));
            }
        }

        // Prepend skills plugins so skill context appears early and ordering is deterministic.
        plugins.extend(config.plugins);
        config.plugins = plugins;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::{Phase, StepContext};
    use crate::session::Session;
    use crate::traits::tool::ToolDescriptor;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn make_skills_root() -> (TempDir, PathBuf) {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s1")).unwrap();
        fs::write(
            root.join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: ok\n---\nDo X\n",
        )
        .unwrap();
        (td, root)
    }

    #[tokio::test]
    async fn wire_skills_inserts_tools_and_plugin() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder().with_skills_root(root).build();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));

        assert_eq!(cfg.plugins.len(), 1);
        assert_eq!(cfg.plugins[0].id(), "skills");

        // Verify injection does not panic and includes catalog.
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
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert_eq!(step.system_context.len(), 2);
        assert!(step.system_context[0].contains("<available_skills>"));
        assert!(step.system_context[1].contains("<skill id=\"s1\">"));
    }

    #[tokio::test]
    async fn wire_skills_runtime_only_injects_active_skills_without_catalog() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_root(root)
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::RuntimeOnly,
                ..SkillsConfig::default()
            })
            .build();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg = os.wire_skills_into(cfg, &mut tools).unwrap();

        assert_eq!(cfg.plugins.len(), 1);
        assert_eq!(cfg.plugins[0].id(), "skills_runtime");

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
        cfg.plugins[0]
            .on_phase(Phase::BeforeInference, &mut step)
            .await;
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("<skill id=\"s1\">"));
    }

    #[test]
    fn wire_skills_disabled_is_noop() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_root(root)
            .with_skills_config(SkillsConfig {
                mode: SkillsMode::Disabled,
                ..SkillsConfig::default()
            })
            .build();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini");
        let cfg2 = os.wire_skills_into(cfg.clone(), &mut tools).unwrap();

        assert!(tools.is_empty());
        assert_eq!(cfg2.plugins.len(), cfg.plugins.len());
    }

    #[derive(Debug)]
    struct FakeSkillsPlugin;

    #[async_trait::async_trait]
    impl AgentPlugin for FakeSkillsPlugin {
        fn id(&self) -> &str {
            "skills"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
    }

    #[test]
    fn wire_skills_errors_if_plugin_already_installed() {
        let (_td, root) = make_skills_root();
        let os = AgentOs::builder().with_skills_root(root).build();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(FakeSkillsPlugin));

        let err = os.wire_skills_into(cfg, &mut tools).unwrap_err();
        assert!(err.to_string().contains("skills plugin already installed"));
    }
}
