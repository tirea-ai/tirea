//! Thread CRUD operations.

use awaken_contract::contract::storage::{StorageError, ThreadStore};
use awaken_contract::thread::Thread;

/// Create a new thread.
pub async fn create_thread(
    store: &dyn ThreadStore,
    title: Option<String>,
) -> Result<Thread, StorageError> {
    let mut thread = Thread::new();
    if let Some(t) = title {
        thread.metadata.title = Some(t);
    }
    thread.metadata.created_at = Some(now_ms());
    thread.metadata.updated_at = Some(now_ms());
    store.save_thread(&thread).await?;
    Ok(thread)
}

/// Get a thread by ID.
pub async fn get_thread(
    store: &dyn ThreadStore,
    thread_id: &str,
) -> Result<Option<Thread>, StorageError> {
    store.load_thread(thread_id).await
}

/// List thread IDs with pagination.
pub async fn list_threads(
    store: &dyn ThreadStore,
    offset: usize,
    limit: usize,
) -> Result<Vec<String>, StorageError> {
    store.list_threads(offset, limit).await
}

/// Update thread title.
pub async fn update_thread_title(
    store: &dyn ThreadStore,
    thread_id: &str,
    title: String,
) -> Result<Thread, StorageError> {
    let mut thread = store
        .load_thread(thread_id)
        .await?
        .ok_or_else(|| StorageError::NotFound(thread_id.to_string()))?;
    thread.metadata.title = Some(title);
    thread.metadata.updated_at = Some(now_ms());
    store.save_thread(&thread).await?;
    Ok(thread)
}

/// Apply a partial update to a thread's metadata.
pub async fn patch_thread(
    store: &(dyn ThreadStore + Sync),
    thread_id: &str,
    title: Option<String>,
    custom: Option<std::collections::HashMap<String, serde_json::Value>>,
) -> Result<Thread, StorageError> {
    let mut thread = store
        .load_thread(thread_id)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("thread {thread_id}")))?;

    if let Some(t) = title {
        thread.metadata.title = Some(t);
    }
    if let Some(c) = custom {
        for (k, v) in c {
            thread.metadata.custom.insert(k, v);
        }
    }
    thread.metadata.updated_at = Some(now_ms());
    store.save_thread(&thread).await?;
    Ok(thread)
}

/// Delete a thread after verifying it exists.
pub async fn delete_thread(
    store: &(dyn ThreadStore + Sync),
    thread_id: &str,
) -> Result<bool, StorageError> {
    store
        .load_thread(thread_id)
        .await?
        .ok_or_else(|| StorageError::NotFound(format!("thread {thread_id}")))?;
    store.delete_thread(thread_id).await?;
    Ok(true)
}

/// List thread summaries (id, title, updated_at) with pagination.
pub async fn list_summaries(
    store: &(dyn ThreadStore + Sync),
    offset: usize,
    limit: usize,
) -> Result<Vec<serde_json::Value>, StorageError> {
    let ids = store.list_threads(offset, limit).await?;
    let mut summaries = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(thread) = store.load_thread(&id).await? {
            summaries.push(serde_json::json!({
                "id": thread.id,
                "title": thread.metadata.title,
                "updated_at": thread.metadata.updated_at,
            }));
        }
    }
    Ok(summaries)
}

use awaken_contract::now_ms;

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::storage::ThreadStore;

    /// Simple in-memory thread store for testing.
    #[derive(Default)]
    struct MockThreadStore {
        threads: std::sync::RwLock<std::collections::HashMap<String, Thread>>,
        messages: std::sync::RwLock<
            std::collections::HashMap<String, Vec<awaken_contract::contract::message::Message>>,
        >,
    }

    #[async_trait::async_trait]
    impl ThreadStore for MockThreadStore {
        async fn load_thread(&self, thread_id: &str) -> Result<Option<Thread>, StorageError> {
            Ok(self.threads.read().unwrap().get(thread_id).cloned())
        }

        async fn save_thread(&self, thread: &Thread) -> Result<(), StorageError> {
            self.threads
                .write()
                .unwrap()
                .insert(thread.id.clone(), thread.clone());
            Ok(())
        }

        async fn delete_thread(&self, thread_id: &str) -> Result<(), StorageError> {
            self.threads.write().unwrap().remove(thread_id);
            self.messages.write().unwrap().remove(thread_id);
            Ok(())
        }

        async fn list_threads(
            &self,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<String>, StorageError> {
            let guard = self.threads.read().unwrap();
            let mut ids: Vec<String> = guard.keys().cloned().collect();
            ids.sort();
            Ok(ids.into_iter().skip(offset).take(limit).collect())
        }

        async fn load_messages(
            &self,
            thread_id: &str,
        ) -> Result<Option<Vec<awaken_contract::contract::message::Message>>, StorageError>
        {
            Ok(self.messages.read().unwrap().get(thread_id).cloned())
        }

        async fn save_messages(
            &self,
            thread_id: &str,
            messages: &[awaken_contract::contract::message::Message],
        ) -> Result<(), StorageError> {
            self.messages
                .write()
                .unwrap()
                .insert(thread_id.to_owned(), messages.to_vec());
            Ok(())
        }

        async fn delete_messages(&self, thread_id: &str) -> Result<(), StorageError> {
            if !self.threads.read().unwrap().contains_key(thread_id) {
                return Err(StorageError::NotFound(thread_id.to_owned()));
            }
            self.messages.write().unwrap().remove(thread_id);
            Ok(())
        }

        async fn update_thread_metadata(
            &self,
            id: &str,
            metadata: awaken_contract::thread::ThreadMetadata,
        ) -> Result<(), StorageError> {
            let mut guard = self.threads.write().unwrap();
            let thread = guard
                .get_mut(id)
                .ok_or_else(|| StorageError::NotFound(id.to_owned()))?;
            thread.metadata = metadata;
            Ok(())
        }
    }

    #[tokio::test]
    async fn create_thread_assigns_uuid_v7_id() {
        let store = MockThreadStore::default();
        let thread = create_thread(&store, Some("Test".to_string()))
            .await
            .unwrap();
        assert_eq!(thread.id.len(), 36);
        assert_eq!(&thread.id[14..15], "7");
        assert_eq!(thread.metadata.title.as_deref(), Some("Test"));
        assert!(thread.metadata.created_at.is_some());
    }

    #[tokio::test]
    async fn create_thread_without_title() {
        let store = MockThreadStore::default();
        let thread = create_thread(&store, None).await.unwrap();
        assert!(thread.metadata.title.is_none());
    }

    #[tokio::test]
    async fn get_thread_returns_none_for_missing() {
        let store = MockThreadStore::default();
        let result = get_thread(&store, "missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_thread_returns_existing() {
        let store = MockThreadStore::default();
        let created = create_thread(&store, Some("T".to_string())).await.unwrap();
        let loaded = get_thread(&store, &created.id).await.unwrap().unwrap();
        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.metadata.title.as_deref(), Some("T"));
    }

    #[tokio::test]
    async fn list_threads_paginated() {
        let store = MockThreadStore::default();
        for _ in 0..5 {
            create_thread(&store, None).await.unwrap();
        }
        let page1 = list_threads(&store, 0, 3).await.unwrap();
        assert_eq!(page1.len(), 3);
        let page2 = list_threads(&store, 3, 3).await.unwrap();
        assert_eq!(page2.len(), 2);
    }

    #[tokio::test]
    async fn update_thread_title_works() {
        let store = MockThreadStore::default();
        let created = create_thread(&store, Some("Old".to_string()))
            .await
            .unwrap();
        let updated = update_thread_title(&store, &created.id, "New".to_string())
            .await
            .unwrap();
        assert_eq!(updated.metadata.title.as_deref(), Some("New"));
    }

    #[tokio::test]
    async fn update_thread_title_not_found() {
        let store = MockThreadStore::default();
        let result = update_thread_title(&store, "missing", "Title".to_string()).await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn patch_thread_updates_title_and_custom() {
        let store = MockThreadStore::default();
        let thread = create_thread(&store, Some("original".into()))
            .await
            .unwrap();

        let mut custom = std::collections::HashMap::new();
        custom.insert("key1".into(), serde_json::json!("value1"));

        let updated = patch_thread(&store, &thread.id, Some("new title".into()), Some(custom))
            .await
            .unwrap();

        assert_eq!(updated.metadata.title.as_deref(), Some("new title"));
        assert_eq!(
            updated.metadata.custom.get("key1").unwrap(),
            &serde_json::json!("value1")
        );
        assert!(updated.metadata.updated_at.is_some());
    }

    #[tokio::test]
    async fn patch_thread_not_found() {
        let store = MockThreadStore::default();
        let result = patch_thread(&store, "nonexistent", None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_thread_removes_it() {
        let store = MockThreadStore::default();
        let thread = create_thread(&store, Some("to-delete".into()))
            .await
            .unwrap();
        let result = delete_thread(&store, &thread.id).await.unwrap();
        assert!(result);
        let loaded = store.load_thread(&thread.id).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn delete_thread_not_found() {
        let store = MockThreadStore::default();
        let result = delete_thread(&store, "nonexistent").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_summaries_returns_metadata() {
        let store = MockThreadStore::default();
        let t1 = create_thread(&store, Some("Alpha".into())).await.unwrap();
        let t2 = create_thread(&store, Some("Beta".into())).await.unwrap();
        let summaries = list_summaries(&store, 0, 10).await.unwrap();
        assert_eq!(summaries.len(), 2);
        // Both thread IDs appear in the summaries
        let ids: Vec<&str> = summaries
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&t1.id.as_str()));
        assert!(ids.contains(&t2.id.as_str()));
        // Each summary has the expected fields
        for s in &summaries {
            assert!(s.get("id").is_some());
            assert!(s.get("title").is_some());
            assert!(s.get("updated_at").is_some());
        }
    }
}
