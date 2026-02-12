use crate::phase::Phase;
use crate::phase::StepContext;
use crate::plugin::AgentPlugin;
use crate::skills::state::{SkillState, SKILLS_STATE_PATH};
use async_trait::async_trait;
use serde_json::Value;

/// Injects activated skills (instructions + loaded materials) into the LLM context.
///
/// This plugin reads skill state from the session document and adds it as system context
/// during `BeforeInference`.
#[derive(Debug, Default, Clone)]
pub struct SkillRuntimePlugin;

impl SkillRuntimePlugin {
    pub fn new() -> Self {
        Self
    }

    fn render_context(state: &SkillState) -> String {
        if state.active.is_empty() {
            return String::new();
        }

        let mut out = String::new();
        for skill_id in &state.active {
            if let Some(instr) = state.instructions.get(skill_id) {
                out.push_str(&format!("<skill id=\"{skill_id}\">\n"));
                out.push_str(instr.trim_end());
                out.push_str("\n</skill>\n");
            }

            // references
            let prefix = format!("{skill_id}:");
            let mut refs: Vec<_> = state
                .references
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(_, r)| r)
                .collect();
            refs.sort_by(|a, b| a.path.cmp(&b.path));
            for r in refs {
                out.push_str(&format!(
                    "<skill_reference skill=\"{}\" path=\"{}\" truncated=\"{}\">\n",
                    r.skill, r.path, r.truncated
                ));
                out.push_str(r.content.trim_end());
                out.push_str("\n</skill_reference>\n");
            }

            // script results (summary only)
            let mut scripts: Vec<_> = state
                .scripts
                .iter()
                .filter(|(k, _)| k.starts_with(&prefix))
                .map(|(_, s)| s)
                .collect();
            scripts.sort_by(|a, b| a.script.cmp(&b.script));
            for s in scripts {
                out.push_str(&format!(
                    "<skill_script_result skill=\"{}\" script=\"{}\" exit_code=\"{}\" stdout_truncated=\"{}\" stderr_truncated=\"{}\">\n",
                    s.skill, s.script, s.exit_code, s.truncated_stdout, s.truncated_stderr
                ));
                if !s.stdout.trim().is_empty() {
                    out.push_str("<stdout>\n");
                    out.push_str(s.stdout.trim_end());
                    out.push_str("\n</stdout>\n");
                }
                if !s.stderr.trim().is_empty() {
                    out.push_str("<stderr>\n");
                    out.push_str(s.stderr.trim_end());
                    out.push_str("\n</stderr>\n");
                }
                out.push_str("</skill_script_result>\n");
            }
        }

        out.trim_end().to_string()
    }
}

#[async_trait]
impl AgentPlugin for SkillRuntimePlugin {
    fn id(&self) -> &str {
        "skills_runtime"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase != Phase::BeforeInference {
            return;
        }

        let state = match step.session.rebuild_state() {
            Ok(s) => s,
            Err(_) => return,
        };

        let Some(skills_value) = state.get(SKILLS_STATE_PATH).cloned() else {
            return;
        };

        let parsed: SkillState = match serde_json::from_value::<SkillState>(skills_value) {
            Ok(s) => s,
            Err(_) => return,
        };

        let rendered = Self::render_context(&parsed);
        if rendered.is_empty() {
            return;
        }

        // Treat skills as system-level instructions by default.
        step.system(rendered);
    }

    fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
        // This initializes runtime scratchpad only; persisted skill state lives in session state.
        // Keeping this empty avoids duplicating two sources of truth.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::traits::tool::ToolDescriptor;
    use serde_json::json;

    #[tokio::test]
    async fn plugin_injects_active_skill_instructions() {
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
        let p = SkillRuntimePlugin::new();
        p.on_phase(Phase::BeforeInference, &mut step).await;
        assert_eq!(step.system_context.len(), 1);
        assert!(step.system_context[0].contains("Do X"));
    }

    #[tokio::test]
    async fn plugin_sorts_references_and_scripts_by_path() {
        let session = Session::with_initial_state(
            "s",
            json!({
                "skills": {
                    "active": ["a"],
                    "instructions": {"a": "Do X"},
                    "references": {
                        "a:references/b.md": {"skill":"a","path":"references/b.md","sha256":"x","truncated":false,"content":"B","bytes":1},
                        "a:references/a.md": {"skill":"a","path":"references/a.md","sha256":"y","truncated":false,"content":"A","bytes":1}
                    },
                    "scripts": {
                        "a:scripts/z.sh": {"skill":"a","script":"scripts/z.sh","sha256":"1","truncated_stdout":false,"truncated_stderr":false,"exit_code":0,"stdout":"Z","stderr":""},
                        "a:scripts/a.sh": {"skill":"a","script":"scripts/a.sh","sha256":"2","truncated_stdout":false,"truncated_stderr":false,"exit_code":0,"stdout":"A","stderr":""}
                    }
                }
            }),
        );
        let mut step = StepContext::new(&session, vec![ToolDescriptor::new("t", "t", "t")]);
        let p = SkillRuntimePlugin::new();
        p.on_phase(Phase::BeforeInference, &mut step).await;
        let s = &step.system_context[0];

        let idx_ref_a = s.find("path=\"references/a.md\"").unwrap();
        let idx_ref_b = s.find("path=\"references/b.md\"").unwrap();
        assert!(idx_ref_a < idx_ref_b);

        let idx_script_a = s.find("script=\"scripts/a.sh\"").unwrap();
        let idx_script_z = s.find("script=\"scripts/z.sh\"").unwrap();
        assert!(idx_script_a < idx_script_z);
    }
}
