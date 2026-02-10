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
    has_pending: bool,
    emitted_run_finished: bool,
    stopped: bool,
}

impl AgUiEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
            has_pending: false,
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
            AgentEvent::Pending { .. } => {
                self.has_pending = true;
            }
            AgentEvent::RunFinish { .. } if self.has_pending => {
                // When pending, suppress RunFinish — run stays open for client interaction.
                return Vec::new();
            }
            AgentEvent::RunFinish { .. } => {
                self.emitted_run_finished = true;
            }
            _ => {}
        }

        ev.to_ag_ui_events(&mut self.ctx)
    }

    pub fn fallback_finished(&self, thread_id: &str, run_id: &str) -> Option<AGUIEvent> {
        if self.stopped || self.has_pending || self.emitted_run_finished {
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
    fn test_agui_encoder_suppresses_run_finish_when_pending() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());
        let pending = AgentEvent::Pending {
            interaction: Interaction::new("i1", "x"),
        };
        let out = enc.on_agent_event(&pending);
        assert!(!out.is_empty());

        let finish = AgentEvent::RunFinish {
            thread_id: "th".to_string(),
            run_id: "r".to_string(),
            result: None,
        };
        let out = enc.on_agent_event(&finish);
        assert!(out.is_empty());
        assert!(enc.fallback_finished("th", "r").is_none());
    }
}

