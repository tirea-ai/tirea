use super::definition::AgentEventType;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
