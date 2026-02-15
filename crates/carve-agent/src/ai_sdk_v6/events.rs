use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stream event types compatible with AI SDK v6.
///
/// These events map directly to the AI SDK UI Message Stream protocol.
/// See: https://ai-sdk.dev/docs/ai-sdk-ui/stream-protocol
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UIStreamEvent {
    // ========================================================================
    // Message Lifecycle
    // ========================================================================
    /// Indicates the beginning of a new message with metadata.
    ///
    /// AI SDK v6 expects this as `{"type":"start","messageId":"..."}`.
    #[serde(rename = "start")]
    MessageStart {
        /// Unique identifier for this message.
        #[serde(rename = "messageId", skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        /// Optional message metadata.
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    // ========================================================================
    // Text Streaming (start/delta/end pattern)
    // ========================================================================
    /// Indicates the beginning of a text block.
    TextStart {
        /// Unique identifier for this text block.
        id: String,
    },

    /// Contains incremental text content for the text block.
    TextDelta {
        /// Identifier matching the text-start event.
        id: String,
        /// Incremental text content.
        delta: String,
    },

    /// Indicates the end of a text block.
    TextEnd {
        /// Identifier matching the text-start event.
        id: String,
    },

    // ========================================================================
    // Reasoning Streaming (for models like DeepSeek, o1)
    // ========================================================================
    /// Indicates the beginning of a reasoning block.
    ReasoningStart {
        /// Unique identifier for this reasoning block.
        id: String,
    },

    /// Contains incremental reasoning content.
    ReasoningDelta {
        /// Identifier matching the reasoning-start event.
        id: String,
        /// Incremental reasoning content.
        delta: String,
    },

    /// Indicates the end of a reasoning block.
    ReasoningEnd {
        /// Identifier matching the reasoning-start event.
        id: String,
    },

    // ========================================================================
    // Tool Input Streaming
    // ========================================================================
    /// Indicates the beginning of tool input streaming.
    ToolInputStart {
        /// Unique identifier for this tool call.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Name of the tool being called.
        #[serde(rename = "toolName")]
        tool_name: String,
    },

    /// Contains incremental chunks of tool input as it's being generated.
    ToolInputDelta {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Incremental tool input text.
        #[serde(rename = "inputTextDelta")]
        input_text_delta: String,
    },

    /// Indicates that tool input is complete and ready for execution.
    ToolInputAvailable {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Name of the tool being called.
        #[serde(rename = "toolName")]
        tool_name: String,
        /// Complete tool input as JSON.
        input: Value,
    },

    // ========================================================================
    // Tool Output
    // ========================================================================
    /// Contains the result of tool execution.
    ToolOutputAvailable {
        /// Identifier matching the tool-input-start event.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Tool execution result as JSON.
        output: Value,
    },

    // ========================================================================
    // Step Boundaries (for multi-step agents)
    // ========================================================================
    /// Marks the beginning of a step.
    StartStep,

    /// Marks the completion of an LLM API call step.
    FinishStep,

    // ========================================================================
    // Source References (for RAG)
    // ========================================================================
    /// References an external URL.
    SourceUrl {
        /// Unique identifier for this source.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// The URL being referenced.
        url: String,
        /// Optional title for the source.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },

    /// References a document or file.
    SourceDocument {
        /// Unique identifier for this source.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// IANA media type of the document.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Title of the document.
        title: String,
        /// Optional filename.
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
    },

    /// Contains a file reference.
    ///
    /// AI SDK v6 strict schema only includes `url`, `mediaType`, and optional `providerMetadata`.
    File {
        /// URL to the file.
        url: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    // ========================================================================
    // Stream Lifecycle
    // ========================================================================
    /// Indicates message completion.
    Finish {
        /// Optional reason for finishing (stop, length, content-filter, tool-calls, error, other).
        #[serde(rename = "finishReason", skip_serializing_if = "Option::is_none")]
        finish_reason: Option<String>,
        /// Optional message metadata.
        #[serde(rename = "messageMetadata", skip_serializing_if = "Option::is_none")]
        message_metadata: Option<Value>,
    },

    /// Signals stream abortion with a reason.
    Abort {
        /// Optional reason for the abort.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// Appends error messages to stream.
    Error {
        /// Error text.
        #[serde(rename = "errorText")]
        error_text: String,
    },

    // ========================================================================
    // Custom Data (extensible)
    // ========================================================================
    /// Custom data event with a type prefix pattern (data-*).
    #[serde(untagged)]
    Data {
        /// Custom type name (should start with "data-").
        #[serde(rename = "type")]
        data_type: String,
        /// Custom data payload.
        data: Value,
    },
}

impl UIStreamEvent {
    // ========================================================================
    // Factory Methods
    // ========================================================================

    /// Create a start event (message start).
    pub fn message_start(message_id: impl Into<String>) -> Self {
        Self::MessageStart {
            message_id: Some(message_id.into()),
            message_metadata: None,
        }
    }

    /// Create a start event without message ID.
    pub fn start() -> Self {
        Self::MessageStart {
            message_id: None,
            message_metadata: None,
        }
    }

    /// Create a text-start event.
    pub fn text_start(id: impl Into<String>) -> Self {
        Self::TextStart { id: id.into() }
    }

    /// Create a text-delta event.
    pub fn text_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::TextDelta {
            id: id.into(),
            delta: delta.into(),
        }
    }

    /// Create a text-end event.
    pub fn text_end(id: impl Into<String>) -> Self {
        Self::TextEnd { id: id.into() }
    }

    /// Create a reasoning-start event.
    pub fn reasoning_start(id: impl Into<String>) -> Self {
        Self::ReasoningStart { id: id.into() }
    }

    /// Create a reasoning-delta event.
    pub fn reasoning_delta(id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ReasoningDelta {
            id: id.into(),
            delta: delta.into(),
        }
    }

    /// Create a reasoning-end event.
    pub fn reasoning_end(id: impl Into<String>) -> Self {
        Self::ReasoningEnd { id: id.into() }
    }

    /// Create a tool-input-start event.
    pub fn tool_input_start(tool_call_id: impl Into<String>, tool_name: impl Into<String>) -> Self {
        Self::ToolInputStart {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
        }
    }

    /// Create a tool-input-delta event.
    pub fn tool_input_delta(tool_call_id: impl Into<String>, delta: impl Into<String>) -> Self {
        Self::ToolInputDelta {
            tool_call_id: tool_call_id.into(),
            input_text_delta: delta.into(),
        }
    }

    /// Create a tool-input-available event.
    pub fn tool_input_available(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        input: Value,
    ) -> Self {
        Self::ToolInputAvailable {
            tool_call_id: tool_call_id.into(),
            tool_name: tool_name.into(),
            input,
        }
    }

    /// Create a tool-output-available event.
    pub fn tool_output_available(tool_call_id: impl Into<String>, output: Value) -> Self {
        Self::ToolOutputAvailable {
            tool_call_id: tool_call_id.into(),
            output,
        }
    }

    /// Create a start-step event.
    pub fn start_step() -> Self {
        Self::StartStep
    }

    /// Create a finish-step event.
    pub fn finish_step() -> Self {
        Self::FinishStep
    }

    /// Create a source-url event.
    pub fn source_url(
        source_id: impl Into<String>,
        url: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        Self::SourceUrl {
            source_id: source_id.into(),
            url: url.into(),
            title,
        }
    }

    /// Create a source-document event.
    pub fn source_document(
        source_id: impl Into<String>,
        media_type: impl Into<String>,
        title: impl Into<String>,
        filename: Option<String>,
    ) -> Self {
        Self::SourceDocument {
            source_id: source_id.into(),
            media_type: media_type.into(),
            title: title.into(),
            filename,
        }
    }

    /// Create a file event.
    pub fn file(url: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self::File {
            url: url.into(),
            media_type: media_type.into(),
            provider_metadata: None,
        }
    }

    /// Create a finish event.
    pub fn finish() -> Self {
        Self::Finish {
            finish_reason: None,
            message_metadata: None,
        }
    }

    /// Create a finish event with a reason.
    pub fn finish_with_reason(reason: impl Into<String>) -> Self {
        Self::Finish {
            finish_reason: Some(reason.into()),
            message_metadata: None,
        }
    }

    /// Create an abort event.
    pub fn abort(reason: impl Into<String>) -> Self {
        Self::Abort {
            reason: Some(reason.into()),
        }
    }

    /// Create an error event.
    pub fn error(error_text: impl Into<String>) -> Self {
        Self::Error {
            error_text: error_text.into(),
        }
    }

    /// Create a custom data event.
    pub fn data(name: impl Into<String>, data: Value) -> Self {
        let data_type = format!("data-{}", name.into());
        Self::Data { data_type, data }
    }
}
