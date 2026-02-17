use super::{AGUIMessage, MessageRole};
use carve_protocol_contract::ProtocolHistoryEncoder;
use carve_agent_contract::{Message, Role};

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
