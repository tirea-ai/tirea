use super::*;

struct MemoryEntry {
    thread: Thread,
    version: Version,
    deltas: Vec<ThreadDelta>,
}

/// In-memory storage for testing.
#[derive(Default)]
pub struct MemoryStorage {
    entries: tokio::sync::RwLock<std::collections::HashMap<String, MemoryEntry>>,
}

impl MemoryStorage {
    /// Create a new in-memory storage.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ThreadStore for MemoryStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError> {
        let mut entries = self.entries.write().await;
        if entries.contains_key(&thread.id) {
            return Err(StorageError::AlreadyExists);
        }
        entries.insert(
            thread.id.clone(),
            MemoryEntry {
                thread: thread.clone(),
                version: 0,
                deltas: Vec::new(),
            },
        );
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        thread_id: &str,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get_mut(thread_id)
            .ok_or_else(|| StorageError::NotFound(thread_id.to_string()))?;

        delta.apply_to(&mut entry.thread);
        entry.version += 1;
        entry.deltas.push(delta.clone());
        Ok(Committed {
            version: entry.version,
        })
    }

    async fn load(&self, thread_id: &str) -> Result<Option<ThreadHead>, StorageError> {
        let entries = self.entries.read().await;
        Ok(entries.get(thread_id).map(|e| ThreadHead {
            thread: e.thread.clone(),
            version: e.version,
        }))
    }

    async fn delete(&self, thread_id: &str) -> Result<(), StorageError> {
        let mut entries = self.entries.write().await;
        entries.remove(thread_id);
        Ok(())
    }

    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        let mut entries = self.entries.write().await;
        let version = entries.get(&thread.id).map_or(0, |e| e.version + 1);
        entries.insert(
            thread.id.clone(),
            MemoryEntry {
                thread: thread.clone(),
                version,
                deltas: Vec::new(),
            },
        );
        Ok(())
    }
}

#[async_trait]
impl ThreadQuery for MemoryStorage {
    async fn list_threads(&self, query: &ThreadListQuery) -> Result<ThreadListPage, StorageError> {
        let entries = self.entries.read().await;
        let mut ids: Vec<String> = entries
            .iter()
            .filter(|(_, e)| {
                if let Some(ref rid) = query.resource_id {
                    e.thread.resource_id.as_deref() == Some(rid.as_str())
                } else {
                    true
                }
            })
            .filter(|(_, e)| {
                if let Some(ref pid) = query.parent_thread_id {
                    e.thread.parent_thread_id.as_deref() == Some(pid.as_str())
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
        Ok(ThreadListPage {
            items,
            total,
            has_more,
        })
    }
}

#[async_trait]
impl ThreadSync for MemoryStorage {
    async fn load_deltas(
        &self,
        thread_id: &str,
        after_version: Version,
    ) -> Result<Vec<ThreadDelta>, StorageError> {
        let entries = self.entries.read().await;
        let entry = entries
            .get(thread_id)
            .ok_or_else(|| StorageError::NotFound(thread_id.to_string()))?;
        // Deltas are 1-indexed: delta[0] produced version 1, delta[1] produced version 2, etc.
        let skip = after_version as usize;
        Ok(entry.deltas[skip..].to_vec())
    }
}
