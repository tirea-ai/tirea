use crate::ag_ui::{AGUIContext, AGUIEvent, AGUIMessage, MessageRole, RunAgentRequest};
use crate::ui_stream::{AiSdkEncoder, StreamState, ToolState, UIMessage, UIMessagePart, UIRole, UIStreamEvent};
use crate::{gen_message_id, AgentEvent, Message, Role, Visibility};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Target AI SDK major version for the protocol adapters in this crate.
pub const AI_SDK_VERSION: &str = "v6";

/// Protocol input boundary:
/// protocol request -> internal `RunRequest`.
pub trait ProtocolInputAdapter {
    type Request;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest;
}

/// Protocol history boundary:
/// stored `Message` → protocol-specific history message format.
///
/// This is the reverse of [`ProtocolInputAdapter`]: while input adapters
/// convert protocol messages into internal `Message`s, this trait converts
/// stored internal `Message`s back into protocol-native formats for REST
/// query endpoints (e.g. loading chat history on page refresh).
pub trait ProtocolHistoryEncoder {
    type HistoryMessage: Serialize;

    fn encode_message(msg: &Message) -> Self::HistoryMessage;

    fn encode_messages<'a>(msgs: impl IntoIterator<Item = &'a Message>) -> Vec<Self::HistoryMessage> {
        msgs.into_iter().map(Self::encode_message).collect()
    }
}

/// Protocol output boundary:
/// internal `AgentEvent` -> protocol event(s).
///
/// Transport layers (SSE/NATS/etc.) should depend on this trait and remain
/// agnostic to protocol-specific branching.
pub trait ProtocolOutputEncoder {
    type Event: Serialize;

    fn prologue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event>;

    fn epilogue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSdkV6RunRequest {
    #[serde(rename = "sessionId")]
    pub thread_id: String,
    pub input: String,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
}

pub struct AiSdkV6InputAdapter;

impl ProtocolInputAdapter for AiSdkV6InputAdapter {
    type Request = AiSdkV6RunRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest {
        crate::agent_os::RunRequest {
            agent_id,
            thread_id: if request.thread_id.trim().is_empty() {
                None
            } else {
                Some(request.thread_id)
            },
            run_id: request.run_id,
            resource_id: None,
            initial_state: None,
            messages: vec![Message::user(request.input)],
            runtime: std::collections::HashMap::new(),
        }
    }
}

pub struct AgUiInputAdapter;

impl ProtocolInputAdapter for AgUiInputAdapter {
    type Request = RunAgentRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> crate::agent_os::RunRequest {
        let mut runtime = std::collections::HashMap::new();
        if let Some(parent_run_id) = request.parent_run_id.clone() {
            runtime.insert(
                "parent_run_id".to_string(),
                serde_json::Value::String(parent_run_id),
            );
        }

        crate::agent_os::RunRequest {
            agent_id,
            thread_id: Some(request.thread_id),
            run_id: Some(request.run_id),
            resource_id: None,
            initial_state: request.state,
            messages: convert_agui_messages(&request.messages),
            runtime,
        }
    }
}

impl ProtocolHistoryEncoder for AgUiInputAdapter {
    type HistoryMessage = AGUIMessage;

    /// Reverse of [`convert_agui_messages`]: maps internal `Message` back to
    /// AG-UI's `AGUIMessage` format with correct camelCase field names.
    fn encode_message(msg: &Message) -> AGUIMessage {
        AGUIMessage {
            id: msg.id.clone(),
            role: match msg.role {
                Role::System => MessageRole::System,
                Role::User => MessageRole::User,
                Role::Assistant => MessageRole::Assistant,
                Role::Tool => MessageRole::Tool,
            },
            content: msg.content.clone(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}

impl ProtocolHistoryEncoder for AiSdkV6InputAdapter {
    type HistoryMessage = UIMessage;

    /// Maps internal `Message` to AI SDK v6 `UIMessage` with `parts` structure.
    ///
    /// - `Message.content` → `UIMessagePart::Text`
    /// - `Message.tool_calls` → `UIMessagePart::Tool` per call
    /// - `Role::Tool` → `UIRole::Assistant` (tool results are assistant-side in AI SDK)
    fn encode_message(msg: &Message) -> UIMessage {
        let role = match msg.role {
            Role::System => UIRole::System,
            Role::User => UIRole::User,
            Role::Assistant | Role::Tool => UIRole::Assistant,
        };

        let mut parts = Vec::new();

        if !msg.content.is_empty() {
            parts.push(UIMessagePart::Text {
                text: msg.content.clone(),
                state: Some(StreamState::Done),
            });
        }

        if let Some(ref calls) = msg.tool_calls {
            for tc in calls {
                parts.push(UIMessagePart::Tool {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    state: ToolState::OutputAvailable,
                    input: Some(tc.arguments.clone()),
                    output: None,
                    error_text: None,
                });
            }
        }

        UIMessage {
            id: msg.id.clone().unwrap_or_default(),
            role,
            metadata: msg.metadata.as_ref().and_then(|m| serde_json::to_value(m).ok()),
            parts,
        }
    }
}

fn convert_agui_messages(messages: &[AGUIMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| m.role != MessageRole::Assistant)
        .map(|m| {
            let role = match m.role {
                MessageRole::System | MessageRole::Developer => Role::System,
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::Tool => Role::Tool,
            };
            Message {
                id: Some(m.id.clone().unwrap_or_else(gen_message_id)),
                role,
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: m.tool_call_id.clone(),
                visibility: Visibility::default(),
                metadata: None,
            }
        })
        .collect()
}

pub struct AiSdkV6ProtocolEncoder {
    inner: AiSdkEncoder,
    run_info: Option<UIStreamEvent>,
}

impl AiSdkV6ProtocolEncoder {
    pub fn new(run_id: String, thread_id: Option<String>) -> Self {
        let run_info = thread_id.map(|thread_id| {
            UIStreamEvent::data(
                "run-info",
                json!({
                    "protocol": "ai-sdk-ui-message-stream",
                    "protocolVersion": "v1",
                    "aiSdkVersion": AI_SDK_VERSION,
                    "threadId": thread_id,
                    "runId": run_id
                }),
            )
        });
        Self {
            inner: AiSdkEncoder::new(run_id),
            run_info,
        }
    }
}

impl ProtocolOutputEncoder for AiSdkV6ProtocolEncoder {
    type Event = UIStreamEvent;

    fn prologue(&mut self) -> Vec<Self::Event> {
        let mut events = self.inner.prologue();
        if let Some(run_info) = self.run_info.take() {
            events.push(run_info);
        }
        events
    }

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.inner.on_agent_event(ev)
    }
}

pub struct AgUiProtocolEncoder {
    ctx: AGUIContext,
}

impl AgUiProtocolEncoder {
    pub fn new(thread_id: String, run_id: String) -> Self {
        Self {
            ctx: AGUIContext::new(thread_id, run_id),
        }
    }
}

impl ProtocolOutputEncoder for AgUiProtocolEncoder {
    type Event = AGUIEvent;

    fn on_agent_event(&mut self, ev: &AgentEvent) -> Vec<Self::Event> {
        self.ctx.on_agent_event(ev)
    }

    fn epilogue(&mut self) -> Vec<Self::Event> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MessageMetadata, ToolCall};

    // ========================================================================
    // AG-UI History Encoder Tests
    // ========================================================================

    #[test]
    fn test_agui_history_encoder_user_message() {
        let msg = Message {
            id: Some("msg_1".to_string()),
            role: Role::User,
            content: "hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AgUiInputAdapter::encode_message(&msg);
        assert_eq!(encoded.id, Some("msg_1".to_string()));
        assert_eq!(encoded.role, MessageRole::User);
        assert_eq!(encoded.content, "hello");
        assert!(encoded.tool_call_id.is_none());
    }

    #[test]
    fn test_agui_history_encoder_assistant_message() {
        let msg = Message {
            id: Some("msg_2".to_string()),
            role: Role::Assistant,
            content: "hi there".to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "rust"}),
            }]),
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AgUiInputAdapter::encode_message(&msg);
        assert_eq!(encoded.role, MessageRole::Assistant);
        assert_eq!(encoded.content, "hi there");
        // AG-UI doesn't carry tool_calls in AGUIMessage
        // (they're represented as TOOL_CALL_* events)
        assert!(encoded.tool_call_id.is_none());
    }

    #[test]
    fn test_agui_history_encoder_tool_message() {
        let msg = Message {
            id: Some("msg_3".to_string()),
            role: Role::Tool,
            content: "{\"result\":42}".to_string(),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AgUiInputAdapter::encode_message(&msg);
        assert_eq!(encoded.role, MessageRole::Tool);
        assert_eq!(encoded.tool_call_id, Some("call_1".to_string()));
        assert_eq!(encoded.content, "{\"result\":42}");
    }

    #[test]
    fn test_agui_history_encoder_system_message() {
        let msg = Message {
            id: None,
            role: Role::System,
            content: "You are helpful.".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AgUiInputAdapter::encode_message(&msg);
        assert_eq!(encoded.role, MessageRole::System);
        assert!(encoded.id.is_none());
    }

    #[test]
    fn test_agui_history_encoder_camelcase_serialization() {
        let msg = Message {
            id: Some("msg_1".to_string()),
            role: Role::Tool,
            content: "result".to_string(),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AgUiInputAdapter::encode_message(&msg);
        let json = serde_json::to_string(&encoded).unwrap();
        assert!(json.contains("toolCallId"));
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn test_agui_roundtrip_preserves_content() {
        // AGUIMessage → convert_agui_messages → Message → encode_message → AGUIMessage
        let original = AGUIMessage {
            id: Some("msg_1".to_string()),
            role: MessageRole::User,
            content: "hello world".to_string(),
            tool_call_id: None,
        };
        let internal = convert_agui_messages(&[original.clone()]);
        assert_eq!(internal.len(), 1);
        let roundtripped = AgUiInputAdapter::encode_message(&internal[0]);
        assert_eq!(roundtripped.id, original.id);
        assert_eq!(roundtripped.role, original.role);
        assert_eq!(roundtripped.content, original.content);
        assert_eq!(roundtripped.tool_call_id, original.tool_call_id);
    }

    #[test]
    fn test_agui_roundtrip_tool_message() {
        let original = AGUIMessage {
            id: Some("msg_t".to_string()),
            role: MessageRole::Tool,
            content: "tool output".to_string(),
            tool_call_id: Some("call_99".to_string()),
        };
        let internal = convert_agui_messages(&[original.clone()]);
        let roundtripped = AgUiInputAdapter::encode_message(&internal[0]);
        assert_eq!(roundtripped.role, MessageRole::Tool);
        assert_eq!(roundtripped.tool_call_id, Some("call_99".to_string()));
    }

    #[test]
    fn test_agui_encode_messages_batch() {
        let msgs = vec![
            Message::user("hello"),
            Message::assistant("world"),
        ];
        let encoded = AgUiInputAdapter::encode_messages(msgs.iter());
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[0].role, MessageRole::User);
        assert_eq!(encoded[1].role, MessageRole::Assistant);
    }

    // ========================================================================
    // AI SDK History Encoder Tests
    // ========================================================================

    #[test]
    fn test_ai_sdk_history_encoder_user_message() {
        let msg = Message {
            id: Some("msg_1".to_string()),
            role: Role::User,
            content: "hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        assert_eq!(encoded.id, "msg_1");
        assert_eq!(encoded.role, UIRole::User);
        assert_eq!(encoded.parts.len(), 1);
        assert!(matches!(
            &encoded.parts[0],
            UIMessagePart::Text { text, state: Some(StreamState::Done) } if text == "hello"
        ));
    }

    #[test]
    fn test_ai_sdk_history_encoder_assistant_with_tool_calls() {
        let msg = Message {
            id: Some("msg_2".to_string()),
            role: Role::Assistant,
            content: "Let me search.".to_string(),
            tool_calls: Some(vec![
                ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: serde_json::json!({"query": "rust"}),
                },
                ToolCall {
                    id: "call_2".to_string(),
                    name: "fetch".to_string(),
                    arguments: serde_json::json!({"url": "https://example.com"}),
                },
            ]),
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        assert_eq!(encoded.role, UIRole::Assistant);
        assert_eq!(encoded.parts.len(), 3); // 1 text + 2 tool parts
        assert!(matches!(&encoded.parts[0], UIMessagePart::Text { text, .. } if text == "Let me search."));
        assert!(matches!(
            &encoded.parts[1],
            UIMessagePart::Tool { tool_call_id, tool_name, state: ToolState::OutputAvailable, .. }
            if tool_call_id == "call_1" && tool_name == "search"
        ));
        assert!(matches!(
            &encoded.parts[2],
            UIMessagePart::Tool { tool_call_id, tool_name, .. }
            if tool_call_id == "call_2" && tool_name == "fetch"
        ));
    }

    #[test]
    fn test_ai_sdk_history_encoder_tool_role_maps_to_assistant() {
        let msg = Message {
            id: Some("msg_3".to_string()),
            role: Role::Tool,
            content: "{\"result\":42}".to_string(),
            tool_calls: None,
            tool_call_id: Some("call_1".to_string()),
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        // In AI SDK, tool results are part of the assistant turn
        assert_eq!(encoded.role, UIRole::Assistant);
        assert_eq!(encoded.parts.len(), 1);
        assert!(matches!(&encoded.parts[0], UIMessagePart::Text { text, .. } if text == "{\"result\":42}"));
    }

    #[test]
    fn test_ai_sdk_history_encoder_empty_content_no_text_part() {
        let msg = Message {
            id: Some("msg_4".to_string()),
            role: Role::Assistant,
            content: String::new(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({}),
            }]),
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        // No Text part when content is empty
        assert_eq!(encoded.parts.len(), 1);
        assert!(matches!(&encoded.parts[0], UIMessagePart::Tool { .. }));
    }

    #[test]
    fn test_ai_sdk_history_encoder_no_id_defaults_empty() {
        let msg = Message {
            id: None,
            role: Role::User,
            content: "hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        assert_eq!(encoded.id, "");
    }

    #[test]
    fn test_ai_sdk_history_encoder_with_metadata() {
        let msg = Message {
            id: Some("msg_5".to_string()),
            role: Role::Assistant,
            content: "response".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: Some(MessageMetadata {
                run_id: Some("run_1".to_string()),
                step_index: Some(2),
            }),
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        assert!(encoded.metadata.is_some());
        let meta = encoded.metadata.unwrap();
        assert_eq!(meta["run_id"], "run_1");
        assert_eq!(meta["step_index"], 2);
    }

    #[test]
    fn test_ai_sdk_history_encoder_system_message() {
        let msg = Message {
            id: Some("msg_sys".to_string()),
            role: Role::System,
            content: "You are helpful.".to_string(),
            tool_calls: None,
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        assert_eq!(encoded.role, UIRole::System);
    }

    #[test]
    fn test_ai_sdk_encode_messages_batch() {
        let msgs = vec![
            Message::user("hello"),
            Message::assistant("world"),
        ];
        let encoded = AiSdkV6InputAdapter::encode_messages(msgs.iter());
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[0].role, UIRole::User);
        assert_eq!(encoded[1].role, UIRole::Assistant);
    }

    #[test]
    fn test_ai_sdk_history_encoder_serialization() {
        let msg = Message {
            id: Some("msg_1".to_string()),
            role: Role::Assistant,
            content: "Hello".to_string(),
            tool_calls: Some(vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "test"}),
            }]),
            tool_call_id: None,
            visibility: Visibility::default(),
            metadata: None,
        };
        let encoded = AiSdkV6InputAdapter::encode_message(&msg);
        let json = serde_json::to_string(&encoded).unwrap();
        // Verify camelCase field names
        assert!(json.contains("toolCallId"));
        assert!(json.contains("toolName"));
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("tool_name"));
    }

    #[test]
    fn test_convert_agui_messages_generates_id_when_missing() {
        let msg = AGUIMessage {
            id: None,
            role: MessageRole::User,
            content: "hello".into(),
            tool_call_id: None,
        };
        let result = convert_agui_messages(&[msg]);
        assert_eq!(result.len(), 1);
        assert!(result[0].id.is_some(), "must generate id when client omits it");
        let id = result[0].id.as_ref().unwrap();
        assert_eq!(id.len(), 36, "generated id should be a UUID");
    }

    #[test]
    fn test_convert_agui_messages_preserves_client_id() {
        let msg = AGUIMessage {
            id: Some("client-id-123".into()),
            role: MessageRole::User,
            content: "hello".into(),
            tool_call_id: None,
        };
        let result = convert_agui_messages(&[msg]);
        assert_eq!(result[0].id.as_deref(), Some("client-id-123"));
    }
}
