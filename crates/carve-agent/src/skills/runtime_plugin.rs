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
#[derive(Debug, Default)]
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

    fn initial_data(&self) -> Option<(&'static str, Value)> {
        // This initializes plugin data only; the persisted skill state lives in session state.
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
}
