use crate::change::AgentChangeSet;
use crate::conversation::AgentState;
use async_trait::async_trait;

use super::{
    AgentStateHead, Committed, MessagePage, MessageQuery, ThreadListPage, ThreadListQuery, ThreadStoreError,
    Version, paginate_in_memory,
};

#[async_trait]
pub trait ThreadReader: Send + Sync {
    /// Load an AgentState and its current version.
    async fn load(&self, agent_state_id: &str) -> Result<Option<AgentStateHead>, ThreadStoreError>;

    /// Load an AgentState without version info. Convenience wrapper.
    async fn load_thread(
        &self,
        agent_state_id: &str,
    ) -> Result<Option<AgentState>, ThreadStoreError> {
        Ok(self.load(agent_state_id).await?.map(|h| h.agent_state))
    }

    /// Load paginated messages for an AgentState.
    async fn load_messages(
        &self,
        agent_state_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, ThreadStoreError> {
        let head = self
            .load(agent_state_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(agent_state_id.to_string()))?;
        Ok(paginate_in_memory(&head.agent_state.messages, query))
    }

    /// List AgentState ids.
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, ThreadStoreError>;

    /// List all AgentState ids with default paging.
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

    /// List AgentState ids with explicit query.
    async fn list_paginated(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, ThreadStoreError> {
        self.list_threads(query).await
    }

    /// Return total message count.
    async fn message_count(&self, agent_state_id: &str) -> Result<usize, ThreadStoreError> {
        let head = self
            .load(agent_state_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(agent_state_id.to_string()))?;
        Ok(head.agent_state.messages.len())
    }
}

#[async_trait]
pub trait ThreadWriter: ThreadReader {
    /// Create a new AgentState.
    async fn create(&self, agent_state: &AgentState) -> Result<Committed, ThreadStoreError>;

    /// Append an AgentChangeSet to an existing AgentState.
    async fn append(
        &self,
        agent_state_id: &str,
        delta: &AgentChangeSet,
    ) -> Result<Committed, ThreadStoreError>;

    /// Delete an AgentState.
    async fn delete(&self, agent_state_id: &str) -> Result<(), ThreadStoreError>;

    /// Upsert helper.
    async fn save(&self, agent_state: &AgentState) -> Result<(), ThreadStoreError> {
        let _ = self.delete(&agent_state.id).await;
        self.create(agent_state).await?;
        Ok(())
    }
}

#[async_trait]
pub trait ThreadSync: ThreadWriter {
    /// Load delta list appended after a specific version.
    async fn load_deltas(
        &self,
        agent_state_id: &str,
        after_version: Version,
    ) -> Result<Vec<AgentChangeSet>, ThreadStoreError>;
}

/// Full storage trait.
pub trait ThreadStore: ThreadWriter {}

impl<T: ThreadWriter + ?Sized> ThreadStore for T {}
