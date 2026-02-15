use crate::protocol::{interaction_to_ag_ui_events, AGUIEvent};
use carve_agent_runtime_contract::AgentEvent;
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;
use uuid::Uuid;

// AG-UI Context
// ============================================================================

/// Context for AG-UI event conversion.
///
/// Maintains state needed for converting internal AgentEvents to AG-UI events.
#[derive(Debug, Clone)]
pub struct AGUIContext {
    /// Thread identifier (conversation context).
    pub thread_id: String,
    /// Current run identifier.
    pub run_id: String,
    /// Current message identifier.
    pub message_id: String,
    /// Step counter for generating step names.
    step_counter: u32,
    /// Whether text message stream has started.
    pub(super) text_started: bool,
    /// Whether text has ever been ended (used to detect restarts).
    text_ever_ended: bool,
    /// Current step name.
    current_step: Option<String>,
    /// Whether a terminal event (Error/Aborted) has been emitted.
    /// After this, all subsequent events are suppressed.
    stopped: bool,
    /// Last emitted state snapshot, used to compute RFC 6902 deltas.
    last_state: Option<Value>,
}

impl AGUIContext {
    /// Create a new AG-UI context.
    pub fn new(thread_id: String, run_id: String) -> Self {
        let run_id_prefix: String = run_id.chars().take(8).collect();
        let message_id = format!("msg_{run_id_prefix}");
        Self {
            thread_id,
            run_id,
            message_id,
            step_counter: 0,
            text_started: false,
            text_ever_ended: false,
            current_step: None,
            stopped: false,
            last_state: None,
        }
    }

    /// Generate the next step name.
    pub fn next_step_name(&mut self) -> String {
        self.step_counter += 1;
        let name = format!("step_{}", self.step_counter);
        self.current_step = Some(name.clone());
        name
    }

    /// Get the current step name.
    pub fn current_step_name(&self) -> String {
        self.current_step
            .clone()
            .unwrap_or_else(|| format!("step_{}", self.step_counter))
    }

    /// Mark text stream as started.
    ///
    /// If text was previously ended, a new `message_id` is generated so that
    /// each TEXT_MESSAGE_START / TEXT_MESSAGE_END cycle uses a unique ID.
    /// This prevents CopilotKit / AG-UI runtimes from confusing reopened
    /// message IDs with already-ended ones (which causes "text-end for
    /// missing text part" errors on the frontend).
    pub fn start_text(&mut self) -> bool {
        let was_started = self.text_started;
        self.text_started = true;
        if !was_started {
            // Generate a fresh message_id when restarting text after a prior end.
            if self.text_ever_ended {
                self.new_message_id();
            }
            true
        } else {
            false
        }
    }

    /// Mark text stream as ended and return whether it was active.
    pub fn end_text(&mut self) -> bool {
        let was_started = self.text_started;
        self.text_started = false;
        if was_started {
            self.text_ever_ended = true;
        }
        was_started
    }

    /// Whether a text stream is currently open.
    pub fn is_text_open(&self) -> bool {
        self.text_started
    }

    /// Reset text lifecycle state for a new step with a pre-generated message ID.
    ///
    /// This ensures that the streaming message ID matches the stored `Message.id`
    /// for the assistant message produced by this step.
    pub fn reset_for_step(&mut self, message_id: String) {
        self.message_id = message_id;
        self.text_started = false;
        self.text_ever_ended = false;
    }

    /// Generate a new message ID.
    pub fn new_message_id(&mut self) -> String {
        let run_id_prefix: String = self.run_id.chars().take(8).collect();
        self.message_id = format!("msg_{run_id_prefix}_{}", Uuid::new_v4().simple());
        self.message_id.clone()
    }

    /// Convert an AgentEvent to AG-UI protocol compatible events.
    ///
    /// Handles full stream lifecycle: text start/end pairs, step counters,
    /// terminal event suppression (after Error), and Pending event filtering.
    pub fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<AGUIEvent> {
        // After a terminal event (Error/Aborted), suppress everything.
        if self.stopped {
            return Vec::new();
        }

        // Lifecycle bookkeeping before conversion.
        match ev {
            AgentEvent::Error { .. } | AgentEvent::Aborted { .. } => {
                self.stopped = true;
            }
            AgentEvent::InteractionRequested { .. } | AgentEvent::InteractionResolved { .. } => {
                return vec![];
            }
            // Pending: close current text stream and emit interaction tool-call events.
            AgentEvent::Pending { interaction } => {
                let mut events = Vec::new();
                if self.end_text() {
                    events.push(AGUIEvent::text_message_end(&self.message_id));
                }
                events.extend(interaction_to_ag_ui_events(interaction));
                return events;
            }
            _ => {}
        }

        match ev {
            AgentEvent::RunStart {
                thread_id,
                run_id,
                parent_run_id,
            } => {
                vec![AGUIEvent::run_started(
                    thread_id,
                    run_id,
                    parent_run_id.clone(),
                )]
            }
            AgentEvent::RunFinish {
                thread_id,
                run_id,
                result,
                ..
            } => {
                let mut events = vec![];
                if self.end_text() {
                    events.push(AGUIEvent::text_message_end(&self.message_id));
                }
                events.push(AGUIEvent::run_finished(thread_id, run_id, result.clone()));
                events
            }

            AgentEvent::TextDelta { delta } => {
                let mut events = vec![];
                if self.start_text() {
                    events.push(AGUIEvent::text_message_start(&self.message_id));
                }
                events.push(AGUIEvent::text_message_content(&self.message_id, delta));
                events
            }

            AgentEvent::ToolCallStart { id, name } => {
                let mut events = vec![];
                if self.end_text() {
                    events.push(AGUIEvent::text_message_end(&self.message_id));
                }
                events.push(AGUIEvent::tool_call_start(
                    id,
                    name,
                    Some(self.message_id.clone()),
                ));
                events
            }
            AgentEvent::ToolCallDelta { id, args_delta } => {
                vec![AGUIEvent::tool_call_args(id, args_delta)]
            }
            AgentEvent::ToolCallReady { id, .. } => {
                vec![AGUIEvent::tool_call_end(id)]
            }
            AgentEvent::ToolCallDone {
                id,
                result,
                message_id,
                ..
            } => {
                let content = match serde_json::to_string(&result.to_json()) {
                    Ok(content) => content,
                    Err(err) => {
                        warn!(error = %err, tool_call_id = %id, "failed to serialize tool result for AG-UI");
                        r#"{"error":"failed to serialize tool result"}"#.to_string()
                    }
                };
                let msg_id = if message_id.is_empty() {
                    format!("result_{id}")
                } else {
                    message_id.clone()
                };
                vec![AGUIEvent::tool_call_result(msg_id, id, content)]
            }

            AgentEvent::StepStart { message_id } => {
                if !message_id.is_empty() {
                    self.reset_for_step(message_id.clone());
                }
                vec![AGUIEvent::step_started(self.next_step_name())]
            }
            AgentEvent::StepEnd => {
                vec![AGUIEvent::step_finished(self.current_step_name())]
            }

            AgentEvent::StateSnapshot { snapshot } => {
                let mut events = Vec::new();
                // Emit RFC 6902 delta if we have a previous state to diff against.
                if let Some(ref old) = self.last_state {
                    let patch = json_patch::diff(old, snapshot);
                    if !patch.0.is_empty() {
                        let delta = patch
                            .0
                            .iter()
                            .map(|op| serde_json::to_value(op).expect("RFC 6902 op serializes"))
                            .collect();
                        events.push(AGUIEvent::state_delta(delta));
                    }
                }
                self.last_state = Some(snapshot.clone());
                events.push(AGUIEvent::state_snapshot(snapshot.clone()));
                events
            }
            AgentEvent::StateDelta { delta } => {
                vec![AGUIEvent::state_delta(delta.clone())]
            }
            AgentEvent::MessagesSnapshot { messages } => {
                vec![AGUIEvent::messages_snapshot(messages.clone())]
            }

            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => {
                vec![AGUIEvent::activity_snapshot(
                    message_id.clone(),
                    activity_type.clone(),
                    value_to_map(content),
                    *replace,
                )]
            }
            AgentEvent::ActivityDelta {
                message_id,
                activity_type,
                patch,
            } => {
                vec![AGUIEvent::activity_delta(
                    message_id.clone(),
                    activity_type.clone(),
                    patch.clone(),
                )]
            }

            // Pending is handled above (early return).
            AgentEvent::Pending { .. } => unreachable!(),

            AgentEvent::Aborted { reason } => {
                vec![AGUIEvent::run_error(reason, Some("ABORTED".to_string()))]
            }
            AgentEvent::Error { message } => {
                vec![AGUIEvent::run_error(message, None)]
            }
            AgentEvent::InferenceComplete { .. } => vec![],
            AgentEvent::InteractionRequested { .. } | AgentEvent::InteractionResolved { .. } => {
                vec![]
            }
        }
    }
}

pub(super) fn value_to_map(value: &Value) -> HashMap<String, Value> {
    match value.as_object() {
        Some(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        None => {
            let mut map = HashMap::new();
            map.insert("value".to_string(), value.clone());
            map
        }
    }
}

// ============================================================================
