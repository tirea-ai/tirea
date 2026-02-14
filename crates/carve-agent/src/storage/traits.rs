use super::*;

#[async_trait]
pub trait ThreadStore: Send + Sync {
    /// Create a new thread. Returns `AlreadyExists` if the id is taken.
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError>;

    /// Append a delta to an existing thread.
    ///
    /// Version is managed internally by the backend — callers do not need to
    /// track it. Each successful append atomically increments the version.
    async fn append(&self, thread_id: &str, delta: &ThreadDelta)
        -> Result<Committed, StorageError>;

    /// Load a thread and its current version.
    async fn load(&self, thread_id: &str) -> Result<Option<ThreadHead>, StorageError>;

    /// Delete a thread.
    async fn delete(&self, thread_id: &str) -> Result<(), StorageError>;

    /// Upsert a thread (delete + create). Convenience wrapper.
    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        let _ = self.delete(&thread.id).await;
        self.create(thread).await?;
        Ok(())
    }

    /// Load a thread without version info. Convenience wrapper.
    async fn load_thread(&self, thread_id: &str) -> Result<Option<Thread>, StorageError> {
        Ok(self.load(thread_id).await?.map(|h| h.thread))
    }
}

/// Query operations — default impls based on `ThreadStore::load()`.
///
/// Database backends should override with efficient queries.
#[async_trait]
pub trait ThreadQuery: ThreadStore {
    /// Load a paginated slice of messages for a thread.
    async fn load_messages(
        &self,
        thread_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, StorageError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| StorageError::NotFound(thread_id.to_string()))?;
        Ok(paginate_in_memory(&head.thread.messages, query))
    }

    /// List threads with pagination.
    async fn list_threads(&self, query: &ThreadListQuery) -> Result<ThreadListPage, StorageError>;

    /// List all thread IDs. Convenience wrapper.
    async fn list(&self) -> Result<Vec<String>, StorageError> {
        let page = self
            .list_threads(&ThreadListQuery {
                offset: 0,
                limit: 200,
                resource_id: None,
                parent_thread_id: None,
            })
            .await?;
        Ok(page.items)
    }

    /// List threads with pagination. Convenience alias for `list_threads`.
    async fn list_paginated(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
        self.list_threads(query).await
    }

    /// Get total message count for a thread. Convenience wrapper.
    async fn message_count(&self, thread_id: &str) -> Result<usize, StorageError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| StorageError::NotFound(thread_id.to_string()))?;
        Ok(head.thread.messages.len())
    }
}

/// Sync operations — for backends with delta replay capability.
#[async_trait]
pub trait ThreadSync: ThreadStore {
    /// Load deltas appended after `after_version`.
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<ThreadDelta>, StorageError>;
}
