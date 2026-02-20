use crate::event::interaction::Interaction;
use crate::tool::contract::ToolResult;
use crate::event::termination::TerminationReason;
use tirea_state::TrackedPatch;
use genai::chat::Usage;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// Agent loop events for streaming execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Run started.
    RunStart {
        thread_id: String,
        run_id: String,
        parent_run_id: Option<String>,
    },
    /// Run finished.
    RunFinish {
        thread_id: String,
        run_id: String,
        result: Option<Value>,
        /// Why this run terminated.
        termination: TerminationReason,
    },

    /// LLM text delta.
    TextDelta { delta: String },

    /// Tool call started.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta.
    ToolCallDelta { id: String, args_delta: String },
    /// Tool call input is complete.
    ToolCallReady {
        id: String,
        name: String,
        arguments: Value,
    },
    /// Tool call completed.
    ToolCallDone {
        id: String,
        result: ToolResult,
        patch: Option<TrackedPatch>,
        /// Pre-generated ID for the stored tool result message.
        message_id: String,
    },

    /// Step started.
    StepStart {
        /// Pre-generated ID for the assistant message this step will produce.
        message_id: String,
    },
    /// Step completed.
    StepEnd,

    /// LLM inference completed with token usage data.
    InferenceComplete {
        /// Model used for this inference.
        model: String,
        /// Token usage.
        usage: Option<Usage>,
        /// Duration of the LLM call in milliseconds.
        duration_ms: u64,
    },

    /// State snapshot.
    StateSnapshot { snapshot: Value },
    /// State delta.
    StateDelta { delta: Vec<Value> },
    /// Messages snapshot.
    MessagesSnapshot { messages: Vec<Value> },

    /// Activity snapshot.
    ActivitySnapshot {
        message_id: String,
        activity_type: String,
        content: Value,
        replace: Option<bool>,
    },
    /// Activity delta.
    ActivityDelta {
        message_id: String,
        activity_type: String,
        patch: Vec<Value>,
    },

    /// Interaction request created.
    InteractionRequested { interaction: Interaction },
    /// Interaction resolution received.
    InteractionResolved {
        interaction_id: String,
        result: Value,
    },
    /// Pending interaction request.
    Pending { interaction: Interaction },

    /// Error occurred.
    Error { message: String },
}

impl AgentEvent {
    /// Extract the response text from a `RunFinish` result value.
    pub fn extract_response(result: &Option<Value>) -> String {
        result
            .as_ref()
            .and_then(|v| v.get("response"))
            .and_then(|r| r.as_str())
            .unwrap_or_default()
            .to_string()
    }

    fn event_type(&self) -> AgentEventType {
        match self {
            Self::RunStart { .. } => AgentEventType::RunStart,
            Self::RunFinish { .. } => AgentEventType::RunFinish,
            Self::TextDelta { .. } => AgentEventType::TextDelta,
            Self::ToolCallStart { .. } => AgentEventType::ToolCallStart,
            Self::ToolCallDelta { .. } => AgentEventType::ToolCallDelta,
            Self::ToolCallReady { .. } => AgentEventType::ToolCallReady,
            Self::ToolCallDone { .. } => AgentEventType::ToolCallDone,
            Self::StepStart { .. } => AgentEventType::StepStart,
            Self::StepEnd => AgentEventType::StepEnd,
            Self::InferenceComplete { .. } => AgentEventType::InferenceComplete,
            Self::StateSnapshot { .. } => AgentEventType::StateSnapshot,
            Self::StateDelta { .. } => AgentEventType::StateDelta,
            Self::MessagesSnapshot { .. } => AgentEventType::MessagesSnapshot,
            Self::ActivitySnapshot { .. } => AgentEventType::ActivitySnapshot,
            Self::ActivityDelta { .. } => AgentEventType::ActivityDelta,
            Self::InteractionRequested { .. } => AgentEventType::InteractionRequested,
            Self::InteractionResolved { .. } => AgentEventType::InteractionResolved,
            Self::Pending { .. } => AgentEventType::Pending,
            Self::Error { .. } => AgentEventType::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AgentEventType {
    RunStart,
    RunFinish,
    TextDelta,
    ToolCallStart,
    ToolCallDelta,
    ToolCallReady,
    ToolCallDone,
    StepStart,
    StepEnd,
    InferenceComplete,
    StateSnapshot,
    StateDelta,
    MessagesSnapshot,
    ActivitySnapshot,
    ActivityDelta,
    InteractionRequested,
    InteractionResolved,
    Pending,
    Error,
}

#[derive(Debug, Clone)]
struct RuntimeEnvelopeMeta {
    event_type: AgentEventType,
    run_id: String,
    thread_id: String,
    seq: u64,
    timestamp_ms: u64,
    step_id: Option<String>,
}

const RUNTIME_ENVELOPE_META_MAX_BUFFERED: usize = 4096;
static RUNTIME_ENVELOPE_META_BUFFER: OnceLock<Mutex<VecDeque<RuntimeEnvelopeMeta>>> =
    OnceLock::new();

thread_local! {
    static SERIALIZE_RUN_HINT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn runtime_envelope_meta_buffer() -> &'static Mutex<VecDeque<RuntimeEnvelopeMeta>> {
    RUNTIME_ENVELOPE_META_BUFFER.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Register runtime envelope metadata for one emitted event.
pub fn register_runtime_event_envelope_meta(
    event: &AgentEvent,
    run_id: &str,
    thread_id: &str,
    seq: u64,
    timestamp_ms: u64,
    step_id: Option<String>,
) {
    let entry = RuntimeEnvelopeMeta {
        event_type: event.event_type(),
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        seq,
        timestamp_ms,
        step_id,
    };
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");
    buffer.push_back(entry);
    if buffer.len() > RUNTIME_ENVELOPE_META_MAX_BUFFERED {
        let overflow = buffer.len() - RUNTIME_ENVELOPE_META_MAX_BUFFERED;
        buffer.drain(0..overflow);
    }
}

/// Clear all buffered runtime envelope metadata used during event serialization.
pub fn clear_runtime_event_envelope_meta() {
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");
    buffer.clear();
    SERIALIZE_RUN_HINT.with(|hint| {
        *hint.borrow_mut() = None;
    });
}

fn take_runtime_event_envelope_meta(
    event_type: AgentEventType,
    run_hint: Option<(&str, &str)>,
) -> Option<RuntimeEnvelopeMeta> {
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");

    if let Some((run_id, thread_id)) = run_hint {
        if let Some(pos) = buffer.iter().position(|entry| {
            entry.event_type == event_type && entry.run_id == run_id && entry.thread_id == thread_id
        }) {
            return buffer.remove(pos);
        }
    }

    if let Some(pos) = buffer
        .iter()
        .position(|entry| entry.event_type == event_type)
    {
        return buffer.remove(pos);
    }

    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EventEnvelope {
    #[serde(rename = "type")]
    event_type: AgentEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunStartData {
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunFinishData {
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    termination: TerminationReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TextDeltaData {
    delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallStartData {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallDeltaData {
    id: String,
    args_delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallReadyData {
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCallDoneData {
    id: String,
    result: ToolResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<TrackedPatch>,
    #[serde(default)]
    message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StepStartData {
    #[serde(default)]
    message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InferenceCompleteData {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateSnapshotData {
    snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StateDeltaData {
    delta: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessagesSnapshotData {
    messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivitySnapshotData {
    message_id: String,
    activity_type: String,
    content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    replace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivityDeltaData {
    message_id: String,
    activity_type: String,
    patch: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InteractionRequestedData {
    interaction: Interaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InteractionResolvedData {
    interaction_id: String,
    result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingData {
    interaction: Interaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ErrorData {
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmptyData {}

fn to_data_value<T: Serialize>(value: &T) -> serde_json::Result<Option<Value>> {
    let encoded = serde_json::to_value(value)?;
    if matches!(encoded, Value::Object(ref o) if o.is_empty()) {
        Ok(None)
    } else {
        Ok(Some(encoded))
    }
}

fn from_data_value<T, E>(value: Option<Value>) -> Result<T, E>
where
    T: DeserializeOwned,
    E: serde::de::Error,
{
    let data = value.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    serde_json::from_value(data).map_err(E::custom)
}

impl Serialize for AgentEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let event_type = self.event_type();
        let run_hint = match self {
            Self::RunStart {
                run_id, thread_id, ..
            }
            | Self::RunFinish {
                run_id, thread_id, ..
            } => Some((run_id.clone(), thread_id.clone())),
            _ => SERIALIZE_RUN_HINT.with(|hint| hint.borrow().clone()),
        };
        let runtime_meta = take_runtime_event_envelope_meta(
            event_type,
            run_hint
                .as_ref()
                .map(|(run_id, thread_id)| (run_id.as_str(), thread_id.as_str())),
        );
        let meta_run_id = || runtime_meta.as_ref().map(|m| m.run_id.clone());
        let meta_thread_id = || runtime_meta.as_ref().map(|m| m.thread_id.clone());
        let meta_seq = || runtime_meta.as_ref().map(|m| m.seq);
        let meta_timestamp_ms = || runtime_meta.as_ref().map(|m| m.timestamp_ms);
        let meta_step_id = || runtime_meta.as_ref().and_then(|m| m.step_id.clone());

        let envelope = match self {
            Self::RunStart {
                thread_id,
                run_id,
                parent_run_id,
            } => EventEnvelope {
                event_type: AgentEventType::RunStart,
                run_id: meta_run_id().or_else(|| Some(run_id.clone())),
                thread_id: meta_thread_id().or_else(|| Some(thread_id.clone())),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&RunStartData {
                    parent_run_id: parent_run_id.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::RunFinish {
                thread_id,
                run_id,
                result,
                termination,
            } => EventEnvelope {
                event_type: AgentEventType::RunFinish,
                run_id: meta_run_id().or_else(|| Some(run_id.clone())),
                thread_id: meta_thread_id().or_else(|| Some(thread_id.clone())),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&RunFinishData {
                    result: result.clone(),
                    termination: termination.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::TextDelta { delta } => EventEnvelope {
                event_type: AgentEventType::TextDelta,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&TextDeltaData {
                    delta: delta.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ToolCallStart { id, name } => EventEnvelope {
                event_type: AgentEventType::ToolCallStart,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ToolCallStartData {
                    id: id.clone(),
                    name: name.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ToolCallDelta { id, args_delta } => EventEnvelope {
                event_type: AgentEventType::ToolCallDelta,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ToolCallDeltaData {
                    id: id.clone(),
                    args_delta: args_delta.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ToolCallReady {
                id,
                name,
                arguments,
            } => EventEnvelope {
                event_type: AgentEventType::ToolCallReady,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ToolCallReadyData {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ToolCallDone {
                id,
                result,
                patch,
                message_id,
            } => EventEnvelope {
                event_type: AgentEventType::ToolCallDone,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ToolCallDoneData {
                    id: id.clone(),
                    result: result.clone(),
                    patch: patch.clone(),
                    message_id: message_id.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::StepStart { message_id } => EventEnvelope {
                event_type: AgentEventType::StepStart,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&StepStartData {
                    message_id: message_id.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::StepEnd => EventEnvelope {
                event_type: AgentEventType::StepEnd,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&EmptyData {}).map_err(serde::ser::Error::custom)?,
            },
            Self::InferenceComplete {
                model,
                usage,
                duration_ms,
            } => EventEnvelope {
                event_type: AgentEventType::InferenceComplete,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&InferenceCompleteData {
                    model: model.clone(),
                    usage: usage.clone(),
                    duration_ms: *duration_ms,
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::StateSnapshot { snapshot } => EventEnvelope {
                event_type: AgentEventType::StateSnapshot,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&StateSnapshotData {
                    snapshot: snapshot.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::StateDelta { delta } => EventEnvelope {
                event_type: AgentEventType::StateDelta,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&StateDeltaData {
                    delta: delta.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::MessagesSnapshot { messages } => EventEnvelope {
                event_type: AgentEventType::MessagesSnapshot,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&MessagesSnapshotData {
                    messages: messages.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                replace,
            } => EventEnvelope {
                event_type: AgentEventType::ActivitySnapshot,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ActivitySnapshotData {
                    message_id: message_id.clone(),
                    activity_type: activity_type.clone(),
                    content: content.clone(),
                    replace: *replace,
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ActivityDelta {
                message_id,
                activity_type,
                patch,
            } => EventEnvelope {
                event_type: AgentEventType::ActivityDelta,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ActivityDeltaData {
                    message_id: message_id.clone(),
                    activity_type: activity_type.clone(),
                    patch: patch.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::InteractionRequested { interaction } => EventEnvelope {
                event_type: AgentEventType::InteractionRequested,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&InteractionRequestedData {
                    interaction: interaction.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::InteractionResolved {
                interaction_id,
                result,
            } => EventEnvelope {
                event_type: AgentEventType::InteractionResolved,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&InteractionResolvedData {
                    interaction_id: interaction_id.clone(),
                    result: result.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::Pending { interaction } => EventEnvelope {
                event_type: AgentEventType::Pending,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&PendingData {
                    interaction: interaction.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::Error { message } => EventEnvelope {
                event_type: AgentEventType::Error,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ErrorData {
                    message: message.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
        };

        match self {
            Self::RunStart {
                run_id, thread_id, ..
            } => {
                let hint = runtime_meta
                    .as_ref()
                    .map(|meta| (meta.run_id.clone(), meta.thread_id.clone()))
                    .unwrap_or_else(|| (run_id.clone(), thread_id.clone()));
                SERIALIZE_RUN_HINT.with(|current| {
                    *current.borrow_mut() = Some(hint);
                });
            }
            Self::RunFinish {
                run_id, thread_id, ..
            } => {
                let finished = runtime_meta
                    .as_ref()
                    .map(|meta| (meta.run_id.clone(), meta.thread_id.clone()))
                    .unwrap_or_else(|| (run_id.clone(), thread_id.clone()));
                SERIALIZE_RUN_HINT.with(|current| {
                    let mut current = current.borrow_mut();
                    let should_clear = current
                        .as_ref()
                        .map(|active| active == &finished)
                        .unwrap_or(true);
                    if should_clear {
                        *current = None;
                    }
                });
            }
            _ => {}
        }

        envelope.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AgentEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let envelope = EventEnvelope::deserialize(deserializer)?;
        match envelope.event_type {
            AgentEventType::RunStart => {
                let data: RunStartData = from_data_value(envelope.data)?;
                let run_id = envelope
                    .run_id
                    .ok_or_else(|| serde::de::Error::custom("run_start missing run_id"))?;
                let thread_id = envelope
                    .thread_id
                    .ok_or_else(|| serde::de::Error::custom("run_start missing thread_id"))?;
                Ok(Self::RunStart {
                    thread_id,
                    run_id,
                    parent_run_id: data.parent_run_id,
                })
            }
            AgentEventType::RunFinish => {
                let data: RunFinishData = from_data_value(envelope.data)?;
                let run_id = envelope
                    .run_id
                    .ok_or_else(|| serde::de::Error::custom("run_finish missing run_id"))?;
                let thread_id = envelope
                    .thread_id
                    .ok_or_else(|| serde::de::Error::custom("run_finish missing thread_id"))?;
                Ok(Self::RunFinish {
                    thread_id,
                    run_id,
                    result: data.result,
                    termination: data.termination,
                })
            }
            AgentEventType::TextDelta => {
                let data: TextDeltaData = from_data_value(envelope.data)?;
                Ok(Self::TextDelta { delta: data.delta })
            }
            AgentEventType::ToolCallStart => {
                let data: ToolCallStartData = from_data_value(envelope.data)?;
                Ok(Self::ToolCallStart {
                    id: data.id,
                    name: data.name,
                })
            }
            AgentEventType::ToolCallDelta => {
                let data: ToolCallDeltaData = from_data_value(envelope.data)?;
                Ok(Self::ToolCallDelta {
                    id: data.id,
                    args_delta: data.args_delta,
                })
            }
            AgentEventType::ToolCallReady => {
                let data: ToolCallReadyData = from_data_value(envelope.data)?;
                Ok(Self::ToolCallReady {
                    id: data.id,
                    name: data.name,
                    arguments: data.arguments,
                })
            }
            AgentEventType::ToolCallDone => {
                let data: ToolCallDoneData = from_data_value(envelope.data)?;
                Ok(Self::ToolCallDone {
                    id: data.id,
                    result: data.result,
                    patch: data.patch,
                    message_id: data.message_id,
                })
            }
            AgentEventType::StepStart => {
                let data: StepStartData = from_data_value(envelope.data)?;
                Ok(Self::StepStart {
                    message_id: data.message_id,
                })
            }
            AgentEventType::StepEnd => Ok(Self::StepEnd),
            AgentEventType::InferenceComplete => {
                let data: InferenceCompleteData = from_data_value(envelope.data)?;
                Ok(Self::InferenceComplete {
                    model: data.model,
                    usage: data.usage,
                    duration_ms: data.duration_ms,
                })
            }
            AgentEventType::StateSnapshot => {
                let data: StateSnapshotData = from_data_value(envelope.data)?;
                Ok(Self::StateSnapshot {
                    snapshot: data.snapshot,
                })
            }
            AgentEventType::StateDelta => {
                let data: StateDeltaData = from_data_value(envelope.data)?;
                Ok(Self::StateDelta { delta: data.delta })
            }
            AgentEventType::MessagesSnapshot => {
                let data: MessagesSnapshotData = from_data_value(envelope.data)?;
                Ok(Self::MessagesSnapshot {
                    messages: data.messages,
                })
            }
            AgentEventType::ActivitySnapshot => {
                let data: ActivitySnapshotData = from_data_value(envelope.data)?;
                Ok(Self::ActivitySnapshot {
                    message_id: data.message_id,
                    activity_type: data.activity_type,
                    content: data.content,
                    replace: data.replace,
                })
            }
            AgentEventType::ActivityDelta => {
                let data: ActivityDeltaData = from_data_value(envelope.data)?;
                Ok(Self::ActivityDelta {
                    message_id: data.message_id,
                    activity_type: data.activity_type,
                    patch: data.patch,
                })
            }
            AgentEventType::InteractionRequested => {
                let data: InteractionRequestedData = from_data_value(envelope.data)?;
                Ok(Self::InteractionRequested {
                    interaction: data.interaction,
                })
            }
            AgentEventType::InteractionResolved => {
                let data: InteractionResolvedData = from_data_value(envelope.data)?;
                Ok(Self::InteractionResolved {
                    interaction_id: data.interaction_id,
                    result: data.result,
                })
            }
            AgentEventType::Pending => {
                let data: PendingData = from_data_value(envelope.data)?;
                Ok(Self::Pending {
                    interaction: data.interaction,
                })
            }
            AgentEventType::Error => {
                let data: ErrorData = from_data_value(envelope.data)?;
                Ok(Self::Error {
                    message: data.message,
                })
            }
        }
    }
}
