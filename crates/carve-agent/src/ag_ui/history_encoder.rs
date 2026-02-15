use super::{AGUIMessage, MessageRole};
use crate::protocol::ProtocolHistoryEncoder;
use crate::{Message, Role};

pub struct AgUiHistoryEncoder;

impl ProtocolHistoryEncoder for AgUiHistoryEncoder {
    type HistoryMessage = AGUIMessage;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ag_ui::request::convert_agui_messages;
    use crate::ag_ui::AGUIMessage;
    use crate::protocol::ProtocolHistoryEncoder;
    use crate::types::ToolCall;
    use crate::{Message, Visibility};

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
        let encoded = AgUiHistoryEncoder::encode_message(&msg);
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
        let encoded = AgUiHistoryEncoder::encode_message(&msg);
        assert_eq!(encoded.role, MessageRole::Assistant);
        assert_eq!(encoded.content, "hi there");
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
        let encoded = AgUiHistoryEncoder::encode_message(&msg);
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
        let encoded = AgUiHistoryEncoder::encode_message(&msg);
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
        let encoded = AgUiHistoryEncoder::encode_message(&msg);
        let json = serde_json::to_string(&encoded).unwrap();
        assert!(json.contains("toolCallId"));
        assert!(!json.contains("tool_call_id"));
    }

    #[test]
    fn test_agui_roundtrip_preserves_content() {
        let original = AGUIMessage {
            id: Some("msg_1".to_string()),
            role: MessageRole::User,
            content: "hello world".to_string(),
            tool_call_id: None,
        };
        let internal = convert_agui_messages(&[original.clone()]);
        assert_eq!(internal.len(), 1);
        let roundtripped = AgUiHistoryEncoder::encode_message(&internal[0]);
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
        let roundtripped = AgUiHistoryEncoder::encode_message(&internal[0]);
        assert_eq!(roundtripped.role, MessageRole::Tool);
        assert_eq!(roundtripped.tool_call_id, Some("call_99".to_string()));
    }

    #[test]
    fn test_agui_encode_messages_batch() {
        let msgs = vec![Message::user("hello"), Message::assistant("world")];
        let encoded = AgUiHistoryEncoder::encode_messages(msgs.iter());
        assert_eq!(encoded.len(), 2);
        assert_eq!(encoded[0].role, MessageRole::User);
        assert_eq!(encoded[1].role, MessageRole::Assistant);
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
        assert!(
            result[0].id.is_some(),
            "must generate id when client omits it"
        );
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
