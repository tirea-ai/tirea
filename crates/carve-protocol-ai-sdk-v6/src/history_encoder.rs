use super::{StreamState, ToolState, UIMessage, UIMessagePart, UIRole};
use carve_protocol_contract::ProtocolHistoryEncoder;
use carve_agent_contract::{Message, Role};
use tracing::warn;

pub struct AiSdkV6HistoryEncoder;

impl ProtocolHistoryEncoder for AiSdkV6HistoryEncoder {
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

        let metadata = match msg.metadata.as_ref() {
            Some(metadata) => match serde_json::to_value(metadata) {
                Ok(value) => Some(value),
                Err(err) => {
                    warn!(
                        error = %err,
                        message_id = msg.id.as_deref().unwrap_or_default(),
                        "failed to serialize message metadata for AI SDK history"
                    );
                    None
                }
            },
            None => None,
        };

        UIMessage {
            id: msg.id.clone().unwrap_or_default(),
            role,
            metadata,
            parts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_protocol_contract::ProtocolHistoryEncoder;
    use carve_agent_contract::{Message, MessageMetadata, ToolCall, Visibility};

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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
        assert_eq!(encoded.id, "msg_1");
        assert_eq!(encoded.role, UIRole::User);
        assert_eq!(encoded.parts.len(), 1);
        assert!(matches!(
            &encoded.parts[0],
            UIMessagePart::Text {
                text,
                state: Some(StreamState::Done)
            } if text == "hello"
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
        assert_eq!(encoded.role, UIRole::System);
    }

    #[test]
    fn test_ai_sdk_encode_messages_batch() {
        let msgs = vec![Message::user("hello"), Message::assistant("world")];
        let encoded = AiSdkV6HistoryEncoder::encode_messages(msgs.iter());
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
        let encoded = AiSdkV6HistoryEncoder::encode_message(&msg);
        let json = serde_json::to_string(&encoded).unwrap();
        assert!(json.contains("toolCallId"));
        assert!(json.contains("toolName"));
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("tool_name"));
    }
}
