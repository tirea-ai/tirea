//! Async message inbox operations.

use awaken_contract::contract::storage::{MailboxEntry, MailboxStore, StorageError};
use serde_json::Value;

/// Push a message to a thread's mailbox.
pub async fn push_message(
    store: &dyn MailboxStore,
    mailbox_id: &str,
    payload: Value,
) -> Result<MailboxEntry, StorageError> {
    let entry = MailboxEntry {
        entry_id: uuid::Uuid::now_v7().to_string(),
        mailbox_id: mailbox_id.to_string(),
        payload,
        created_at: now_millis(),
    };
    store.push_message(&entry).await?;
    Ok(entry)
}

/// Peek at messages in a mailbox without removing them.
pub async fn peek_messages(
    store: &dyn MailboxStore,
    mailbox_id: &str,
    limit: usize,
) -> Result<Vec<MailboxEntry>, StorageError> {
    store.peek_messages(mailbox_id, limit).await
}

/// Pop messages from a mailbox (oldest first), removing them.
pub async fn pop_messages(
    store: &dyn MailboxStore,
    mailbox_id: &str,
    limit: usize,
) -> Result<Vec<MailboxEntry>, StorageError> {
    store.pop_messages(mailbox_id, limit).await
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Simple in-memory mailbox store for testing.
    #[derive(Default)]
    struct MockMailboxStore {
        entries: std::sync::RwLock<BTreeMap<String, Vec<MailboxEntry>>>,
    }

    #[async_trait::async_trait]
    impl MailboxStore for MockMailboxStore {
        async fn push_message(&self, entry: &MailboxEntry) -> Result<(), StorageError> {
            self.entries
                .write()
                .unwrap()
                .entry(entry.mailbox_id.clone())
                .or_default()
                .push(entry.clone());
            Ok(())
        }

        async fn pop_messages(
            &self,
            mailbox_id: &str,
            limit: usize,
        ) -> Result<Vec<MailboxEntry>, StorageError> {
            let mut guard = self.entries.write().unwrap();
            let queue = match guard.get_mut(mailbox_id) {
                Some(q) => q,
                None => return Ok(Vec::new()),
            };
            let drain_count = limit.min(queue.len());
            Ok(queue.drain(..drain_count).collect())
        }

        async fn peek_messages(
            &self,
            mailbox_id: &str,
            limit: usize,
        ) -> Result<Vec<MailboxEntry>, StorageError> {
            let guard = self.entries.read().unwrap();
            Ok(guard
                .get(mailbox_id)
                .map(|q| q.iter().take(limit).cloned().collect())
                .unwrap_or_default())
        }
    }

    #[tokio::test]
    async fn push_and_peek_messages() {
        let store = MockMailboxStore::default();
        let entry = push_message(&store, "inbox-1", serde_json::json!({"text": "hello"}))
            .await
            .unwrap();
        assert_eq!(entry.mailbox_id, "inbox-1");
        assert!(!entry.entry_id.is_empty());

        let peeked = peek_messages(&store, "inbox-1", 10).await.unwrap();
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0].payload["text"], "hello");
    }

    #[tokio::test]
    async fn pop_removes_messages() {
        let store = MockMailboxStore::default();
        push_message(&store, "inbox-1", serde_json::json!(1))
            .await
            .unwrap();
        push_message(&store, "inbox-1", serde_json::json!(2))
            .await
            .unwrap();

        let popped = pop_messages(&store, "inbox-1", 1).await.unwrap();
        assert_eq!(popped.len(), 1);
        assert_eq!(popped[0].payload, serde_json::json!(1));

        let remaining = peek_messages(&store, "inbox-1", 10).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].payload, serde_json::json!(2));
    }

    #[tokio::test]
    async fn pop_empty_mailbox() {
        let store = MockMailboxStore::default();
        let popped = pop_messages(&store, "empty", 10).await.unwrap();
        assert!(popped.is_empty());
    }

    #[tokio::test]
    async fn peek_empty_mailbox() {
        let store = MockMailboxStore::default();
        let peeked = peek_messages(&store, "empty", 10).await.unwrap();
        assert!(peeked.is_empty());
    }

    #[tokio::test]
    async fn push_generates_uuid_v7_entry_id() {
        let store = MockMailboxStore::default();
        let entry = push_message(&store, "inbox-1", serde_json::json!(null))
            .await
            .unwrap();
        assert_eq!(entry.entry_id.len(), 36);
        assert_eq!(&entry.entry_id[14..15], "7");
    }
}
