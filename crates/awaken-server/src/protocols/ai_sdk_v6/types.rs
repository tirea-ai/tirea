//! AI SDK v6 UI Message Stream protocol types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stream event types compatible with AI SDK v6.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UIStreamEvent {
    /// Message lifecycle start.
    #[serde(rename = "start")]
    MessageStart {
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    /// Text block start.
    TextStart {
        id: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Incremental text content.
    TextDelta {
        id: String,
        delta: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Text block end.
    TextEnd {
        id: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Reasoning block start.
    ReasoningStart {
        id: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Incremental reasoning content.
    ReasoningDelta {
        id: String,
        delta: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Reasoning block end.
    ReasoningEnd {
        id: String,
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Tool input streaming start.
    ToolInputStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },

    /// Incremental tool input.
    ToolInputDelta {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "inputTextDelta")]
        input_text_delta: String,
    },

    /// Tool input complete.
    ToolInputAvailable {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },

    /// Tool output available.
    ToolOutputAvailable {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        output: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },

    /// Tool output error.
    ToolOutputError {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "errorText")]
        error_text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        dynamic: Option<bool>,
    },

    /// Tool output denied.
    ToolOutputDenied {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
    },

    /// Tool approval request.
    ToolApprovalRequest {
        #[serde(rename = "approvalId")]
        approval_id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
    },

    /// Step start.
    StartStep,

    /// Step end.
    FinishStep,

    /// Message completion.
    Finish {
        #[serde(rename = "finishReason", skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    /// Stream error.
    Error {
        #[serde(rename = "errorText")]
        error_text: String,
    },

    /// Custom data event.
    #[serde(untagged)]
    Data {
        #[serde(rename = "type")]
        data_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        data: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        transient: Option<bool>,
    },
}

impl UIStreamEvent {
    pub fn message_start(message_id: impl Into<String>) -> Self {
        Self::MessageStart {
            message_id: Some(message_id.into()),
            message_metadata: None,
        }
    }

    pub fn text_start(id: impl Into<String>) -> Self {
        Self::TextStart {
            id: id.into(),
            provider_metadata: None,
        }
    }

    pub fn text_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::TextDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        }
    }

    pub fn text_end(id: impl Into<String>) -> Self {
        Self::TextEnd {
            id: id.into(),
            provider_metadata: None,
        }
    }

    pub fn reasoning_start(id: impl Into<String>) -> Self {
        Self::ReasoningStart {
            id: id.into(),
            provider_metadata: None,
        }
    }

    pub fn reasoning_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ReasoningDelta {
            id: id.into(),
            delta: delta.into(),
            provider_metadata: None,
        }
    }

    pub fn reasoning_end(id: impl Into<String>) -> Self {
        Self::ReasoningEnd {
            id: id.into(),
            provider_metadata: None,
        }
    }

    pub fn tool_input_start(tool_call_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self::ToolInputStart {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            dynamic: None,
        }
    }

    pub fn tool_input_delta(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ToolInputDelta {
            tool_call_id: tool_call_id.into(),
            input_text_delta: delta.into(),
        }
    }

    pub fn tool_input_available(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
    ) -> Self {
        Self::ToolInputAvailable {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
            dynamic: None,
        }
    }

    pub fn tool_output_available(tool_call_id: impl Into<String>, output: Value) -> Self {
        Self::ToolOutputAvailable {
            tool_call_id: tool_call_id.into(),
            output,
            dynamic: None,
        }
    }

    pub fn tool_output_error(
        tool_call_id: impl Into<String>,
        error_text: impl Into<String>,
    ) -> Self {
        Self::ToolOutputError {
            tool_call_id: tool_call_id.into(),
            error_text: error_text.into(),
            dynamic: None,
        }
    }

    pub fn tool_output_denied(tool_call_id: impl Into<String>) -> Self {
        Self::ToolOutputDenied {
            tool_call_id: tool_call_id.into(),
        }
    }

    pub fn tool_approval_request(
        approval_id: impl Into<String>,
        tool_call_id: impl Into<String>,
    ) -> Self {
        Self::ToolApprovalRequest {
            approval_id: approval_id.into(),
            tool_call_id: tool_call_id.into(),
        }
    }

    pub fn start_step() -> Self {
        Self::StartStep
    }

    pub fn finish_step() -> Self {
        Self::FinishStep
    }

    pub fn finish() -> Self {
        Self::Finish {
            finish_reason: None,
            message_metadata: None,
        }
    }

    pub fn finish_with_reason(reason: impl Into<String>) -> Self {
        Self::Finish {
            finish_reason: Some(reason.into()),
            message_metadata: None,
        }
    }

    pub fn error(error_text: impl Into<String>) -> Self {
        Self::Error {
            error_text: error_text.into(),
        }
    }

    pub fn data(name: impl Into<String>, data: Value) -> Self {
        Self::data_with_options(name, data, None, None)
    }

    pub fn data_with_options(
        name: impl Into<String>,
        data: Value,
        id: Option<String>,
        transient: Option<bool>,
    ) -> Self {
        let name = name.into();
        let data_type = if name.starts_with("data-") {
            name
        } else {
            format!("data-{name}")
        };
        Self::Data {
            data_type,
            id,
            data,
            transient,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_start_serde_roundtrip() {
        let event = UIStreamEvent::message_start("msg-1");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"start\""));
        assert!(json.contains("\"messageId\":\"msg-1\""));
    }

    #[test]
    fn text_delta_serde_roundtrip() {
        let event = UIStreamEvent::text_delta("txt_0", "hello");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"text-delta\""));
        assert!(json.contains("\"delta\":\"hello\""));
    }

    #[test]
    fn tool_input_start_serde() {
        let event = UIStreamEvent::tool_input_start("c1", "search");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"tool-input-start\""));
        assert!(json.contains("\"toolCallId\":\"c1\""));
    }

    #[test]
    fn finish_omits_none_fields() {
        let event = UIStreamEvent::finish();
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("finishReason"));
    }

    #[test]
    fn data_event_prepends_data_prefix() {
        let event = UIStreamEvent::data("activity-snapshot", json!({"key": "val"}));
        match &event {
            UIStreamEvent::Data { data_type, .. } => {
                assert_eq!(data_type, "data-activity-snapshot");
            }
            _ => panic!("expected Data variant"),
        }
    }

    #[test]
    fn data_event_preserves_existing_prefix() {
        let event = UIStreamEvent::data("data-custom", json!(null));
        match &event {
            UIStreamEvent::Data { data_type, .. } => {
                assert_eq!(data_type, "data-custom");
            }
            _ => panic!("expected Data variant"),
        }
    }

    #[test]
    fn error_event_serde() {
        let event = UIStreamEvent::error("something failed");
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"errorText\":\"something failed\""));
    }

    #[test]
    fn step_events_serde() {
        let start = UIStreamEvent::start_step();
        let end = UIStreamEvent::finish_step();
        let start_json = serde_json::to_string(&start).unwrap();
        let end_json = serde_json::to_string(&end).unwrap();
        assert!(start_json.contains("\"type\":\"start-step\""));
        assert!(end_json.contains("\"type\":\"finish-step\""));
    }

    #[test]
    fn reasoning_events_serde() {
        let start = UIStreamEvent::reasoning_start("r1");
        let delta = UIStreamEvent::reasoning_delta("r1", "thinking...");
        let end = UIStreamEvent::reasoning_end("r1");

        let s_json = serde_json::to_string(&start).unwrap();
        let d_json = serde_json::to_string(&delta).unwrap();
        let e_json = serde_json::to_string(&end).unwrap();

        assert!(s_json.contains("reasoning-start"));
        assert!(d_json.contains("thinking..."));
        assert!(e_json.contains("reasoning-end"));
    }

    #[test]
    fn tool_output_events_serde() {
        let available = UIStreamEvent::tool_output_available("c1", json!(42));
        let error = UIStreamEvent::tool_output_error("c1", "fail");
        let denied = UIStreamEvent::tool_output_denied("c1");

        let a_json = serde_json::to_string(&available).unwrap();
        let e_json = serde_json::to_string(&error).unwrap();
        let d_json = serde_json::to_string(&denied).unwrap();

        assert!(a_json.contains("tool-output-available"));
        assert!(e_json.contains("tool-output-error"));
        assert!(d_json.contains("tool-output-denied"));
    }
}
