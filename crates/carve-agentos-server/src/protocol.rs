use carve_agent::ag_ui::{AGUIContext, AGUIEvent};
use carve_agent::stop::StopReason;
use carve_agent::ui_stream::{AiSdkAdapter, UIStreamEvent};
use carve_agent::{agent_event_to_agui, agent_event_to_ui, AgentEvent};

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

        if let AgentEvent::RunFinish { stop_reason, .. } = ev {
            self.finished = true;
            let finish_reason = match stop_reason {
                Some(StopReason::NaturalEnd)
                | Some(StopReason::ContentMatched(_))
                | Some(StopReason::PluginRequested) => "stop",
                Some(StopReason::MaxRoundsReached)
                | Some(StopReason::TimeoutReached)
                | Some(StopReason::TokenBudgetExceeded) => "length",
                Some(StopReason::ToolCalled(_)) => "tool-calls",
                Some(StopReason::Cancelled) => "other",
                Some(StopReason::ConsecutiveErrorsExceeded)
                | Some(StopReason::LoopDetected) => "error",
                Some(StopReason::Custom(_)) => "other",
                None => "stop",
            };
            return vec![
                self.adapter.text_end(),
                UIStreamEvent::finish_with_reason(finish_reason),
            ];
        }

        agent_event_to_ui(ev, self.adapter.text_id())
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

        agent_event_to_agui(ev, &mut self.ctx)
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
            stop_reason: None,
        });
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::Finish { .. }));
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
            stop_reason: None,
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

    #[test]
    fn test_agui_encoder_stopped_after_error() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        let error = AgentEvent::Error {
            message: "LLM failed".to_string(),
        };
        let out = enc.on_agent_event(&error);
        assert!(!out.is_empty()); // Should emit the error event

        // After error, all subsequent events should be ignored
        let text = AgentEvent::TextDelta {
            delta: "ignored".to_string(),
        };
        let out2 = enc.on_agent_event(&text);
        assert!(out2.is_empty());

        // Fallback should also be None since we stopped
        assert!(enc.fallback_finished("th", "r").is_none());
    }

    #[test]
    fn test_agui_encoder_text_then_finish() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        let text = AgentEvent::TextDelta {
            delta: "hello".to_string(),
        };
        let out = enc.on_agent_event(&text);
        assert!(!out.is_empty());

        let finish = AgentEvent::RunFinish {
            thread_id: "th".to_string(),
            run_id: "r".to_string(),
            result: None,
            stop_reason: None,
        };
        let out2 = enc.on_agent_event(&finish);
        // Should include TEXT_MESSAGE_END and RUN_FINISHED
        assert!(out2.len() >= 2);

        // No fallback needed
        assert!(enc.fallback_finished("th", "r").is_none());
    }

    #[test]
    fn test_agui_encoder_multiple_pending_all_skipped() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        for i in 0..5 {
            let pending = AgentEvent::Pending {
                interaction: Interaction::new(&format!("i{}", i), "action"),
            };
            let out = enc.on_agent_event(&pending);
            assert!(out.is_empty());
        }

        // Fallback should still work since no RunFinish was emitted
        assert!(enc.fallback_finished("th", "r").is_some());
    }

    #[test]
    fn test_ai_sdk_encoder_ignores_after_finish() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        // Finish the stream
        let _ = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });

        // Subsequent events should be empty
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "late".to_string(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn test_agui_encoder_tool_call_flow() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        let start = AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "search".to_string(),
        };
        let out = enc.on_agent_event(&start);
        assert!(!out.is_empty());

        let delta = AgentEvent::ToolCallDelta {
            id: "tc-1".to_string(),
            args_delta: r#"{"q":"rust"}"#.to_string(),
        };
        let out = enc.on_agent_event(&delta);
        assert!(!out.is_empty());

        let ready = AgentEvent::ToolCallReady {
            id: "tc-1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let out = enc.on_agent_event(&ready);
        assert!(!out.is_empty());
    }

    // ====================================================================
    // AiSdkEncoder: StopReason → finish_reason mapping tests
    // ====================================================================

    /// Helper to get the finish_reason string from encoder output.
    fn ai_sdk_finish_reason(stop_reason: Option<StopReason>) -> String {
        let mut enc = AiSdkEncoder::new("r".to_string());
        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "r".to_string(),
            result: None,
            stop_reason,
        });
        // Last event should be Finish; extract finish_reason via serialization
        let finish = out.last().unwrap();
        let json = serde_json::to_value(finish).unwrap();
        json["finishReason"]
            .as_str()
            .unwrap_or("(none)")
            .to_string()
    }

    #[test]
    fn test_finish_reason_natural_end() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::NaturalEnd)), "stop");
    }

    #[test]
    fn test_finish_reason_plugin_requested() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::PluginRequested)), "stop");
    }

    #[test]
    fn test_finish_reason_content_matched() {
        assert_eq!(
            ai_sdk_finish_reason(Some(StopReason::ContentMatched("DONE".into()))),
            "stop"
        );
    }

    #[test]
    fn test_finish_reason_max_rounds() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::MaxRoundsReached)), "length");
    }

    #[test]
    fn test_finish_reason_timeout() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::TimeoutReached)), "length");
    }

    #[test]
    fn test_finish_reason_token_budget() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::TokenBudgetExceeded)), "length");
    }

    #[test]
    fn test_finish_reason_tool_called() {
        assert_eq!(
            ai_sdk_finish_reason(Some(StopReason::ToolCalled("finish".into()))),
            "tool-calls"
        );
    }

    #[test]
    fn test_finish_reason_cancelled() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::Cancelled)), "other");
    }

    #[test]
    fn test_finish_reason_custom() {
        assert_eq!(
            ai_sdk_finish_reason(Some(StopReason::Custom("my_reason".into()))),
            "other"
        );
    }

    #[test]
    fn test_finish_reason_consecutive_errors() {
        assert_eq!(
            ai_sdk_finish_reason(Some(StopReason::ConsecutiveErrorsExceeded)),
            "error"
        );
    }

    #[test]
    fn test_finish_reason_loop_detected() {
        assert_eq!(ai_sdk_finish_reason(Some(StopReason::LoopDetected)), "error");
    }

    #[test]
    fn test_finish_reason_none_defaults_to_stop() {
        assert_eq!(ai_sdk_finish_reason(None), "stop");
    }
}
