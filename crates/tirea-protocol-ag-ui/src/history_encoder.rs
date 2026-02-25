pub struct AgUiHistoryEncoder;

impl AgUiHistoryEncoder {
    pub fn encode_message(msg: &tirea_contract::Message) -> crate::Message {
        crate::Message {
            id: msg.id.clone(),
            role: match msg.role {
                tirea_contract::Role::System => crate::Role::System,
                tirea_contract::Role::User => crate::Role::User,
                tirea_contract::Role::Assistant => crate::Role::Assistant,
                tirea_contract::Role::Tool => crate::Role::Tool,
            },
            content: msg.content.clone(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}
