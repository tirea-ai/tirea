//! Thread model — persistent conversation state with message history.
//!
//! Unlike uncarve's Thread which uses JSON patches for state, awaken's Thread
//! is a simpler message container. State management goes through `StateStore`.

use super::message::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// Persisted thread state with messages.
///
/// Uses an owned builder pattern: `with_*` methods consume `self`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    /// Unique thread identifier.
    pub id: String,
    /// Owner/resource identifier (e.g., user_id, org_id).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// Parent thread identifier (links child -> parent for sub-agent lineage).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    /// Messages (Arc-wrapped for efficient cloning).
    pub messages: Vec<Arc<Message>>,
    /// Thread metadata.
    #[serde(default)]
    pub metadata: ThreadMetadata,
}

/// Thread metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadMetadata {
    /// Creation timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<u64>,
    /// Last update timestamp (unix millis).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// Persisted state cursor version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
    /// Custom metadata.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Thread {
    /// Create a new empty thread.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resource_id: None,
            parent_thread_id: None,
            messages: Vec::new(),
            metadata: ThreadMetadata::default(),
        }
    }

    /// Set the resource_id.
    #[must_use]
    pub fn with_resource_id(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Set the parent_thread_id.
    #[must_use]
    pub fn with_parent_thread_id(mut self, parent_thread_id: impl Into<String>) -> Self {
        self.parent_thread_id = Some(parent_thread_id.into());
        self
    }

    /// Add a message (Arc-wrapped for efficient cloning).
    #[must_use]
    pub fn with_message(mut self, msg: Message) -> Self {
        self.messages.push(Arc::new(msg));
        self
    }

    /// Add multiple messages.
    #[must_use]
    pub fn with_messages(mut self, msgs: impl IntoIterator<Item = Message>) -> Self {
        self.messages.extend(msgs.into_iter().map(Arc::new));
        self
    }

    /// Number of messages in the thread.
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::message::{Message, Role};

    #[test]
    fn thread_new_is_empty() {
        let t = Thread::new("t1");
        assert_eq!(t.id, "t1");
        assert_eq!(t.message_count(), 0);
        assert!(t.resource_id.is_none());
        assert!(t.parent_thread_id.is_none());
    }

    #[test]
    fn thread_builder_adds_messages() {
        let t = Thread::new("t1")
            .with_message(Message::user("hello"))
            .with_message(Message::assistant("hi"));
        assert_eq!(t.message_count(), 2);
        assert_eq!(t.messages[0].role, Role::User);
        assert_eq!(t.messages[1].role, Role::Assistant);
    }

    #[test]
    fn thread_with_messages_batch() {
        let msgs = vec![Message::system("sys"), Message::user("hi")];
        let t = Thread::new("t1").with_messages(msgs);
        assert_eq!(t.message_count(), 2);
    }

    #[test]
    fn thread_builder_chains() {
        let t = Thread::new("t1")
            .with_resource_id("user-123")
            .with_parent_thread_id("parent-t1");
        assert_eq!(t.resource_id.as_deref(), Some("user-123"));
        assert_eq!(t.parent_thread_id.as_deref(), Some("parent-t1"));
    }

    #[test]
    fn thread_serde_roundtrip() {
        let t = Thread::new("t1")
            .with_resource_id("org-1")
            .with_message(Message::user("test"));
        let json = serde_json::to_string(&t).unwrap();
        let parsed: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "t1");
        assert_eq!(parsed.resource_id.as_deref(), Some("org-1"));
        assert_eq!(parsed.message_count(), 1);
    }

    #[test]
    fn thread_messages_are_arc_wrapped() {
        let t = Thread::new("t1").with_message(Message::user("hi"));
        let cloned = t.clone();
        // Arc means both point to the same allocation
        assert!(Arc::ptr_eq(&t.messages[0], &cloned.messages[0]));
    }

    #[test]
    fn thread_metadata_extra_fields() {
        let json = r#"{"id":"t1","messages":[],"metadata":{"custom_key":"value"}}"#;
        let t: Thread = serde_json::from_str(json).unwrap();
        assert_eq!(t.metadata.extra.get("custom_key").unwrap(), "value");
    }
}
