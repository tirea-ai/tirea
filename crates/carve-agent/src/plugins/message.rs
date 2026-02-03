//! Message management extension for Context.
//!
//! Provides methods for managing conversation messages:
//! - `append_message(msg)` - Add a message to the conversation
//! - `messages()` - Get all messages
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! async fn after_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, result: &ToolResult) {
//!     if result.is_success() {
//!         ctx.append_message(Message::assistant("Tool completed successfully"));
//!     }
//! }
//! ```

use crate::types::Message;
use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};

/// State path for messages.
pub const MESSAGE_STATE_PATH: &str = "messages";

/// Messages state stored in session state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct MessageState {
    /// Conversation messages.
    #[serde(default)]
    pub items: Vec<Message>,
}

/// Extension trait for message management on Context.
pub trait MessageContextExt {
    /// Append a message to the conversation.
    fn append_message(&self, msg: Message);

    /// Get all messages in the conversation.
    fn messages(&self) -> Vec<Message>;

    /// Get the number of messages.
    fn message_count(&self) -> usize;

    /// Get the last message if any.
    fn last_message(&self) -> Option<Message>;
}

impl MessageContextExt for Context<'_> {
    fn append_message(&self, msg: Message) {
        let state = self.state::<MessageState>(MESSAGE_STATE_PATH);
        state.items_push(msg);
    }

    fn messages(&self) -> Vec<Message> {
        let state = self.state::<MessageState>(MESSAGE_STATE_PATH);
        state.items().ok().unwrap_or_default()
    }

    fn message_count(&self) -> usize {
        self.messages().len()
    }

    fn last_message(&self) -> Option<Message> {
        self.messages().last().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_message_state_default() {
        let state = MessageState::default();
        assert!(state.items.is_empty());
    }

    #[test]
    fn test_message_state_serialization() {
        let mut state = MessageState::default();
        state.items.push(Message::user("Hello"));
        state.items.push(Message::assistant("Hi!"));

        let json = serde_json::to_string(&state).unwrap();
        let parsed: MessageState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].content, "Hello");
    }

    #[test]
    fn test_append_message() {
        let doc = json!({
            "messages": { "items": [] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.append_message(Message::user("Test message"));

        // Note: We can't directly verify the message was added because
        // Context collects ops that need to be applied to see the result.
        // This is tested via the patch mechanism.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_messages_empty() {
        let doc = json!({
            "messages": { "items": [] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.messages().is_empty());
        assert_eq!(ctx.message_count(), 0);
        assert!(ctx.last_message().is_none());
    }

    #[test]
    fn test_messages_with_existing() {
        let doc = json!({
            "messages": {
                "items": [
                    { "role": "user", "content": "Hello" },
                    { "role": "assistant", "content": "Hi!" }
                ]
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let messages = ctx.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(ctx.message_count(), 2);

        let last = ctx.last_message().unwrap();
        assert_eq!(last.content, "Hi!");
    }
}
