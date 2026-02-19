use crate::thread::ThreadChangeSet;
use crate::thread::Thread;
use crate::thread::Version;
use async_trait::async_trait;

use super::{
    paginate_in_memory, AgentStateHead, AgentStateListPage, AgentStateListQuery,
    AgentStateStoreError, Committed, MessagePage, MessageQuery, VersionPrecondition,
};

#[async_trait]
pub trait AgentStateReader: Send + Sync {
    /// Load an Thread and its current version.
    async fn load(&self, thread_id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError>;

    /// Load an Thread without version info. Convenience wrapper.
    async fn load_agent_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<Thread>, AgentStateStoreError> {
        Ok(self.load(thread_id).await?.map(|h| h.agent_state))
    }

    /// Load paginated messages for an Thread.
    async fn load_messages(
        &self,
        thread_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, AgentStateStoreError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| AgentStateStoreError::NotFound(thread_id.to_string()))?;
        Ok(paginate_in_memory(&head.agent_state.messages, query))
    }

    /// List Thread ids.
    async fn list_agent_states(
        &self,
        query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError>;

    /// List all Thread ids with default paging.
    async fn list(&self) -> Result<Vec<String>, AgentStateStoreError> {
        let page = self
            .list_agent_states(&AgentStateListQuery {
                offset: 0,
                limit: 200,
                resource_id: None,
                parent_thread_id: None,
            })
            .await?;
        Ok(page.items)
    }

    /// List Thread ids with explicit query.
    async fn list_paginated(
        &self,
        query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError> {
        self.list_agent_states(query).await
    }

    /// Return total message count.
    async fn message_count(&self, thread_id: &str) -> Result<usize, AgentStateStoreError> {
        let head = self
            .load(thread_id)
            .await?
            .ok_or_else(|| AgentStateStoreError::NotFound(thread_id.to_string()))?;
        Ok(head.agent_state.messages.len())
    }
}

#[async_trait]
pub trait AgentStateWriter: AgentStateReader {
    /// Create a new Thread.
    async fn create(&self, agent_state: &Thread) -> Result<Committed, AgentStateStoreError>;

    /// Append an ThreadChangeSet to an existing Thread.
    async fn append(
        &self,
        thread_id: &str,
        delta: &ThreadChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<Committed, AgentStateStoreError>;

    /// Delete an Thread.
    async fn delete(&self, thread_id: &str) -> Result<(), AgentStateStoreError>;

    /// Upsert or replace the current persisted Thread.
    ///
    /// Implementations must provide atomic semantics suitable for their backend.
    async fn save(&self, agent_state: &Thread) -> Result<(), AgentStateStoreError>;
}

#[async_trait]
pub trait AgentStateSync: AgentStateWriter {
    /// Load delta list appended after a specific version.
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<ThreadChangeSet>, AgentStateStoreError>;
}

/// Full storage trait.
pub trait AgentStateStore: AgentStateWriter {}

impl<T: AgentStateWriter + ?Sized> AgentStateStore for T {}
