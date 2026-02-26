use super::envelope_meta::{
    clear_serialize_run_hint_if_matches, serialize_run_hint, set_serialize_run_hint,
    take_runtime_event_envelope_meta,
};
use super::event::{AgentEvent, AgentEventType};
use super::wire::{
    from_data_value, to_data_value, ActivityDeltaData, ActivitySnapshotData, EmptyData, ErrorData,
    EventEnvelope, InferenceCompleteData, MessagesSnapshotData, ReasoningDeltaData,
    ReasoningEncryptedValueData, RunFinishData, RunStartData, StateDeltaData, StateSnapshotData,
    StepStartData, TextDeltaData, ToolCallDeltaData, ToolCallDoneData, ToolCallReadyData,
    ToolCallResumedData, ToolCallStartData,
};
use crate::runtime::ToolCallOutcome;
use serde::{Deserialize, Serialize};

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
            _ => serialize_run_hint(),
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
            Self::ReasoningDelta { delta } => EventEnvelope {
                event_type: AgentEventType::ReasoningDelta,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ReasoningDeltaData {
                    delta: delta.clone(),
                })
                .map_err(serde::ser::Error::custom)?,
            },
            Self::ReasoningEncryptedValue { encrypted_value } => EventEnvelope {
                event_type: AgentEventType::ReasoningEncryptedValue,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ReasoningEncryptedValueData {
                    encrypted_value: encrypted_value.clone(),
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
                    outcome: ToolCallOutcome::from_tool_result(result),
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
            Self::ToolCallResumed { target_id, result } => EventEnvelope {
                event_type: AgentEventType::ToolCallResumed,
                run_id: meta_run_id(),
                thread_id: meta_thread_id(),
                seq: meta_seq(),
                timestamp_ms: meta_timestamp_ms(),
                step_id: meta_step_id(),
                data: to_data_value(&ToolCallResumedData {
                    target_id: target_id.clone(),
                    result: result.clone(),
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
                set_serialize_run_hint(hint.0, hint.1);
            }
            Self::RunFinish {
                run_id, thread_id, ..
            } => {
                let finished = runtime_meta
                    .as_ref()
                    .map(|meta| (meta.run_id.clone(), meta.thread_id.clone()))
                    .unwrap_or_else(|| (run_id.clone(), thread_id.clone()));
                clear_serialize_run_hint_if_matches(&finished.0, &finished.1);
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
            AgentEventType::ReasoningDelta => {
                let data: ReasoningDeltaData = from_data_value(envelope.data)?;
                Ok(Self::ReasoningDelta { delta: data.delta })
            }
            AgentEventType::ReasoningEncryptedValue => {
                let data: ReasoningEncryptedValueData = from_data_value(envelope.data)?;
                Ok(Self::ReasoningEncryptedValue {
                    encrypted_value: data.encrypted_value,
                })
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
                let _outcome = data.outcome;
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
            AgentEventType::ToolCallResumed => {
                let data: ToolCallResumedData = from_data_value(envelope.data)?;
                Ok(Self::ToolCallResumed {
                    target_id: data.target_id,
                    result: data.result,
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
