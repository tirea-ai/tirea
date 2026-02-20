use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Streaming state for UI parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    /// Content is still streaming.
    Streaming,
    /// Content streaming is complete.
    Done,
}

/// Tool execution state in UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolState {
    /// Tool input is being streamed.
    InputStreaming,
    /// Tool input is complete, ready for execution.
    InputAvailable,
    /// Tool execution completed with output.
    OutputAvailable,
    /// Tool execution resulted in error.
    OutputError,
}

/// A part of a UI message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UIMessagePart {
    /// Text content part.
    Text {
        /// The text content.
        text: String,
        /// Optional streaming state.
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<StreamState>,
    },

    /// Reasoning content part.
    Reasoning {
        /// The reasoning text.
        text: String,
        /// Optional streaming state.
        #[serde(skip_serializing_if = "Option::is_none")]
        state: Option<StreamState>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Tool invocation part.
    #[serde(rename = "tool-invocation")]
    Tool {
        /// Tool call identifier.
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        /// Tool name.
        #[serde(rename = "toolName")]
        tool_name: String,
        /// Tool execution state.
        state: ToolState,
        /// Tool input (available after input-available).
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        /// Tool output (available after output-available).
        #[serde(skip_serializing_if = "Option::is_none")]
        output: Option<Value>,
        /// Error text (if output-error state).
        #[serde(rename = "errorText", skip_serializing_if = "Option::is_none")]
        error_text: Option<String>,
    },

    /// URL source reference.
    SourceUrl {
        /// Source identifier.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// The URL.
        url: String,
        /// Optional title.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Document source reference.
    SourceDocument {
        /// Source identifier.
        #[serde(rename = "sourceId")]
        source_id: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
        /// Document title.
        title: String,
        /// Optional filename.
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        /// Optional provider metadata.
        #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// File attachment.
    File {
        /// File URL.
        url: String,
        /// IANA media type.
        #[serde(rename = "mediaType")]
        media_type: String,
    },

    /// Step start marker.
    StepStart,

    /// Custom data part.
    #[serde(untagged)]
    Data {
        /// Custom type.
        #[serde(rename = "type")]
        data_type: String,
        /// Data payload.
        data: Value,
    },
}

/// Role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UIRole {
    /// System message.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
}

/// A UI message with rich parts.
///
/// This is the source of truth for application state, representing the complete
/// message including metadata, data parts, and all contextual information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UIMessage {
    /// Unique identifier for this message.
    pub id: String,
    /// Role of the message sender.
    pub role: UIRole,
    /// Optional metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Message parts.
    pub parts: Vec<UIMessagePart>,
}

impl UIMessage {
    /// Create a new UI message.
    pub fn new(id: impl Into<String>, role: UIRole) -> Self {
        Self {
            id: id.into(),
            role,
            metadata: None,
            parts: Vec::new(),
        }
    }

    /// Create a user message.
    pub fn user(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: UIRole::User,
            metadata: None,
            parts: vec![UIMessagePart::Text {
                text: text.into(),
                state: Some(StreamState::Done),
            }],
        }
    }

    /// Create an assistant message.
    pub fn assistant(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            role: UIRole::Assistant,
            metadata: None,
            parts: Vec::new(),
        }
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Add a part.
    #[must_use]
    pub fn with_part(mut self, part: UIMessagePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Add multiple parts.
    #[must_use]
    pub fn with_parts(mut self, parts: impl IntoIterator<Item = UIMessagePart>) -> Self {
        self.parts.extend(parts);
        self
    }

    /// Add a text part.
    #[must_use]
    pub fn with_text(self, text: impl Into<String>) -> Self {
        self.with_part(UIMessagePart::Text {
            text: text.into(),
            state: Some(StreamState::Done),
        })
    }

    /// Get all text content concatenated.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                UIMessagePart::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}
