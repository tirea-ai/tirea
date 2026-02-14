use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::skills::registry::SkillRegistry;
use crate::skills::state::{SkillState, SKILLS_STATE_PATH};
use crate::tool_filter::{
    is_runtime_allowed, RUNTIME_ALLOWED_SKILLS_KEY, RUNTIME_EXCLUDED_SKILLS_KEY,
};
use async_trait::async_trait;
use carve_state::Context;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Injects a skills catalog into the LLM context so the model can discover and activate skills.
///
/// This is intentionally non-persistent: the catalog is rebuilt from `SkillRegistry` per step.
#[derive(Debug, Clone)]
pub struct SkillDiscoveryPlugin {
    registry: Arc<dyn SkillRegistry>,
    max_entries: usize,
    max_chars: usize,
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
        // Minimal XML-ish escaping for text nodes.
        // (We intentionally avoid emitting attributes to keep the prompt closer to agentskills examples.)
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn render_catalog(
        &self,
        _active: &HashSet<String>,
        runtime: Option<&carve_state::Runtime>,
    ) -> String {
        let mut metas = self.registry.list();
        metas.retain(|m| {
            is_runtime_allowed(
                runtime,
                &m.id,
                RUNTIME_ALLOWED_SKILLS_KEY,
                RUNTIME_EXCLUDED_SKILLS_KEY,
            )
        });
        if metas.is_empty() {
            return String::new();
        }

        // Keep ordering stable.
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
            // Tool-based integration: omit <location> to keep tokens low.
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
impl AgentPlugin for SkillDiscoveryPlugin {
    fn id(&self) -> &str {
        "skills_discovery"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        if phase != Phase::BeforeInference {
            return;
        }

        // Read active skills from session state (if present) to annotate the catalog.
        let skill_state = ctx.state::<SkillState>(SKILLS_STATE_PATH);
        let active: HashSet<String> = skill_state
            .active()
            .ok()
            .unwrap_or_default()
            .into_iter()
            .collect();

        let rendered = self.render_catalog(&active, Some(&step.thread.runtime));
        if rendered.is_empty() {
            return;
        }

        // Treat the catalog as system-level guidance so the model can select skills.
        step.system(rendered);
    }

    fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::FsSkillRegistry;
    use crate::thread::Thread;
    use crate::traits::tool::ToolDescriptor;
    use carve_state::Context;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_registry() -> (TempDir, Arc<dyn SkillRegistry>) {
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

        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        (td, reg)
    }

    #[tokio::test]
    async fn injects_catalog_with_usage() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let (_td, reg) = make_registry();
        let p = SkillDiscoveryPlugin::new(reg).with_limits(10, 8 * 1024);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        assert_eq!(step.system_context.len(), 1);
        let s = &step.system_context[0];
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("<skills_usage>"));
        // Escaping is applied.
        assert!(s.contains("&amp;"));
        assert!(s.contains("&lt;"));
        assert!(s.contains("&gt;"));
    }

    #[tokio::test]
    async fn marks_active_skills() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let (_td, reg) = make_registry();
        let p = SkillDiscoveryPlugin::new(reg);
        let thread = Thread::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["a"],
                    "instructions": {"a": "Do X"},
                    "references": {},
                    "scripts": {}
                }
            }),
        );
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        let s = &step.system_context[0];
        // Does not include active annotations; active skills are handled by runtime plugin injection.
        assert!(s.contains("<name>a-skill</name>"));
    }

    #[tokio::test]
    async fn does_not_inject_when_registry_empty() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(&root).unwrap();
        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        let p = SkillDiscoveryPlugin::new(reg);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        assert!(step.system_context.is_empty());
    }

    #[tokio::test]
    async fn does_not_inject_when_all_skills_invalid() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("BadSkill")).unwrap();
        fs::write(
            root.join("BadSkill").join("SKILL.md"),
            "---\nname: badskill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        assert!(reg.list().is_empty());

        let p = SkillDiscoveryPlugin::new(reg);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        assert!(step.system_context.is_empty());
    }

    #[tokio::test]
    async fn injects_only_valid_skills_and_never_warnings() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
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

        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        let p = SkillDiscoveryPlugin::new(reg);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;

        assert_eq!(step.system_context.len(), 1);
        let s = &step.system_context[0];
        assert!(s.contains("<name>good-skill</name>"));
        assert!(!s.contains("BadSkill"));
        assert!(!s.contains("skills_warnings"));
        assert!(!s.contains("Skipped skill"));
    }

    #[tokio::test]
    async fn truncates_by_entry_limit_and_emits_note() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
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
        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        let p = SkillDiscoveryPlugin::new(reg).with_limits(2, 8 * 1024);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        let s = &step.system_context[0];
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("truncated"));
        // Only 2 entries should be present.
        assert_eq!(s.matches("<skill>").count(), 2);
    }

    #[tokio::test]
    async fn truncates_by_char_limit() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("s")).unwrap();
        fs::write(
            root.join("s").join("SKILL.md"),
            "---\nname: s\ndescription: A very long description\n---\nBody",
        )
        .unwrap();
        let reg: Arc<dyn SkillRegistry> = Arc::new(FsSkillRegistry::discover_root(root).unwrap());
        let p = SkillDiscoveryPlugin::new(reg).with_limits(10, 256);
        let thread = Thread::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        let s = &step.system_context[0];
        assert!(s.len() <= 256);
    }

    #[tokio::test]
    async fn filters_catalog_by_runtime_skill_policy() {
        let doc = json!({});
        let ctx = Context::new(&doc, "test", "test");
        let (_td, reg) = make_registry();
        let p = SkillDiscoveryPlugin::new(reg);
        let mut thread = Thread::with_initial_state("s", json!({}));
        thread
            .runtime
            .set(RUNTIME_ALLOWED_SKILLS_KEY, vec!["a-skill"])
            .unwrap();
        let mut step = StepContext::new(&thread, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step, &ctx).await;
        assert_eq!(step.system_context.len(), 1);
        let s = &step.system_context[0];
        assert!(s.contains("<name>a-skill</name>"));
        assert!(!s.contains("<name>b-skill</name>"));
    }

    // Registry warnings are logged during discovery; they are intentionally not injected into prompts.
}
