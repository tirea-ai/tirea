use carve_agent::ag_ui::{AGUIContext, AGUIEvent};
use carve_agent::ui_stream::{AiSdkAdapter, UIStreamEvent};
use carve_agent::AgentEvent;

pub struct AiSdkEncoder {
    adapter: AiSdkAdapter,
    finished: bool,
}

impl AiSdkEncoder {
    pub fn new(run_id: String) -> Self {
        Self {
            adapter: AiSdkAdapter::new(run_id),
            finished: false,
        }
    }

    pub fn prologue(&self) -> [UIStreamEvent; 2] {
        [self.adapter.message_start(), self.adapter.text_start()]
    }

    pub fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<UIStreamEvent> {
        if self.finished {
            return Vec::new();
        }

        if matches!(ev, AgentEvent::RunFinish { .. }) {
            self.finished = true;
            return vec![self.adapter.text_end(), self.adapter.finish()];
        }

        ev.to_ui_events(self.adapter.text_id())
    }
}

pub struct AgUiEncoder {
    ctx: AGUIContext,
    emitted_run_finished: bool,
    stopped: bool,
}

impl AgUiEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
            emitted_run_finished: false,
            stopped: false,
        }
    }

    pub fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<AGUIEvent> {
        if self.stopped {
            return Vec::new();
        }

        match ev {
            AgentEvent::Error { .. } => {
                self.stopped = true;
            }
            AgentEvent::RunFinish { .. } => {
                self.emitted_run_finished = true;
            }
            // Skip Pending events: their interaction-to-tool-call conversion
            // is redundant in AG-UI — the LLM's own TOOL_CALL events already
            // inform the client about frontend tool calls.
            AgentEvent::Pending { .. } => {
                return Vec::new();
            }
            _ => {}
        }

        ev.to_ag_ui_events(&mut self.ctx)
    }

    /// Emit a fallback RUN_FINISHED if the inner stream ended without one.
    /// This covers pending interactions (frontend tools, permissions) where
    /// the inner loop exits via PendingInteraction error without RunFinish.
    pub fn fallback_finished(&self, thread_id: &str, run_id: &str) -> Option<AGUIEvent> {
        if self.stopped || self.emitted_run_finished {
            return None;
        }
        Some(AGUIEvent::run_finished(thread_id, run_id, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agent::state_types::Interaction;

    #[test]
    fn test_ai_sdk_encoder_prologue_and_finish() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());
        let pro = enc.prologue();
        assert!(matches!(pro[0], UIStreamEvent::MessageStart { .. }));
        assert!(matches!(pro[1], UIStreamEvent::TextStart { .. }));

        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
        });
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::Finish));
    }

    #[test]
    fn test_agui_encoder_skips_pending_events() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());
        let pending = AgentEvent::Pending {
            interaction: Interaction::new("i1", "x"),
        };
        // Pending events should produce no AG-UI events (redundant with LLM tool calls)
        let out = enc.on_agent_event(&pending);
        assert!(out.is_empty());

        // RunFinish should still work normally
        let finish = AgentEvent::RunFinish {
            thread_id: "th".to_string(),
            run_id: "r".to_string(),
            result: None,
        };
        let out = enc.on_agent_event(&finish);
        assert!(!out.is_empty());
        assert!(enc.fallback_finished("th", "r").is_none());
    }

    #[test]
    fn test_agui_encoder_fallback_finished_when_no_run_finish() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // No RunFinish emitted — fallback should emit one
        let fallback = enc.fallback_finished("th", "r");
        assert!(fallback.is_some());
    }
}
