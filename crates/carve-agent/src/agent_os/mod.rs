use crate::plugin::AgentPlugin;
use crate::r#loop::{run_loop, run_loop_stream};
mod registry;
pub use registry::{AgentRegistry, AgentRegistryError, ToolRegistry, ToolRegistryError};
use crate::skills::{
    SkillDiscoveryPlugin, SkillPlugin, SkillRegistry, SkillRuntimePlugin, SkillSubsystem,
    SkillSubsystemError,
};
use crate::traits::tool::Tool;
use crate::{AgentConfig, AgentDefinition, AgentEvent, AgentLoopError, Session};
use genai::Client;
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

#[derive(Debug, thiserror::Error)]
pub enum AgentOsBuildError {
    #[error(transparent)]
    Tools(#[from] ToolRegistryError),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsResolveError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error(transparent)]
    Wiring(#[from] AgentOsWiringError),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOsRunError {
    #[error(transparent)]
    Resolve(#[from] AgentOsResolveError),

    #[error(transparent)]
    Loop(#[from] AgentLoopError),
}

#[derive(Clone)]
pub struct AgentOs {
    client: Client,
    agents: AgentRegistry,
    base_tools: ToolRegistry,
    skills_registry: Option<Arc<SkillRegistry>>,
    skills: SkillsConfig,
}

#[derive(Clone)]
pub struct AgentOsBuilder {
    client: Option<Client>,
    agents: HashMap<String, AgentDefinition>,
    base_tools: HashMap<String, Arc<dyn Tool>>,
    skills_roots: Vec<PathBuf>,
    skills: SkillsConfig,
}

impl std::fmt::Debug for AgentOs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOs")
            .field("client", &"[genai::Client]")
            .field("agents", &self.agents.len())
            .field("base_tools", &self.base_tools.len())
            .field("skills_registry", &self.skills_registry.is_some())
            .field("skills", &self.skills)
            .finish()
    }
}

impl std::fmt::Debug for AgentOsBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOsBuilder")
            .field("client", &self.client.is_some())
            .field("agents", &self.agents.len())
            .field("base_tools", &self.base_tools.len())
            .field("skills_roots", &self.skills_roots)
            .field("skills", &self.skills)
            .finish()
    }
}

impl AgentOsBuilder {
    pub fn new() -> Self {
        Self {
            client: None,
            agents: HashMap::new(),
            base_tools: HashMap::new(),
            skills_roots: Vec::new(),
            skills: SkillsConfig::default(),
        }
    }

    pub fn with_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    pub fn with_agent(mut self, agent_id: impl Into<String>, def: AgentDefinition) -> Self {
        let agent_id = agent_id.into();
        let mut def = def;
        // The registry key is the canonical id to avoid mismatches.
        def.id = agent_id.clone();
        self.agents.insert(agent_id, def);
        self
    }

    pub fn with_tools(mut self, tools: HashMap<String, Arc<dyn Tool>>) -> Self {
        self.base_tools = tools;
        self
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

    pub fn build(self) -> Result<AgentOs, AgentOsBuildError> {
        let skills_registry = if self.skills_roots.is_empty() {
            None
        } else {
            Some(Arc::new(SkillRegistry::new(self.skills_roots)))
        };

        let mut base_tools = ToolRegistry::new();
        base_tools.extend_named(self.base_tools)?;

        let mut agents = AgentRegistry::new();
        agents.extend_upsert(self.agents);

        Ok(AgentOs {
            client: self.client.unwrap_or_default(),
            agents,
            base_tools,
            skills_registry,
            skills: self.skills,
        })
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

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub fn skill_registry(&self) -> Option<Arc<SkillRegistry>> {
        self.skills_registry.clone()
    }

    pub fn agent(&self, agent_id: &str) -> Option<AgentDefinition> {
        self.agents.get(agent_id)
    }

    pub fn tools(&self) -> HashMap<String, Arc<dyn Tool>> {
        self.base_tools.to_map()
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
        skills.extend_tools(tools).map_err(|e| match e {
            SkillSubsystemError::ToolIdConflict(id) => AgentOsWiringError::SkillsToolIdConflict(id),
        })?;

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

    pub fn resolve(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<(AgentConfig, HashMap<String, Arc<dyn Tool>>, Session), AgentOsResolveError> {
        let def = self
            .agents
            .get(agent_id)
            .ok_or_else(|| AgentOsResolveError::AgentNotFound(agent_id.to_string()))?;

        let mut tools = self.base_tools.to_map();
        let cfg = self.wire_skills_into(def, &mut tools)?;
        Ok((cfg, tools, session))
    }

    pub async fn run(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<(Session, String), AgentOsRunError> {
        let (cfg, tools, session) = self.resolve(agent_id, session)?;
        let (session, text) = run_loop(&self.client, &cfg, session, &tools).await?;
        Ok((session, text))
    }

    pub fn run_stream(
        &self,
        agent_id: &str,
        session: Session,
    ) -> Result<impl futures::Stream<Item = AgentEvent> + Send, AgentOsResolveError> {
        let (cfg, tools, session) = self.resolve(agent_id, session)?;
        Ok(run_loop_stream(self.client.clone(), cfg, session, tools))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::{Phase, StepContext};
    use crate::session::Session;
    use crate::traits::tool::ToolDescriptor;
    use crate::traits::tool::{ToolError, ToolResult};
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
        let os = AgentOs::builder().with_skills_root(root).build().unwrap();

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
            .build()
            .unwrap();

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
            .build()
            .unwrap();

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
        let os = AgentOs::builder().with_skills_root(root).build().unwrap();

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let cfg = AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(FakeSkillsPlugin));

        let err = os.wire_skills_into(cfg, &mut tools).unwrap_err();
        assert!(err.to_string().contains("skills plugin already installed"));
    }

    #[test]
    fn resolve_errors_if_agent_missing() {
        let os = AgentOs::builder().build().unwrap();
        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("missing", session).err().unwrap();
        assert!(matches!(err, AgentOsResolveError::AgentNotFound(_)));
    }

    #[tokio::test]
    async fn resolve_wires_skills_and_preserves_base_tools() {
        #[derive(Debug)]
        struct BaseTool;

        #[async_trait::async_trait]
        impl Tool for BaseTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("base_tool", "Base Tool", "Base Tool")
            }

            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &carve_state::Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("base_tool", json!({"ok": true})))
            }
        }

        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_root(root)
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .with_tools(HashMap::from([(
                "base_tool".to_string(),
                Arc::new(BaseTool) as Arc<dyn Tool>,
            )]))
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let (cfg, tools, _session) = os.resolve("a1", session).unwrap();

        assert_eq!(cfg.id, "a1");
        assert!(tools.contains_key("base_tool"));
        assert!(tools.contains_key("skill"));
        assert!(tools.contains_key("load_skill_reference"));
        assert!(tools.contains_key("skill_script"));
        assert_eq!(cfg.plugins.len(), 1);
    }

    #[tokio::test]
    async fn run_and_run_stream_work_without_llm_when_skip_inference() {
        #[derive(Debug)]
        struct SkipInferencePlugin;

        #[async_trait::async_trait]
        impl AgentPlugin for SkipInferencePlugin {
            fn id(&self) -> &str {
                "skip_inference"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeInference {
                    step.skip_inference = true;
                }
            }
        }

        let def = AgentDefinition::new("gpt-4o-mini").with_plugin(Arc::new(SkipInferencePlugin));
        let os = AgentOs::builder().with_agent("a1", def).build().unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let (_session, text) = os.run("a1", session).await.unwrap();
        assert_eq!(text, "");

        let session = Session::with_initial_state("s2", json!({}));
        let mut stream = os.run_stream("a1", session).unwrap();
        let ev = futures::StreamExt::next(&mut stream).await.unwrap();
        assert!(matches!(ev, AgentEvent::Done { .. }));
    }

    #[tokio::test]
    async fn resolve_errors_on_skills_tool_id_conflict() {
        #[derive(Debug)]
        struct ConflictingTool;

        #[async_trait::async_trait]
        impl Tool for ConflictingTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("skill", "Conflicting", "Conflicting")
            }

            async fn execute(
                &self,
                _args: serde_json::Value,
                _ctx: &carve_state::Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("skill", json!({"ok": true})))
            }
        }

        let (_td, root) = make_skills_root();
        let os = AgentOs::builder()
            .with_skills_root(root)
            .with_agent("a1", AgentDefinition::new("gpt-4o-mini"))
            .with_tools(HashMap::from([(
                "skill".to_string(),
                Arc::new(ConflictingTool) as Arc<dyn Tool>,
            )]))
            .build()
            .unwrap();

        let session = Session::with_initial_state("s", json!({}));
        let err = os.resolve("a1", session).err().unwrap();
        assert!(matches!(
            err,
            AgentOsResolveError::Wiring(AgentOsWiringError::SkillsToolIdConflict(ref id))
            if id == "skill"
        ));
    }
}
