use super::event::AgentEventType;
use crate::event::termination::TerminationReason;
use crate::runtime::ToolCallOutcome;
use crate::tool::contract::ToolResult;
use crate::runtime::result::TokenUsage;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tirea_state::TrackedPatch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EventEnvelope {
    #[serde(rename = "type")]
    pub event_type: AgentEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunStartData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RunFinishData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    pub termination: TerminationReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TextDeltaData {
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReasoningDeltaData {
    pub delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReasoningEncryptedValueData {
    pub encrypted_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolCallStartData {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolCallDeltaData {
    pub id: String,
    pub args_delta: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolCallReadyData {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolCallDoneData {
    pub id: String,
    pub result: ToolResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<TrackedPatch>,
    #[serde(default)]
    pub message_id: String,
    #[serde(default = "default_tool_call_outcome")]
    pub outcome: ToolCallOutcome,
}

const fn default_tool_call_outcome() -> ToolCallOutcome {
    ToolCallOutcome::Succeeded
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StepStartData {
    #[serde(default)]
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct InferenceCompleteData {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StateSnapshotData {
    pub snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StateDeltaData {
    pub delta: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MessagesSnapshotData {
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ActivitySnapshotData {
    pub message_id: String,
    pub activity_type: String,
    pub content: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ActivityDeltaData {
    pub message_id: String,
    pub activity_type: String,
    pub patch: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolCallResumedData {
    pub target_id: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ErrorData {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmptyData {}

pub(crate) fn to_data_value<T: Serialize>(value: &T) -> serde_json::Result<Option<Value>> {
    let encoded = serde_json::to_value(value)?;
    if matches!(encoded, Value::Object(ref o) if o.is_empty()) {
        Ok(None)
    } else {
        Ok(Some(encoded))
    }
}

pub(crate) fn from_data_value<T, E>(value: Option<Value>) -> Result<T, E>
where
    T: DeserializeOwned,
    E: serde::de::Error,
{
    let data = value.unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    serde_json::from_value(data).map_err(E::custom)
}
