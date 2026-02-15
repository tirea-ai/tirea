use super::{
    AiSdkEncoder, StreamState, ToolState, UIMessage, UIMessagePart, UIRole, UIStreamEvent,
    AI_SDK_VERSION,
};
use crate::protocol::{ProtocolHistoryEncoder, ProtocolInputAdapter, ProtocolOutputEncoder};
use crate::{AgentEvent, Message, Role};
use serde::Deserialize;
use serde_json::json;

const RUN_INFO_EVENT_NAME: &str = "run-info";

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
            state: None,
            messages: vec![Message::user(request.input)],
            runtime: std::collections::HashMap::new(),
        }
    }
}

impl ProtocolHistoryEncoder for AiSdkV6InputAdapter {
    type HistoryMessage = UIMessage;

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
            metadata: msg
                .metadata
                .as_ref()
                .and_then(|m| serde_json::to_value(m).ok()),
            parts,
        }
    }
}

pub struct AiSdkV6ProtocolEncoder {
    inner: AiSdkEncoder,
    run_info: Option<UIStreamEvent>,
}

impl AiSdkV6ProtocolEncoder {
    pub fn new(run_id: String, thread_id: Option<String>) -> Self {
        let run_info = thread_id.map(|thread_id| {
            UIStreamEvent::data(
                RUN_INFO_EVENT_NAME,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ProtocolHistoryEncoder;
    use crate::types::{MessageMetadata, ToolCall};
    use crate::{Message, Role, Visibility};

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
        assert_eq!(encoded.parts.len(), 3);
        assert!(matches!(
            &encoded.parts[0],
            UIMessagePart::Text { text, .. } if text == "Let me search."
        ));
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
        assert_eq!(encoded.role, UIRole::Assistant);
        assert_eq!(encoded.parts.len(), 1);
        assert!(matches!(
            &encoded.parts[0],
            UIMessagePart::Text { text, .. } if text == "{\"result\":42}"
        ));
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
        let msgs = vec![Message::user("hello"), Message::assistant("world")];
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
        assert!(json.contains("toolCallId"));
        assert!(json.contains("toolName"));
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("tool_name"));
    }
}
