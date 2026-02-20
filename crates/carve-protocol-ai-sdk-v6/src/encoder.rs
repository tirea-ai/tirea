use super::UIStreamEvent;
use carve_agent_contract::{AgentEvent, StopReason, TerminationReason};
use tracing::warn;

/// Data event name for a full state snapshot payload.
pub const DATA_EVENT_STATE_SNAPSHOT: &str = "state-snapshot";
/// Data event name for RFC6902 state patch payload.
pub const DATA_EVENT_STATE_DELTA: &str = "state-delta";
/// Data event name for messages snapshot payload.
pub const DATA_EVENT_MESSAGES_SNAPSHOT: &str = "messages-snapshot";
/// Data event name for activity snapshot payload.
pub const DATA_EVENT_ACTIVITY_SNAPSHOT: &str = "activity-snapshot";
/// Data event name for activity patch payload.
pub const DATA_EVENT_ACTIVITY_DELTA: &str = "activity-delta";
/// Data event name for pending interaction payload.
pub const DATA_EVENT_INTERACTION: &str = "interaction";
/// Data event name for interaction-requested payload.
pub const DATA_EVENT_INTERACTION_REQUESTED: &str = "interaction-requested";
/// Data event name for interaction-resolved payload.
pub const DATA_EVENT_INTERACTION_RESOLVED: &str = "interaction-resolved";
/// Data event name for inference-complete payload (token usage).
pub const DATA_EVENT_INFERENCE_COMPLETE: &str = "inference-complete";

/// Stateful encoder for AI SDK v6 UI Message Stream protocol.
///
/// Tracks text block lifecycle (open/close) across tool calls, ensuring
/// `text-start` and `text-end` are always properly paired. This mirrors the
/// pattern used by AG-UI encoders for AG-UI.
///
/// # Text lifecycle rules
///
/// - `TextDelta` with text closed → prepend `text-start`, open text
/// - `ToolCallStart` with text open → prepend `text-end`, close text
/// - `RunFinish` with text open → prepend `text-end` before `finish`
/// - `Error` → terminal, no `text-end` needed
#[derive(Debug)]
pub struct AiSdkEncoder {
    message_id: String,
    run_id: String,
    text_open: bool,
    text_counter: u32,
    finished: bool,
    /// Whether an external message ID has been consumed from a StepStart event.
    message_id_set: bool,
}

impl AiSdkEncoder {
    /// Create a new encoder for the given run.
    pub fn new(run_id: String) -> Self {
        let message_id = format!("msg_{}", &run_id[..8.min(run_id.len())]);
        Self {
            message_id,
            run_id,
            text_open: false,
            text_counter: 0,
            finished: false,
            message_id_set: false,
        }
    }

    /// Current text block ID (e.g. `txt_0`, `txt_1`, ...).
    fn text_id(&self) -> String {
        format!("txt_{}", self.text_counter)
    }

    /// Emit `text-start` and mark text as open. Returns the new text ID.
    fn open_text(&mut self) -> UIStreamEvent {
        self.text_open = true;
        UIStreamEvent::text_start(self.text_id())
    }

    /// Emit `text-end` for the current text block and mark text as closed.
    /// Increments the counter so the next text block gets a fresh ID.
    fn close_text(&mut self) -> UIStreamEvent {
        let event = UIStreamEvent::text_end(self.text_id());
        self.text_open = false;
        self.text_counter += 1;
        event
    }

    /// Get the message ID.
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// Get the run ID.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Emit the stream prologue: `message-start`.
    ///
    /// Unlike the old adapter, this does NOT emit `text-start` here —
    /// text blocks are opened lazily when the first `TextDelta` arrives.
    pub fn prologue(&self) -> Vec<UIStreamEvent> {
        vec![UIStreamEvent::message_start(&self.message_id)]
    }

    /// Convert an `AgentEvent` to UI stream events with proper text lifecycle.
    pub fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<UIStreamEvent> {
        if self.finished {
            return Vec::new();
        }

        match ev {
            AgentEvent::TextDelta { delta } => {
                let mut events = Vec::new();
                if !self.text_open {
                    events.push(self.open_text());
                }
                events.push(UIStreamEvent::text_delta(self.text_id(), delta));
                events
            }

            AgentEvent::ToolCallStart { id, name } => {
                let mut events = Vec::new();
                if self.text_open {
                    events.push(self.close_text());
                }
                events.push(UIStreamEvent::tool_input_start(id, name));
                events
            }
            AgentEvent::ToolCallDelta { id, args_delta } => {
                vec![UIStreamEvent::tool_input_delta(id, args_delta)]
            }
            AgentEvent::ToolCallReady {
                id,
                name,
                arguments,
            } => {
                vec![UIStreamEvent::tool_input_available(
                    id,
                    name,
                    arguments.clone(),
                )]
            }
            AgentEvent::ToolCallDone { id, result, .. } => {
                vec![UIStreamEvent::tool_output_available(id, result.to_json())]
            }

            AgentEvent::RunFinish { termination, .. } => {
                self.finished = true;
                let mut events = Vec::new();
                if self.text_open {
                    events.push(self.close_text());
                }
                let finish_reason = Self::map_termination(termination);
                events.push(UIStreamEvent::finish_with_reason(finish_reason));
                events
            }

            AgentEvent::Error { message } => {
                self.finished = true;
                self.text_open = false;
                vec![UIStreamEvent::error(message)]
            }

            AgentEvent::StepStart { message_id } => {
                if !self.message_id_set {
                    self.message_id = message_id.clone();
                    self.message_id_set = true;
                }
                vec![UIStreamEvent::start_step()]
            }
            AgentEvent::StepEnd => {
                let mut events = Vec::new();
                if self.text_open {
                    events.push(self.close_text());
                }
                events.push(UIStreamEvent::finish_step());
                events
            }
            AgentEvent::RunStart { .. } => vec![],
            AgentEvent::InferenceComplete {
                model,
                usage,
                duration_ms,
            } => {
                let payload = serde_json::json!({
                    "model": model,
                    "usage": usage,
                    "duration_ms": duration_ms,
                });
                vec![UIStreamEvent::data(DATA_EVENT_INFERENCE_COMPLETE, payload)]
            }

            AgentEvent::StateSnapshot { snapshot } => {
                vec![UIStreamEvent::data(
                    DATA_EVENT_STATE_SNAPSHOT,
                    snapshot.clone(),
                )]
            }
            AgentEvent::StateDelta { delta } => {
                vec![UIStreamEvent::data(
                    DATA_EVENT_STATE_DELTA,
                    serde_json::Value::Array(delta.clone()),
                )]
            }
            AgentEvent::MessagesSnapshot { messages } => {
                vec![UIStreamEvent::data(
                    DATA_EVENT_MESSAGES_SNAPSHOT,
                    serde_json::Value::Array(messages.clone()),
                )]
            }
            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => {
                let payload = serde_json::json!({
                    "messageId": message_id,
                    "activityType": activity_type,
                    "content": content,
                    "replace": replace,
                });
                vec![UIStreamEvent::data(DATA_EVENT_ACTIVITY_SNAPSHOT, payload)]
            }
            AgentEvent::ActivityDelta {
                message_id,
                activity_type,
                patch,
            } => {
                let payload = serde_json::json!({
                    "messageId": message_id,
                    "activityType": activity_type,
                    "patch": patch,
                });
                vec![UIStreamEvent::data(DATA_EVENT_ACTIVITY_DELTA, payload)]
            }
            AgentEvent::Pending { interaction } => {
                let payload = match serde_json::to_value(interaction) {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!(error = %err, interaction_id = %interaction.id, "failed to serialize pending interaction for AI SDK");
                        serde_json::json!({
                            "id": interaction.id,
                            "error": "failed to serialize interaction",
                        })
                    }
                };
                vec![UIStreamEvent::data(DATA_EVENT_INTERACTION, payload)]
            }
            AgentEvent::InteractionRequested { interaction } => {
                let payload = match serde_json::to_value(interaction) {
                    Ok(payload) => payload,
                    Err(err) => {
                        warn!(error = %err, interaction_id = %interaction.id, "failed to serialize interaction-requested event for AI SDK");
                        serde_json::json!({
                            "id": interaction.id,
                            "error": "failed to serialize interaction",
                        })
                    }
                };
                vec![UIStreamEvent::data(
                    DATA_EVENT_INTERACTION_REQUESTED,
                    payload,
                )]
            }
            AgentEvent::InteractionResolved {
                interaction_id,
                result,
            } => {
                vec![UIStreamEvent::data(
                    DATA_EVENT_INTERACTION_RESOLVED,
                    serde_json::json!({
                        "interactionId": interaction_id,
                        "result": result,
                    }),
                )]
            }
        }
    }

    fn map_termination(reason: &TerminationReason) -> &'static str {
        match reason {
            TerminationReason::NaturalEnd
            | TerminationReason::PluginRequested
            | TerminationReason::PendingInteraction => "stop",
            TerminationReason::Cancelled => "other",
            TerminationReason::Error => "error",
            TerminationReason::Stopped(stop_reason) => match stop_reason {
                StopReason::MaxRoundsReached
                | StopReason::TimeoutReached
                | StopReason::TokenBudgetExceeded => "length",
                StopReason::ToolCalled(_) => "tool-calls",
                StopReason::ContentMatched(_) => "stop",
                StopReason::ConsecutiveErrorsExceeded | StopReason::LoopDetected => "error",
                StopReason::Custom(_) => "other",
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::chat::Usage;

    #[test]
    fn inference_complete_emits_data_event() {
        let mut enc = AiSdkEncoder::new("run_12345678".into());
        let ev = AgentEvent::InferenceComplete {
            model: "gpt-4o".into(),
            usage: Some(Usage {
                prompt_tokens: Some(100),
                completion_tokens: Some(50),
                ..Default::default()
            }),
            duration_ms: 1234,
        };
        let events = enc.on_agent_event(&ev);
        assert_eq!(events.len(), 1);
        match &events[0] {
            UIStreamEvent::Data { data_type, data } => {
                assert_eq!(data_type, &format!("data-{DATA_EVENT_INFERENCE_COMPLETE}"));
                assert_eq!(data["model"], "gpt-4o");
                assert_eq!(data["duration_ms"], 1234);
                assert!(data["usage"].is_object(), "usage: {:?}", data["usage"]);
            }
            other => panic!("expected Data event, got: {:?}", other),
        }
    }

    #[test]
    fn inference_complete_without_usage() {
        let mut enc = AiSdkEncoder::new("run_12345678".into());
        let ev = AgentEvent::InferenceComplete {
            model: "gpt-4o-mini".into(),
            usage: None,
            duration_ms: 500,
        };
        let events = enc.on_agent_event(&ev);
        assert_eq!(events.len(), 1);
        match &events[0] {
            UIStreamEvent::Data { data_type, data } => {
                assert_eq!(data_type, &format!("data-{DATA_EVENT_INFERENCE_COMPLETE}"));
                assert_eq!(data["model"], "gpt-4o-mini");
                assert!(data["usage"].is_null());
            }
            other => panic!("expected Data event, got: {:?}", other),
        }
    }
}
