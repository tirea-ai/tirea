use crate::change::AgentChangeSet;
use crate::conversation::AgentState;
use async_trait::async_trait;

use super::{
    AgentStateHead, Committed, MessagePage, MessageQuery, AgentStateListPage, AgentStateListQuery, AgentStateStoreError,
    Version, paginate_in_memory,
};

#[async_trait]
pub trait AgentStateReader: Send + Sync {
    /// Load an AgentState and its current version.
    async fn load(&self, thread_id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError>;

    /// Load an AgentState without version info. Convenience wrapper.
    async fn load_agent_state(
        &self,
        thread_id: &str,
    ) -> Result<Option<AgentState>, AgentStateStoreError> {
        Ok(self.load(thread_id).await?.map(|h| h.agent_state))
    }

    /// Load paginated messages for an AgentState.
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

    /// List AgentState ids.
    async fn list_agent_states(
        &self,
        query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError>;

    /// List all AgentState ids with default paging.
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

    /// List AgentState ids with explicit query.
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
    /// Create a new AgentState.
    async fn create(&self, agent_state: &AgentState) -> Result<Committed, AgentStateStoreError>;

    /// Append an AgentChangeSet to an existing AgentState.
    async fn append(
        &self,
        thread_id: &str,
        delta: &AgentChangeSet,
    ) -> Result<Committed, AgentStateStoreError>;

    /// Delete an AgentState.
    async fn delete(&self, thread_id: &str) -> Result<(), AgentStateStoreError>;

    /// Upsert helper.
    async fn save(&self, agent_state: &AgentState) -> Result<(), AgentStateStoreError> {
        let _ = self.delete(&agent_state.id).await;
        self.create(agent_state).await?;
        Ok(())
    }
}

#[async_trait]
pub trait AgentStateSync: AgentStateWriter {
    /// Load delta list appended after a specific version.
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<AgentChangeSet>, AgentStateStoreError>;
}

/// Full storage trait.
pub trait AgentStateStore: AgentStateWriter {}

impl<T: AgentStateWriter + ?Sized> AgentStateStore for T {}
