use super::*;

#[async_trait]
pub trait ThreadReader: Send + Sync {
    /// Load a thread and its current version.
    async fn load(&self, thread_id: &str) -> Result<Option<AgentStateHead>, ThreadStoreError>;

    /// Load a thread without version info. Convenience wrapper.
    async fn load_thread(&self, thread_id: &str) -> Result<Option<AgentState>, ThreadStoreError> {
        Ok(self.load(thread_id).await?.map(|h| h.thread))
    }

    /// Load a paginated slice of messages for a thread.
    async fn load_messages(
        &self,
        thread_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, ThreadStoreError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_string()))?;
        Ok(paginate_in_memory(&head.thread.messages, query))
    }

    /// List threads with pagination.
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, ThreadStoreError>;

    /// List all thread IDs. Convenience wrapper.
    async fn list(&self) -> Result<Vec<String>, ThreadStoreError> {
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
    ) -> Result<ThreadListPage, ThreadStoreError> {
        self.list_threads(query).await
    }

    /// Get total message count for a thread. Convenience wrapper.
    async fn message_count(&self, thread_id: &str) -> Result<usize, ThreadStoreError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_string()))?;
        Ok(head.thread.messages.len())
    }
}

/// Write operations for thread persistence.
#[async_trait]
pub trait ThreadWriter: ThreadReader {
    /// Create a new thread. Returns `AlreadyExists` if the id is taken.
    async fn create(&self, thread: &AgentState) -> Result<Committed, ThreadStoreError>;

    /// Append a change set to an existing thread.
    ///
    /// Version is managed internally by the backend — callers do not need to
    /// track it. Each successful append atomically increments the version.
    async fn append(
        &self,
        thread_id: &str,
        delta: &AgentChangeSet,
    ) -> Result<Committed, ThreadStoreError>;

    /// Delete a thread.
    async fn delete(&self, thread_id: &str) -> Result<(), ThreadStoreError>;

    /// Upsert a thread (delete + create). Convenience wrapper.
    async fn save(&self, thread: &AgentState) -> Result<(), ThreadStoreError> {
        let _ = self.delete(&thread.id).await;
        self.create(thread).await?;
        Ok(())
    }
}

/// Sync operations — for backends with delta replay capability.
#[async_trait]
pub trait ThreadSync: ThreadWriter {
    /// Load change sets appended after `after_version`.
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<AgentChangeSet>, ThreadStoreError>;
}

/// Full thread store capability (read + write).
pub trait ThreadStore: ThreadWriter {}

impl<T: ThreadWriter + ?Sized> ThreadStore for T {}
