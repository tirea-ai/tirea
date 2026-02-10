//! Core types for Agent messages and tool calls.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Optional stable message identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: Role,
    pub content: String,
    /// Tool calls made by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID this message responds to (for tool role).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: Role::System,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: Role::User,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: Role::Assistant,
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message with tool calls.
    pub fn assistant_with_tool_calls(content: impl Into<String>, calls: Vec<ToolCall>) -> Self {
        Self {
            id: None,
            role: Role::Assistant,
            content: content.into(),
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
            tool_call_id: None,
        }
    }

    /// Create a tool response message.
    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: None,
            role: Role::Tool,
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique identifier for this tool call.
    pub id: String,
    /// Name of the tool to call.
    pub name: String,
    /// Arguments for the tool as JSON.
    pub arguments: Value,
}

impl ToolCall {
    /// Create a new tool call.
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_user_message() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello");
        assert!(msg.tool_calls.is_none());
        assert!(msg.tool_call_id.is_none());
    }

    #[test]
    fn test_assistant_with_tool_calls() {
        let calls = vec![ToolCall::new("call_1", "search", json!({"query": "rust"}))];
        let msg = Message::assistant_with_tool_calls("Let me search", calls);

        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "Let me search");
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_tool_message() {
        let msg = Message::tool("call_1", "Result: 42");

        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.content, "Result: 42");
        assert_eq!(msg.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message::user("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"role\":\"user\""));
        // tool_calls and tool_call_id should be omitted when None
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn test_tool_call_serialization() {
        let call = ToolCall::new("id_1", "calculator", json!({"expr": "2+2"}));
        let json = serde_json::to_string(&call).unwrap();
        let parsed: ToolCall = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "id_1");
        assert_eq!(parsed.name, "calculator");
        assert_eq!(parsed.arguments["expr"], "2+2");
    }
}
