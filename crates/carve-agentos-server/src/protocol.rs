use carve_agent::ag_ui::{AGUIContext, AGUIEvent};
use carve_agent::{agent_event_to_agui, AgentEvent};

// Re-export the canonical AiSdkEncoder from carve-agent.
pub use carve_agent::AiSdkEncoder;

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
        // After an error, only allow RunFinish through (to emit a terminal event).
        if self.stopped && !matches!(ev, AgentEvent::RunFinish { .. }) {
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
            // However, we must still close any open text stream.
            AgentEvent::Pending { .. } => {
                let mut events = Vec::new();
                if self.ctx.end_text() {
                    events.push(AGUIEvent::text_message_end(&self.ctx.message_id));
                }
                return events;
            }
            _ => {}
        }

        agent_event_to_agui(ev, &mut self.ctx)
    }

    /// Emit fallback events if the inner stream ended without a RUN_FINISHED.
    /// Closes any open text stream before emitting RUN_FINISHED.
    /// This covers pending interactions (frontend tools, permissions) where
    /// the inner loop exits via PendingInteraction error without RunFinish.
    pub fn fallback_finished(&mut self, thread_id: &str, run_id: &str) -> Vec<AGUIEvent> {
        if self.stopped || self.emitted_run_finished {
            return Vec::new();
        }
        let mut events = Vec::new();
        if self.ctx.end_text() {
            events.push(AGUIEvent::text_message_end(&self.ctx.message_id));
        }
        events.push(AGUIEvent::run_finished(thread_id, run_id, None));
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agent::state_types::Interaction;
    use carve_agent::stop::StopReason;
    use carve_agent::UIStreamEvent;

    #[test]
    fn test_ai_sdk_encoder_prologue_only_message_start() {
        let enc = AiSdkEncoder::new("run_1".to_string());
        let pro = enc.prologue();
        // Prologue emits only message-start; text-start is lazy.
        assert_eq!(pro.len(), 1);
        assert!(matches!(pro[0], UIStreamEvent::MessageStart { .. }));
    }

    #[test]
    fn test_ai_sdk_encoder_text_then_finish() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hello".to_string(),
        });
        // First text delta should open text: [text-start, text-delta]
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UIStreamEvent::TextStart { .. }));
        assert!(matches!(out[1], UIStreamEvent::TextDelta { .. }));

        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });
        // Should close text then finish: [text-end, finish]
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::Finish { .. }));
    }

    #[test]
    fn test_ai_sdk_encoder_finish_without_text_no_text_end() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });
        // No text was opened, so no text-end — just finish.
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::Finish { .. }));
    }

    #[test]
    fn test_ai_sdk_encoder_text_tool_text_flow() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        // First text opens txt_0
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });
        assert_eq!(out.len(), 2); // text-start + text-delta

        // Tool call closes txt_0
        let out = enc.on_agent_event(&AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "search".to_string(),
        });
        assert_eq!(out.len(), 2); // text-end + tool-input-start
        assert!(matches!(out[0], UIStreamEvent::TextEnd { .. }));
        assert!(matches!(out[1], UIStreamEvent::ToolInputStart { .. }));

        // Text after tool opens txt_1
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "result".to_string(),
        });
        assert_eq!(out.len(), 2); // text-start + text-delta

        // Finish closes txt_1
        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "t".to_string(),
            run_id: "run_1".to_string(),
            result: None,
            stop_reason: None,
        });
        assert_eq!(out.len(), 2); // text-end + finish
    }

    #[test]
    fn test_ai_sdk_encoder_tool_without_prior_text_no_text_end() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        let out = enc.on_agent_event(&AgentEvent::ToolCallStart {
            id: "tc-1".to_string(),
            name: "search".to_string(),
        });
        // No text was open, so no text-end — just tool-input-start.
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::ToolInputStart { .. }));
    }

    #[test]
    fn test_ai_sdk_encoder_error_no_text_end() {
        let mut enc = AiSdkEncoder::new("run_1".to_string());

        // Open text
        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        // Error is terminal — no text-end
        let out = enc.on_agent_event(&AgentEvent::Error {
            message: "boom".to_string(),
        });
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], UIStreamEvent::Error { .. }));

        // After error, nothing emitted
        let out = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "late".to_string(),
        });
        assert!(out.is_empty());
    }

    #[test]
    fn test_agui_encoder_skips_pending_without_text() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());
        let pending = AgentEvent::Pending {
            interaction: Interaction::new("i1", "x"),
        };
        // Pending without open text produces nothing.
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
        assert!(enc.fallback_finished("th", "r").is_empty());
    }

    #[test]
    fn test_agui_encoder_pending_closes_open_text() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // Open text
        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        // Pending should close text even though it skips the interaction events
        let out = enc.on_agent_event(&AgentEvent::Pending {
            interaction: Interaction::new("i1", "x"),
        });
        assert_eq!(out.len(), 1);
        assert!(out
            .iter()
            .any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));
    }

    #[test]
    fn test_agui_encoder_text_pending_fallback_closes_text() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // Open text
        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "thinking...".to_string(),
        });

        // Pending closes text
        let out = enc.on_agent_event(&AgentEvent::Pending {
            interaction: Interaction::new("i1", "confirm"),
        });
        assert!(out
            .iter()
            .any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));

        // Fallback should only emit RUN_FINISHED (text already closed)
        let fallback = enc.fallback_finished("th", "r");
        assert_eq!(fallback.len(), 1);
        assert!(matches!(fallback[0], AGUIEvent::RunFinished { .. }));
    }

    #[test]
    fn test_agui_encoder_fallback_finished_when_no_run_finish() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // No RunFinish emitted — fallback should emit one
        let fallback = enc.fallback_finished("th", "r");
        assert_eq!(fallback.len(), 1);
        assert!(matches!(fallback[0], AGUIEvent::RunFinished { .. }));
    }

    #[test]
    fn test_agui_encoder_fallback_closes_open_text() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // Open text without closing
        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "interrupted".to_string(),
        });

        // Fallback should close text before emitting RUN_FINISHED
        let fallback = enc.fallback_finished("th", "r");
        assert_eq!(fallback.len(), 2);
        assert!(matches!(fallback[0], AGUIEvent::TextMessageEnd { .. }));
        assert!(matches!(fallback[1], AGUIEvent::RunFinished { .. }));
    }

    #[test]
    fn test_agui_encoder_stopped_after_error() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        let error = AgentEvent::Error {
            message: "LLM failed".to_string(),
        };
        let out = enc.on_agent_event(&error);
        assert!(!out.is_empty()); // Should emit the error event

        // After error, non-terminal events should be ignored
        let out2 = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "ignored".to_string(),
        });
        assert!(out2.is_empty());

        // But RunFinish should still get through (terminal event)
        let out3 = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "th".to_string(),
            run_id: "r".to_string(),
            result: None,
            stop_reason: None,
        });
        assert!(!out3.is_empty());
        assert!(out3
            .iter()
            .any(|e| matches!(e, AGUIEvent::RunFinished { .. })));

        // Fallback should be empty since RunFinish was emitted
        assert!(enc.fallback_finished("th", "r").is_empty());
    }

    #[test]
    fn test_agui_encoder_text_then_finish() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hello".to_string(),
        });

        let out = enc.on_agent_event(&AgentEvent::RunFinish {
            thread_id: "th".to_string(),
            run_id: "r".to_string(),
            result: None,
            stop_reason: None,
        });
        // Should include TEXT_MESSAGE_END and RUN_FINISHED
        assert!(out.len() >= 2);
        assert!(out
            .iter()
            .any(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })));
        assert!(out
            .iter()
            .any(|e| matches!(e, AGUIEvent::RunFinished { .. })));

        // No fallback needed
        assert!(enc.fallback_finished("th", "r").is_empty());
    }

    #[test]
    fn test_agui_encoder_multiple_pending_all_close_text() {
        let mut enc = AgUiEncoder::new("th".to_string(), "r".to_string());

        // First pending without text — nothing
        let out = enc.on_agent_event(&AgentEvent::Pending {
            interaction: Interaction::new("i0", "action"),
        });
        assert!(out.is_empty());

        // Open text
        let _ = enc.on_agent_event(&AgentEvent::TextDelta {
            delta: "hi".to_string(),
        });

        // Second pending with open text — closes it
        let out = enc.on_agent_event(&AgentEvent::Pending {
            interaction: Interaction::new("i1", "action"),
        });
        assert_eq!(out.len(), 1);

        // Third pending without text — nothing again
        let out = enc.on_agent_event(&AgentEvent::Pending {
            interaction: Interaction::new("i2", "action"),
        });
        assert!(out.is_empty());

        // Fallback should work since no RunFinish was emitted
        let fallback = enc.fallback_finished("th", "r");
        assert_eq!(fallback.len(), 1); // Just RUN_FINISHED, text already closed
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
