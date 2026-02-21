use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
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
    /// User approval has been requested for this tool call.
    ApprovalRequested,
    /// User has responded to the approval request.
    ApprovalResponded,
    /// Tool execution completed with output.
    OutputAvailable,
    /// Tool execution resulted in error.
    OutputError,
    /// Tool execution was denied.
    OutputDenied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TextPartType {
    #[serde(rename = "text")]
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReasoningPartType {
    #[serde(rename = "reasoning")]
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SourceUrlPartType {
    #[serde(rename = "source-url")]
    SourceUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SourceDocumentPartType {
    #[serde(rename = "source-document")]
    SourceDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FilePartType {
    #[serde(rename = "file")]
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum StepStartPartType {
    #[serde(rename = "step-start")]
    StepStart,
}

/// User approval details attached to a tool invocation part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolApproval {
    /// Approval request ID.
    pub id: String,
    /// Optional final decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved: Option<bool>,
    /// Optional reason from the approver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Text part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextUIPart {
    #[serde(rename = "type")]
    part_type: TextPartType,
    /// The text content.
    pub text: String,
    /// Optional streaming state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StreamState>,
    /// Optional provider metadata.
    #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

impl TextUIPart {
    pub fn new(text: impl Into<String>, state: Option<StreamState>) -> Self {
        Self {
            part_type: TextPartType::Text,
            text: text.into(),
            state,
            provider_metadata: None,
        }
    }
}

/// Reasoning part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningUIPart {
    #[serde(rename = "type")]
    part_type: ReasoningPartType,
    /// The reasoning text.
    pub text: String,
    /// Optional streaming state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<StreamState>,
    /// Optional provider metadata.
    #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

impl ReasoningUIPart {
    pub fn new(text: impl Into<String>, state: Option<StreamState>) -> Self {
        Self {
            part_type: ReasoningPartType::Reasoning,
            text: text.into(),
            state,
            provider_metadata: None,
        }
    }
}

/// Tool invocation part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolUIPart {
    /// Part type. Must be `dynamic-tool` or `tool-*`.
    #[serde(rename = "type", deserialize_with = "deserialize_tool_part_type")]
    pub part_type: String,
    /// Tool call identifier.
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    /// Tool name (required for `dynamic-tool`).
    #[serde(rename = "toolName", skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Optional display title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Whether the provider executed this tool directly.
    #[serde(rename = "providerExecuted", skip_serializing_if = "Option::is_none")]
    pub provider_executed: Option<bool>,
    /// Tool execution state.
    pub state: ToolState,
    /// Tool input payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    /// Tool output payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Tool error text.
    #[serde(rename = "errorText", skip_serializing_if = "Option::is_none")]
    pub error_text: Option<String>,
    /// Provider metadata associated with the tool input call.
    #[serde(
        rename = "callProviderMetadata",
        skip_serializing_if = "Option::is_none"
    )]
    pub call_provider_metadata: Option<Value>,
    /// Raw input when parsing fails (AI SDK compatibility for output-error).
    #[serde(rename = "rawInput", skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    /// Marks provisional tool outputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preliminary: Option<bool>,
    /// Optional approval state payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<ToolApproval>,
}

impl ToolUIPart {
    /// Create a static tool part (`type = tool-{name}`).
    pub fn static_tool(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        state: ToolState,
    ) -> Self {
        let tool_name = tool_name.into();
        Self {
            part_type: format!("tool-{tool_name}"),
            tool_call_id: tool_call_id.into(),
            tool_name: None,
            title: None,
            provider_executed: None,
            state,
            input: None,
            output: None,
            error_text: None,
            call_provider_metadata: None,
            raw_input: None,
            preliminary: None,
            approval: None,
        }
    }

    /// Create a dynamic tool part (`type = dynamic-tool`).
    pub fn dynamic_tool(
        tool_name: impl Into<String>,
        tool_call_id: impl Into<String>,
        state: ToolState,
    ) -> Self {
        Self {
            part_type: "dynamic-tool".to_string(),
            tool_call_id: tool_call_id.into(),
            tool_name: Some(tool_name.into()),
            title: None,
            provider_executed: None,
            state,
            input: None,
            output: None,
            error_text: None,
            call_provider_metadata: None,
            raw_input: None,
            preliminary: None,
            approval: None,
        }
    }
}

/// URL source reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceUrlUIPart {
    #[serde(rename = "type")]
    part_type: SourceUrlPartType,
    /// Source identifier.
    #[serde(rename = "sourceId")]
    pub source_id: String,
    /// The URL.
    pub url: String,
    /// Optional title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional provider metadata.
    #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

/// Document source reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceDocumentUIPart {
    #[serde(rename = "type")]
    part_type: SourceDocumentPartType,
    /// Source identifier.
    #[serde(rename = "sourceId")]
    pub source_id: String,
    /// IANA media type.
    #[serde(rename = "mediaType")]
    pub media_type: String,
    /// Document title.
    pub title: String,
    /// Optional filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Optional provider metadata.
    #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

/// File attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileUIPart {
    #[serde(rename = "type")]
    part_type: FilePartType,
    /// File URL.
    pub url: String,
    /// IANA media type.
    #[serde(rename = "mediaType")]
    pub media_type: String,
    /// Optional filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Optional provider metadata.
    #[serde(rename = "providerMetadata", skip_serializing_if = "Option::is_none")]
    pub provider_metadata: Option<Value>,
}

/// Step start marker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepStartUIPart {
    #[serde(rename = "type")]
    part_type: StepStartPartType,
}

/// Custom data part.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataUIPart {
    /// Custom type, must start with `data-`.
    #[serde(rename = "type", deserialize_with = "deserialize_data_part_type")]
    pub data_type: String,
    /// Optional stable data part ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Data payload.
    pub data: Value,
}

/// A part of a UI message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum UIMessagePart {
    /// Text content part.
    Text(TextUIPart),
    /// Reasoning content part.
    Reasoning(ReasoningUIPart),
    /// Tool invocation part.
    Tool(ToolUIPart),
    /// URL source reference.
    SourceUrl(SourceUrlUIPart),
    /// Document source reference.
    SourceDocument(SourceDocumentUIPart),
    /// File attachment.
    File(FileUIPart),
    /// Step start marker.
    StepStart(StepStartUIPart),
    /// Custom data part.
    Data(DataUIPart),
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
            parts: vec![UIMessagePart::Text(TextUIPart::new(
                text,
                Some(StreamState::Done),
            ))],
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
        self.with_part(UIMessagePart::Text(TextUIPart::new(
            text,
            Some(StreamState::Done),
        )))
    }

    /// Get all text content concatenated.
    pub fn text_content(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                UIMessagePart::Text(part) => Some(part.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

fn deserialize_tool_part_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == "dynamic-tool" || value.starts_with("tool-") {
        Ok(value)
    } else {
        Err(D::Error::custom(format!(
            "invalid tool part type '{value}', expected 'dynamic-tool' or 'tool-*'"
        )))
    }
}

fn deserialize_data_part_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value.starts_with("data-") {
        Ok(value)
    } else {
        Err(D::Error::custom(format!(
            "invalid data part type '{value}', expected 'data-*'"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn static_tool_part_serializes_with_tool_prefix_type() {
        let mut part = ToolUIPart::static_tool("search", "call_1", ToolState::InputAvailable);
        part.input = Some(json!({ "q": "rust" }));
        let value = serde_json::to_value(UIMessagePart::Tool(part)).expect("serialize tool part");

        assert_eq!(value["type"], "tool-search");
        assert_eq!(value["toolCallId"], "call_1");
        assert!(value.get("toolName").is_none());
    }

    #[test]
    fn dynamic_tool_part_serializes_with_tool_name() {
        let part = ToolUIPart::dynamic_tool("search", "call_2", ToolState::InputStreaming);
        let value = serde_json::to_value(UIMessagePart::Tool(part)).expect("serialize tool part");

        assert_eq!(value["type"], "dynamic-tool");
        assert_eq!(value["toolCallId"], "call_2");
        assert_eq!(value["toolName"], "search");
    }

    #[test]
    fn tool_part_rejects_invalid_type() {
        let err = serde_json::from_value::<ToolUIPart>(json!({
            "type": "tool",
            "toolCallId": "call_1",
            "state": "input-available"
        }))
        .expect_err("invalid tool type must be rejected");

        assert!(err.to_string().contains("dynamic-tool"));
    }

    #[test]
    fn data_part_rejects_invalid_type_prefix() {
        let err = serde_json::from_value::<DataUIPart>(json!({
            "type": "reasoning-encrypted",
            "data": { "v": 1 }
        }))
        .expect_err("invalid data type must be rejected");

        assert!(err.to_string().contains("data-*"));
    }

    #[test]
    fn file_part_roundtrip_preserves_filename_and_provider_metadata() {
        let part = UIMessagePart::File(FileUIPart {
            part_type: FilePartType::File,
            url: "https://example.com/a.png".to_string(),
            media_type: "image/png".to_string(),
            filename: Some("a.png".to_string()),
            provider_metadata: Some(json!({ "source": "upload" })),
        });

        let raw = serde_json::to_string(&part).expect("serialize file part");
        let restored: UIMessagePart = serde_json::from_str(&raw).expect("deserialize file part");

        assert!(matches!(
            restored,
            UIMessagePart::File(FileUIPart {
                filename: Some(filename),
                provider_metadata: Some(provider_metadata),
                ..
            }) if filename == "a.png" && provider_metadata["source"] == "upload"
        ));
    }
}
