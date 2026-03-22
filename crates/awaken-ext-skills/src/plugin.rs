use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;

use awaken_contract::StateError;
use awaken_contract::contract::context_message::ContextMessage;
use awaken_contract::model::Phase;
use awaken_runtime::plugins::{Plugin, PluginDescriptor, PluginRegistrar};
use awaken_runtime::state::StateKeyOptions;
use awaken_runtime::{PhaseContext, PhaseHook, StateCommand};

use crate::registry::SkillRegistry;
use crate::skill::SkillMeta;
use crate::skill_md::parse_skill_md;
use crate::state::SkillState;
use crate::{SKILLS_ACTIVE_INSTRUCTIONS_PLUGIN_ID, SKILLS_DISCOVERY_PLUGIN_ID};

/// Injects a skills catalog into the LLM context so the model can discover and activate skills.
#[derive(Clone)]
pub struct SkillDiscoveryPlugin {
    registry: Arc<dyn SkillRegistry>,
    max_entries: usize,
    max_chars: usize,
}

impl std::fmt::Debug for SkillDiscoveryPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillDiscoveryPlugin")
            .field("max_entries", &self.max_entries)
            .field("max_chars", &self.max_chars)
            .finish_non_exhaustive()
    }
}

impl SkillDiscoveryPlugin {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self {
            registry,
            max_entries: 32,
            max_chars: 16 * 1024,
        }
    }

    pub fn with_limits(mut self, max_entries: usize, max_chars: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self.max_chars = max_chars.max(256);
        self
    }

    fn escape_text(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn render_catalog(&self, _active: &HashSet<String>) -> String {
        let mut metas: Vec<SkillMeta> = self
            .registry
            .snapshot()
            .values()
            .map(|s| s.meta().clone())
            .collect();

        if metas.is_empty() {
            return String::new();
        }

        metas.sort_by(|a, b| a.id.cmp(&b.id));

        let total = metas.len();
        let mut out = String::new();
        out.push_str("<available_skills>\n");

        let mut shown = 0usize;
        for m in metas.into_iter().take(self.max_entries) {
            let id = Self::escape_text(&m.id);
            let mut desc = m.description.clone();
            if m.name != m.id && !m.name.trim().is_empty() {
                if desc.trim().is_empty() {
                    desc = m.name.clone();
                } else {
                    desc = format!("{}: {}", m.name.trim(), desc.trim());
                }
            }
            let desc = Self::escape_text(&desc);

            out.push_str("<skill>\n");
            out.push_str(&format!("<name>{}</name>\n", id));
            if !desc.trim().is_empty() {
                out.push_str(&format!("<description>{}</description>\n", desc));
            }
            out.push_str("</skill>\n");
            shown += 1;

            if out.len() >= self.max_chars {
                break;
            }
        }

        out.push_str("</available_skills>\n");

        if shown < total {
            out.push_str(&format!(
                "Note: available_skills truncated (total={}, shown={}).\n",
                total, shown
            ));
        }

        out.push_str("<skills_usage>\n");
        out.push_str("If a listed skill is relevant, call tool \"skill\" with {\"skill\": \"<id or name>\"} before answering.\n");
        out.push_str("Skill resources are not auto-loaded: use \"load_skill_resource\" with {\"skill\": \"<id>\", \"path\": \"references/<file>|assets/<file>\"}.\n");
        out.push_str("To run skill scripts: use \"skill_script\" with {\"skill\": \"<id>\", \"script\": \"scripts/<file>\", \"args\": [..]}.\n");
        out.push_str("</skills_usage>");

        if out.len() > self.max_chars {
            out.truncate(self.max_chars);
        }

        out.trim_end().to_string()
    }
}

struct SkillDiscoveryHook {
    plugin: SkillDiscoveryPlugin,
}

#[async_trait]
impl PhaseHook for SkillDiscoveryHook {
    async fn run(&self, ctx: &PhaseContext) -> Result<StateCommand, StateError> {
        let active: HashSet<String> = ctx
            .state::<SkillState>()
            .map(|s| s.active.iter().cloned().collect())
            .unwrap_or_default();

        let rendered = self.plugin.render_catalog(&active);
        if rendered.is_empty() {
            return Ok(StateCommand::new());
        }

        let mut cmd = StateCommand::new();
        cmd.schedule_action::<crate::AddContextMessage>(ContextMessage::system(
            "skill_catalog",
            rendered,
        ))?;
        Ok(cmd)
    }
}

impl Plugin for SkillDiscoveryPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: SKILLS_DISCOVERY_PLUGIN_ID,
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_key::<SkillState>(StateKeyOptions {
            persistent: true,
            retain_on_uninstall: false,
            scope: awaken_contract::state::KeyScope::Run,
        })?;

        registrar.register_phase_hook(
            SKILLS_DISCOVERY_PLUGIN_ID,
            Phase::BeforeInference,
            SkillDiscoveryHook {
                plugin: self.clone(),
            },
        )?;

        Ok(())
    }
}

/// Injects activated skill instructions as hidden suffix prompt segments.
#[derive(Clone)]
pub struct ActiveSkillInstructionsPlugin {
    registry: Arc<dyn SkillRegistry>,
}

impl std::fmt::Debug for ActiveSkillInstructionsPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActiveSkillInstructionsPlugin")
            .finish_non_exhaustive()
    }
}

impl ActiveSkillInstructionsPlugin {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }

    async fn render_active_instructions(&self, active_ids: Vec<String>) -> String {
        let mut rendered = Vec::new();
        let mut ids = active_ids;
        ids.sort();
        ids.dedup();

        for skill_id in ids {
            let Some(skill) = self.registry.get(&skill_id) else {
                continue;
            };

            let raw = match skill.read_instructions().await {
                Ok(raw) => raw,
                Err(err) => {
                    tracing::warn!(skill_id = %skill_id, error = %err, "failed to read active skill instructions");
                    continue;
                }
            };
            let doc = match parse_skill_md(&raw) {
                Ok(doc) => doc,
                Err(err) => {
                    tracing::warn!(skill_id = %skill_id, error = %err, "failed to parse active SKILL.md");
                    continue;
                }
            };
            let body = doc.body.trim();
            if body.is_empty() {
                continue;
            }

            rendered.push(format!(
                "<skill_instruction skill=\"{skill_id}\">\n{body}\n</skill_instruction>"
            ));
        }

        if rendered.is_empty() {
            String::new()
        } else {
            format!(
                "<active_skill_instructions>\n{}\n</active_skill_instructions>",
                rendered.join("\n")
            )
        }
    }
}

struct ActiveSkillInstructionsHook {
    plugin: ActiveSkillInstructionsPlugin,
}

#[async_trait]
impl PhaseHook for ActiveSkillInstructionsHook {
    async fn run(&self, ctx: &PhaseContext) -> Result<StateCommand, StateError> {
        let active: Vec<String> = ctx
            .state::<SkillState>()
            .map(|s| s.active.iter().cloned().collect())
            .unwrap_or_default();
        if active.is_empty() {
            return Ok(StateCommand::new());
        }

        let rendered = self.plugin.render_active_instructions(active).await;
        if rendered.is_empty() {
            return Ok(StateCommand::new());
        }

        let mut cmd = StateCommand::new();
        cmd.schedule_action::<crate::AddContextMessage>(ContextMessage::suffix_system(
            "active_skill_instructions",
            rendered,
        ))?;
        Ok(cmd)
    }
}

impl Plugin for ActiveSkillInstructionsPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: SKILLS_ACTIVE_INSTRUCTIONS_PLUGIN_ID,
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        // SkillState registration is handled by SkillDiscoveryPlugin.
        // We only register the phase hook here.
        registrar.register_phase_hook(
            SKILLS_ACTIVE_INSTRUCTIONS_PLUGIN_ID,
            Phase::BeforeInference,
            ActiveSkillInstructionsHook {
                plugin: self.clone(),
            },
        )?;

        Ok(())
    }
}

/// High-level facade for wiring skills into an agent.
#[derive(Clone)]
pub struct SkillSubsystem {
    registry: Arc<dyn SkillRegistry>,
}

impl std::fmt::Debug for SkillSubsystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillSubsystem").finish_non_exhaustive()
    }
}

/// Errors returned when wiring the skills subsystem into an agent.
#[derive(Debug, thiserror::Error)]
pub enum SkillSubsystemError {
    #[error("tool id already registered: {0}")]
    ToolIdConflict(String),
}

impl SkillSubsystem {
    pub fn new(registry: Arc<dyn SkillRegistry>) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &Arc<dyn SkillRegistry> {
        &self.registry
    }

    /// Build the discovery plugin (injects skills catalog before inference).
    pub fn discovery_plugin(&self) -> SkillDiscoveryPlugin {
        SkillDiscoveryPlugin::new(self.registry.clone())
    }

    /// Build the active instructions plugin (injects active skill instructions).
    pub fn active_instructions_plugin(&self) -> ActiveSkillInstructionsPlugin {
        ActiveSkillInstructionsPlugin::new(self.registry.clone())
    }

    /// Construct the skills tools map.
    pub fn tools(
        &self,
    ) -> std::collections::HashMap<String, Arc<dyn awaken_contract::contract::tool::Tool>> {
        let mut out: std::collections::HashMap<
            String,
            Arc<dyn awaken_contract::contract::tool::Tool>,
        > = std::collections::HashMap::new();
        let _ = self.extend_tools(&mut out);
        out
    }

    /// Add skills tools to an existing tool map.
    pub fn extend_tools(
        &self,
        tools: &mut std::collections::HashMap<
            String,
            Arc<dyn awaken_contract::contract::tool::Tool>,
        >,
    ) -> Result<(), SkillSubsystemError> {
        use crate::tools;
        use awaken_contract::contract::tool::Tool;

        let registry = self.registry.clone();
        let tool_defs: Vec<Arc<dyn Tool>> = vec![
            Arc::new(tools::SkillActivateTool::new(registry.clone())),
            Arc::new(tools::LoadSkillResourceTool::new(registry.clone())),
            Arc::new(tools::SkillScriptTool::new(registry)),
        ];

        for t in tool_defs {
            let id = t.descriptor().id.clone();
            if tools.contains_key(&id) {
                return Err(SkillSubsystemError::ToolIdConflict(id));
            }
            tools.insert(id, t);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{FsSkill, InMemorySkillRegistry};
    use crate::skill::Skill;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_registry(skills: Vec<Arc<dyn Skill>>) -> Arc<dyn SkillRegistry> {
        Arc::new(InMemorySkillRegistry::from_skills(skills))
    }

    fn make_skills() -> (TempDir, Vec<Arc<dyn Skill>>) {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("a-skill")).unwrap();
        fs::create_dir_all(root.join("b-skill")).unwrap();
        let mut fa = fs::File::create(root.join("a-skill").join("SKILL.md")).unwrap();
        fa.write_all(b"---\nname: a-skill\ndescription: Desc & \"<tag>\"\n---\nBody\n")
            .unwrap();
        fs::write(
            root.join("b-skill").join("SKILL.md"),
            "---\nname: b-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        let skills = FsSkill::into_arc_skills(result.skills);
        (td, skills)
    }

    #[test]
    fn render_catalog_contains_available_skills_and_usage() {
        let (_td, skills) = make_skills();
        let p = SkillDiscoveryPlugin::new(make_registry(skills)).with_limits(10, 8 * 1024);
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("<skills_usage>"));
        assert!(s.contains("&amp;"));
        assert!(s.contains("&lt;"));
        assert!(s.contains("&gt;"));
    }

    #[test]
    fn render_catalog_marks_skills() {
        let (_td, skills) = make_skills();
        let p = SkillDiscoveryPlugin::new(make_registry(skills));
        let active: HashSet<String> = ["a-skill".to_string()].into();
        let s = p.render_catalog(&active);
        assert!(s.contains("<name>a-skill</name>"));
    }

    #[test]
    fn render_catalog_returns_empty_for_no_skills() {
        let p = SkillDiscoveryPlugin::new(make_registry(vec![]));
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.is_empty());
    }

    #[test]
    fn render_catalog_empty_for_all_invalid_skills() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("BadSkill")).unwrap();
        fs::write(
            root.join("BadSkill").join("SKILL.md"),
            "---\nname: badskill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        assert!(result.skills.is_empty());

        let skills = FsSkill::into_arc_skills(result.skills);
        let p = SkillDiscoveryPlugin::new(make_registry(skills));
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.is_empty());
    }

    #[test]
    fn render_catalog_only_valid_skills_and_never_warnings() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("good-skill")).unwrap();
        fs::create_dir_all(root.join("BadSkill")).unwrap();
        fs::write(
            root.join("good-skill").join("SKILL.md"),
            "---\nname: good-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();
        fs::write(
            root.join("BadSkill").join("SKILL.md"),
            "---\nname: badskill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        let skills = FsSkill::into_arc_skills(result.skills);
        let p = SkillDiscoveryPlugin::new(make_registry(skills));
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.contains("<name>good-skill</name>"));
        assert!(!s.contains("BadSkill"));
    }

    #[test]
    fn render_catalog_truncates_by_entry_limit() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        for i in 0..5 {
            let name = format!("s{i}");
            fs::create_dir_all(root.join(&name)).unwrap();
            fs::write(
                root.join(&name).join("SKILL.md"),
                format!("---\nname: {name}\ndescription: ok\n---\nBody\n"),
            )
            .unwrap();
        }
        let result = FsSkill::discover(root).unwrap();
        let skills = FsSkill::into_arc_skills(result.skills);
        let p = SkillDiscoveryPlugin::new(make_registry(skills)).with_limits(2, 8 * 1024);
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("truncated"));
        assert_eq!(s.matches("<skill>").count(), 2);
    }

    #[test]
    fn render_catalog_truncates_by_char_limit() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s")).unwrap();
        fs::write(
            root.join("s").join("SKILL.md"),
            "---\nname: s\ndescription: A very long description\n---\nBody",
        )
        .unwrap();
        let result = FsSkill::discover(root).unwrap();
        let skills = FsSkill::into_arc_skills(result.skills);
        let p = SkillDiscoveryPlugin::new(make_registry(skills)).with_limits(10, 256);
        let active = HashSet::new();
        let s = p.render_catalog(&active);
        assert!(s.len() <= 256);
    }

    #[test]
    fn subsystem_tools_returns_expected_ids() {
        let (_td, skills) = make_skills();
        let sys = SkillSubsystem::new(make_registry(skills));
        let tools = sys.tools();
        assert!(tools.contains_key(crate::SKILL_ACTIVATE_TOOL_ID));
        assert!(tools.contains_key(crate::SKILL_LOAD_RESOURCE_TOOL_ID));
        assert!(tools.contains_key(crate::SKILL_SCRIPT_TOOL_ID));
        assert_eq!(tools.len(), 3);
    }

    #[test]
    fn subsystem_extend_tools_detects_conflict() {
        let (_td, skills) = make_skills();
        let sys = SkillSubsystem::new(make_registry(skills));
        let mut tools = sys.tools();
        let err = sys.extend_tools(&mut tools).unwrap_err();
        assert!(err.to_string().contains("tool id already registered"));
    }

    #[tokio::test]
    async fn active_instructions_renders_for_activated_skill() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("docx")).unwrap();
        fs::write(
            root.join("docx").join("SKILL.md"),
            "---\nname: docx\ndescription: ok\n---\n# DOCX\nUse docx-js.\n",
        )
        .unwrap();

        let result = FsSkill::discover(root).unwrap();
        let skills = FsSkill::into_arc_skills(result.skills);
        let plugin = ActiveSkillInstructionsPlugin::new(make_registry(skills));
        let rendered = plugin
            .render_active_instructions(vec!["docx".to_string()])
            .await;
        assert!(rendered.contains("<active_skill_instructions>"));
        assert!(rendered.contains("Use docx-js."));
    }

    #[tokio::test]
    async fn active_instructions_empty_for_no_active_skills() {
        let (_td, skills) = make_skills();
        let plugin = ActiveSkillInstructionsPlugin::new(make_registry(skills));
        let rendered = plugin.render_active_instructions(vec![]).await;
        assert!(rendered.is_empty());
    }
}
