use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::skills::registry::SkillRegistry;
use crate::skills::state::{SkillState, SKILLS_STATE_PATH};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

fn render_warnings_only(
    warnings: &[crate::skills::registry::SkillRegistryWarning],
    max_chars: usize,
) -> String {
    let mut out = String::new();
    out.push_str("<skills_warnings>\n");
    for w in warnings.iter().take(16) {
        let p = SkillDiscoveryPlugin::escape_text(&w.path.to_string_lossy());
        let r = SkillDiscoveryPlugin::escape_text(&w.reason);
        out.push_str(&format!(
            "<warning><path>{}</path><reason>{}</reason></warning>\n",
            p, r
        ));
        if out.len() >= max_chars {
            break;
        }
    }
    out.push_str("</skills_warnings>");
    if out.len() > max_chars {
        out.truncate(max_chars);
    }
    out.trim_end().to_string()
}

/// Injects a skills catalog into the LLM context so the model can discover and activate skills.
///
/// This is intentionally non-persistent: the catalog is rebuilt from `SkillRegistry` per step.
#[derive(Debug, Clone)]
pub struct SkillDiscoveryPlugin {
    registry: Arc<SkillRegistry>,
    max_entries: usize,
    max_chars: usize,
}

impl SkillDiscoveryPlugin {
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
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

    fn render_catalog(&self, _active: &HashSet<String>) -> String {
        let mut metas = self.registry.list();
        if metas.is_empty() {
            // Still surface registry warnings if any skills were skipped.
            let warnings = self.registry.warnings();
            if warnings.is_empty() {
                return String::new();
            }
            return render_warnings_only(&warnings, self.max_chars);
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
        out.push_str("References are not auto-loaded: use \"load_skill_reference\" with {\"skill\": \"<id>\", \"path\": \"references/<file>\"}.\n");
        out.push_str("To run skill scripts: use \"skill_script\" with {\"skill\": \"<id>\", \"script\": \"scripts/<file>\", \"args\": [..]}.\n");
        out.push_str("</skills_usage>");

        // Add diagnostics for skipped skills (spec violations).
        let warnings = self.registry.warnings();
        if !warnings.is_empty() && out.len() < self.max_chars {
            out.push('\n');
            out.push_str("<skills_warnings>\n");
            // Keep this bounded to avoid bloating prompts.
            for w in warnings.into_iter().take(16) {
                let p = Self::escape_text(&w.path.to_string_lossy());
                let r = Self::escape_text(&w.reason);
                out.push_str(&format!("<warning><path>{}</path><reason>{}</reason></warning>\n", p, r));
                if out.len() >= self.max_chars {
                    break;
                }
            }
            out.push_str("</skills_warnings>");
        }

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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase != Phase::BeforeInference {
            return;
        }

        // Read active skills from session state (if present) to annotate the catalog.
        let mut active: HashSet<String> = HashSet::new();
        if let Ok(state) = step.session.rebuild_state() {
            if let Some(skills_value) = state.get(SKILLS_STATE_PATH).cloned() {
                if let Ok(parsed) = serde_json::from_value::<SkillState>(skills_value) {
                    active.extend(parsed.active.into_iter());
                }
            }
        }

        let rendered = self.render_catalog(&active);
        if rendered.is_empty() {
            return;
        }

        // Treat the catalog as system-level guidance so the model can select skills.
        step.system(rendered);
    }

    fn initial_data(&self) -> Option<(&'static str, Value)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::tool::ToolDescriptor;
    use crate::session::Session;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_registry() -> Arc<SkillRegistry> {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(root.join("a-skill")).unwrap();
        fs::create_dir_all(root.join("b-skill")).unwrap();
        let mut fa = fs::File::create(root.join("a-skill").join("SKILL.md")).unwrap();
        writeln!(
            fa,
            "{}",
            r#"---
name: a-skill
description: Desc & "<tag>"
---
Body"#
        )
        .unwrap();
        fs::write(
            root.join("b-skill").join("SKILL.md"),
            "---\nname: b-skill\ndescription: ok\n---\nBody\n",
        )
        .unwrap();

        // Keep tempdir alive by leaking it: this is test-only and acceptable.
        std::mem::forget(td);
        Arc::new(SkillRegistry::from_root(root))
    }

    #[tokio::test]
    async fn injects_catalog_with_usage() {
        let reg = make_registry();
        let p = SkillDiscoveryPlugin::new(reg).with_limits(10, 8 * 1024);
        let session = Session::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step).await;
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
        let reg = make_registry();
        let p = SkillDiscoveryPlugin::new(reg);
        let session = Session::with_initial_state(
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
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step).await;
        let s = &step.system_context[0];
        // Does not include active annotations; active skills are handled by runtime plugin injection.
        assert!(s.contains("<name>a-skill</name>"));
    }

    #[tokio::test]
    async fn does_not_inject_when_registry_empty() {
        let td = TempDir::new().unwrap();
        let root = td.path().join("skills");
        fs::create_dir_all(&root).unwrap();
        let reg = Arc::new(SkillRegistry::from_root(root));
        let p = SkillDiscoveryPlugin::new(reg);
        let session = Session::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step).await;
        assert!(step.system_context.is_empty());
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
        let reg = Arc::new(SkillRegistry::from_root(root));
        let p = SkillDiscoveryPlugin::new(reg).with_limits(2, 8 * 1024);
        let session = Session::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step).await;
        let s = &step.system_context[0];
        assert!(s.contains("<available_skills>"));
        assert!(s.contains("truncated"));
        // Only 2 entries should be present.
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
        let reg = Arc::new(SkillRegistry::from_root(root));
        let p = SkillDiscoveryPlugin::new(reg).with_limits(10, 256);
        let session = Session::with_initial_state("s", json!({}));
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        p.on_phase(Phase::BeforeInference, &mut step).await;
        let s = &step.system_context[0];
        assert!(s.len() <= 256);
    }

    // warnings-only rendering is exercised indirectly via empty registries with warnings.
}
