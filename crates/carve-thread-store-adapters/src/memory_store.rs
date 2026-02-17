use async_trait::async_trait;
use carve_agent_contract::storage::{
    AgentChangeSet, AgentStateHead, AgentStateListPage, AgentStateListQuery, AgentStateReader,
    AgentStateStoreError, AgentStateSync, AgentStateWriter, Version,
};
use carve_agent_contract::{AgentState, Committed};

struct MemoryEntry {
    agent_state: AgentState,
    version: Version,
    deltas: Vec<AgentChangeSet>,
}

/// In-memory storage for testing and local development.
#[derive(Default)]
pub struct MemoryStore {
    entries: tokio::sync::RwLock<std::collections::HashMap<String, MemoryEntry>>,
}

impl MemoryStore {
    /// Create a new in-memory storage.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl AgentStateWriter for MemoryStore {
    async fn create(&self, thread: &AgentState) -> Result<Committed, AgentStateStoreError> {
        let mut entries = self.entries.write().await;
        if entries.contains_key(&thread.id) {
            return Err(AgentStateStoreError::AlreadyExists);
        }
        entries.insert(
            thread.id.clone(),
            MemoryEntry {
                agent_state: thread.clone(),
                version: 0,
                deltas: Vec::new(),
            },
        );
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        thread_id: &str,
        delta: &AgentChangeSet,
    ) -> Result<Committed, AgentStateStoreError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get_mut(thread_id)
            .ok_or_else(|| AgentStateStoreError::NotFound(thread_id.to_string()))?;

        delta.apply_to(&mut entry.agent_state);
        entry.version += 1;
        entry.deltas.push(delta.clone());
        Ok(Committed {
            version: entry.version,
        })
    }

    async fn delete(&self, thread_id: &str) -> Result<(), AgentStateStoreError> {
        let mut entries = self.entries.write().await;
        entries.remove(thread_id);
        Ok(())
    }

    async fn save(&self, thread: &AgentState) -> Result<(), AgentStateStoreError> {
        let mut entries = self.entries.write().await;
        let version = entries.get(&thread.id).map_or(0, |e| e.version + 1);
        entries.insert(
            thread.id.clone(),
            MemoryEntry {
                agent_state: thread.clone(),
                version,
                deltas: Vec::new(),
            },
        );
        Ok(())
    }
}

#[async_trait]
impl AgentStateReader for MemoryStore {
    async fn load(&self, thread_id: &str) -> Result<Option<AgentStateHead>, AgentStateStoreError> {
        let entries = self.entries.read().await;
        Ok(entries.get(thread_id).map(|e| AgentStateHead {
            agent_state: e.agent_state.clone(),
            version: e.version,
        }))
    }

    async fn list_agent_states(
        &self,
        query: &AgentStateListQuery,
    ) -> Result<AgentStateListPage, AgentStateStoreError> {
        let entries = self.entries.read().await;
        let mut ids: Vec<String> = entries
            .iter()
            .filter(|(_, e)| {
                if let Some(ref rid) = query.resource_id {
                    e.agent_state.resource_id.as_deref() == Some(rid.as_str())
                } else {
                    true
                }
            })
            .filter(|(_, e)| {
                if let Some(ref pid) = query.parent_thread_id {
                    e.agent_state.parent_thread_id.as_deref() == Some(pid.as_str())
                } else {
                    true
                }
            })
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        let total = ids.len();
        let limit = query.limit.clamp(1, 200);
        let offset = query.offset.min(total);
        let end = (offset + limit + 1).min(total);
        let slice = &ids[offset..end];
        let has_more = slice.len() > limit;
        let items: Vec<String> = slice.iter().take(limit).cloned().collect();
        Ok(AgentStateListPage {
            items,
            total,
            has_more,
        })
    }
}

#[async_trait]
impl AgentStateSync for MemoryStore {
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<AgentChangeSet>, AgentStateStoreError> {
        let entries = self.entries.read().await;
        let entry = entries
            .get(thread_id)
            .ok_or_else(|| AgentStateStoreError::NotFound(thread_id.to_string()))?;
        // Deltas are 1-indexed: delta[0] produced version 1, delta[1] produced version 2, etc.
        let skip = after_version as usize;
        Ok(entry.deltas[skip..].to_vec())
    }
}
