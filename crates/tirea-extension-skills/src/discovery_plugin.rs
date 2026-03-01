use crate::tool_filter::{is_scope_allowed, SCOPE_ALLOWED_SKILLS_KEY, SCOPE_EXCLUDED_SKILLS_KEY};
use crate::{SkillMeta, SkillRegistry, SkillState, SKILLS_DISCOVERY_PLUGIN_ID};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use tirea_contract::runtime::behavior::{AgentBehavior, ReadOnlyContext};
use tirea_contract::runtime::action::Action;
use tirea_contract::runtime::inference::InferenceContext;
use tirea_contract::runtime::phase::step::StepContext;
use tirea_contract::runtime::phase::Phase;

/// Injects a skills catalog into the LLM context so the model can discover and activate skills.
///
/// This is intentionally non-persistent: the catalog is rebuilt from the registry snapshot per step.
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

    fn render_catalog(
        &self,
        _active: &HashSet<String>,
        runtime: Option<&tirea_contract::RunConfig>,
    ) -> String {
        let mut metas: Vec<SkillMeta> = self
            .registry
            .snapshot()
            .values()
            .map(|s| s.meta().clone())
            .filter(|m| {
                is_scope_allowed(
                    runtime,
                    &m.id,
                    SCOPE_ALLOWED_SKILLS_KEY,
                    SCOPE_EXCLUDED_SKILLS_KEY,
                )
            })
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

#[async_trait]
impl AgentBehavior for SkillDiscoveryPlugin {
    fn id(&self) -> &str {
        SKILLS_DISCOVERY_PLUGIN_ID
    }

    async fn before_inference(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let active: HashSet<String> = ctx
            .snapshot_of::<SkillState>()
            .ok()
            .map(|s| s.active.into_iter().collect())
            .unwrap_or_default();

        let rendered = self.render_catalog(&active, Some(ctx.run_config()));
        if rendered.is_empty() {
            return vec![];
        }

        vec![Box::new(InjectSkillCatalog(rendered))]
    }
}

/// Inject the skill catalog into the system prompt context.
pub struct InjectSkillCatalog(pub String);

impl Action for InjectSkillCatalog {
    fn label(&self) -> &'static str {
        "add_system_context"
    }

    fn validate(&self, phase: Phase) -> Result<(), String> {
        if phase == Phase::BeforeInference {
            Ok(())
        } else {
            Err(format!(
                "InjectSkillCatalog is only allowed in BeforeInference, got {phase}"
            ))
        }
    }

    fn apply(self: Box<Self>, step: &mut StepContext<'_>) {
        step.extensions
            .get_or_default::<InferenceContext>()
            .system_context
            .push(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FsSkill, InMemorySkillRegistry, Skill};
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;
    use tirea_contract::runtime::phase::Phase;
    use tirea_contract::RunConfig;
    use tirea_state::DocCell;

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

    fn count_system_context_actions(actions: &[Box<dyn Action>]) -> usize {
        actions
            .iter()
            .filter(|a| a.label() == "add_system_context")
            .count()
    }

    /// Apply AddSystemContext actions to collect their string values for assertion.
    fn apply_and_extract_system_contexts(actions: Vec<Box<dyn Action>>) -> Vec<String> {
        use tirea_contract::runtime::inference::InferenceContext;
        use tirea_contract::testing::TestFixture;

        let fix = TestFixture::new();
        let mut step = fix.step(vec![]);
        for action in actions {
            if action.label() == "add_system_context" {
                action.apply(&mut step);
            }
        }
        step.extensions
            .get::<InferenceContext>()
            .map(|inf| inf.system_context.clone())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn injects_catalog_with_usage() {
        let (_td, skills) = make_skills();
        let p = SkillDiscoveryPlugin::new(make_registry(skills)).with_limits(10, 8 * 1024);
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        assert_eq!(count_system_context_actions(&actions), 1);
        let contexts = apply_and_extract_system_contexts(actions);
        assert_eq!(contexts.len(), 1);
        let s = &contexts[0];
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("<skills_usage>"));
        assert!(s.contains("&amp;"));
        assert!(s.contains("&lt;"));
        assert!(s.contains("&gt;"));
    }

    #[tokio::test]
    async fn marks_active_skills() {
        let (_td, skills) = make_skills();
        let p = SkillDiscoveryPlugin::new(make_registry(skills));
        let config = RunConfig::new();
        let doc = DocCell::new(json!({
            "skills": {
                "active": ["a"],
                "instructions": {"a": "Do X"},
                "references": {},
                "scripts": {}
            }
        }));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        let contexts = apply_and_extract_system_contexts(actions);
        let s = &contexts[0];
        assert!(s.contains("<name>a-skill</name>"));
    }

    #[tokio::test]
    async fn does_not_inject_when_skills_empty() {
        let p = SkillDiscoveryPlugin::new(make_registry(vec![]));
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn does_not_inject_when_all_skills_invalid() {
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
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn injects_only_valid_skills_and_never_warnings() {
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
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        let contexts = apply_and_extract_system_contexts(actions);
        assert_eq!(contexts.len(), 1);
        let s = &contexts[0];
        assert!(s.contains("<name>good-skill</name>"));
        assert!(!s.contains("BadSkill"));
        assert!(!s.contains("skills_warnings"));
        assert!(!s.contains("Skipped skill"));
    }

    #[tokio::test]
    async fn truncates_by_entry_limit_and_emits_note() {
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
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        let contexts = apply_and_extract_system_contexts(actions);
        let s = &contexts[0];
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("truncated"));
        assert_eq!(s.matches("<skill>").count(), 2);
    }

    #[tokio::test]
    async fn truncates_by_char_limit() {
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
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        let contexts = apply_and_extract_system_contexts(actions);
        let s = &contexts[0];
        assert!(s.len() <= 256);
    }

    #[tokio::test]
    async fn filters_catalog_by_runtime_skill_policy() {
        let (_td, skills) = make_skills();
        let p = SkillDiscoveryPlugin::new(make_registry(skills));
        let mut config = RunConfig::new();
        config
            .set(SCOPE_ALLOWED_SKILLS_KEY, vec!["a-skill"])
            .unwrap();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&p, &ctx).await;
        let contexts = apply_and_extract_system_contexts(actions);
        assert_eq!(contexts.len(), 1);
        let s = &contexts[0];
        assert!(s.contains("<name>a-skill</name>"));
        assert!(!s.contains("<name>b-skill</name>"));
    }
}
