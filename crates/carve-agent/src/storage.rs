//! Storage backend trait for session persistence.

use crate::session::Session;
use async_trait::async_trait;
use std::path::PathBuf;
use thiserror::Error;

/// Storage errors.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Session not found.
    #[error("Session not found: {0}")]
    NotFound(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Storage backend trait for session persistence.
///
/// Implementations handle the actual IO operations.
#[async_trait]
pub trait Storage: Send + Sync {
    /// Load a session by ID.
    ///
    /// Returns `Ok(None)` if the session doesn't exist.
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError>;

    /// Save a session.
    async fn save(&self, session: &Session) -> Result<(), StorageError>;

    /// Delete a session.
    async fn delete(&self, id: &str) -> Result<(), StorageError>;

    /// List all session IDs.
    async fn list(&self) -> Result<Vec<String>, StorageError>;
}

/// File-based storage implementation.
pub struct FileStorage {
    base_path: PathBuf,
}

impl FileStorage {
    /// Create a new file storage with the given base path.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.base_path.join(format!("{}.json", id))
    }
}

#[async_trait]
impl Storage for FileStorage {
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError> {
        let path = self.session_path(id);

        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let session: Session =
            serde_json::from_str(&content).map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(session))
    }

    async fn save(&self, session: &Session) -> Result<(), StorageError> {
        // Ensure directory exists
        if !self.base_path.exists() {
            tokio::fs::create_dir_all(&self.base_path).await?;
        }

        let path = self.session_path(&session.id);
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        tokio::fs::write(&path, content).await?;

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let path = self.session_path(id);

        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }

        Ok(())
    }

    async fn list(&self) -> Result<Vec<String>, StorageError> {
        if !self.base_path.exists() {
            return Ok(Vec::new());
        }

        let mut entries = tokio::fs::read_dir(&self.base_path).await?;
        let mut ids = Vec::new();

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    if let Some(id) = stem.to_str() {
                        ids.push(id.to_string());
                    }
                }
            }
        }

        Ok(ids)
    }
}

/// In-memory storage for testing.
#[derive(Default)]
pub struct MemoryStorage {
    sessions: tokio::sync::RwLock<std::collections::HashMap<String, Session>>,
}

impl MemoryStorage {
    /// Create a new in-memory storage.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(id).cloned())
    }

    async fn save(&self, session: &Session) -> Result<(), StorageError> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(id);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<String>, StorageError> {
        let sessions = self.sessions.read().await;
        Ok(sessions.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Message;
    use carve_state::{path, Op, Patch, TrackedPatch};
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_memory_storage_save_load() {
        let storage = MemoryStorage::new();
        let session = Session::new("test-1").with_message(Message::user("Hello"));

        storage.save(&session).await.unwrap();
        let loaded = storage.load("test-1").await.unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.message_count(), 1);
    }

    #[tokio::test]
    async fn test_memory_storage_load_not_found() {
        let storage = MemoryStorage::new();
        let loaded = storage.load("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_delete() {
        let storage = MemoryStorage::new();
        let session = Session::new("test-1");

        storage.save(&session).await.unwrap();
        assert!(storage.load("test-1").await.unwrap().is_some());

        storage.delete("test-1").await.unwrap();
        assert!(storage.load("test-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_list() {
        let storage = MemoryStorage::new();

        storage.save(&Session::new("session-1")).await.unwrap();
        storage.save(&Session::new("session-2")).await.unwrap();

        let mut ids = storage.list().await.unwrap();
        ids.sort();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"session-1".to_string()));
        assert!(ids.contains(&"session-2".to_string()));
    }

    #[tokio::test]
    async fn test_memory_storage_update_session() {
        let storage = MemoryStorage::new();

        // Save initial session
        let session = Session::new("test-1").with_message(Message::user("Hello"));
        storage.save(&session).await.unwrap();

        // Update session
        let session = session.with_message(Message::assistant("Hi!"));
        storage.save(&session).await.unwrap();

        // Load and verify
        let loaded = storage.load("test-1").await.unwrap().unwrap();
        assert_eq!(loaded.message_count(), 2);
    }

    #[tokio::test]
    async fn test_memory_storage_with_state_and_patches() {
        let storage = MemoryStorage::new();

        let session = Session::with_initial_state("test-1", json!({"counter": 0}))
            .with_message(Message::user("Increment"))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(5))),
            ));

        storage.save(&session).await.unwrap();

        let loaded = storage.load("test-1").await.unwrap().unwrap();
        assert_eq!(loaded.patch_count(), 1);
        assert_eq!(loaded.state["counter"], 0);

        // Rebuild state should apply patches
        let state = loaded.rebuild_state().unwrap();
        assert_eq!(state["counter"], 5);
    }

    #[tokio::test]
    async fn test_memory_storage_delete_nonexistent() {
        let storage = MemoryStorage::new();
        // Deleting non-existent session should not error
        storage.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn test_memory_storage_list_empty() {
        let storage = MemoryStorage::new();
        let ids = storage.list().await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn test_memory_storage_concurrent_access() {
        use std::sync::Arc;

        let storage = Arc::new(MemoryStorage::new());

        // Spawn multiple tasks
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let storage = Arc::clone(&storage);
                tokio::spawn(async move {
                    let session = Session::new(format!("session-{}", i));
                    storage.save(&session).await.unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        let ids = storage.list().await.unwrap();
        assert_eq!(ids.len(), 10);
    }

    // File storage tests

    #[tokio::test]
    async fn test_file_storage_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let session = Session::new("test-1").with_message(Message::user("Hello"));
        storage.save(&session).await.unwrap();

        let loaded = storage.load("test-1").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.message_count(), 1);
    }

    #[tokio::test]
    async fn test_file_storage_load_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let loaded = storage.load("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_file_storage_delete() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let session = Session::new("test-1");
        storage.save(&session).await.unwrap();
        assert!(storage.load("test-1").await.unwrap().is_some());

        storage.delete("test-1").await.unwrap();
        assert!(storage.load("test-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_file_storage_list() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        storage.save(&Session::new("session-a")).await.unwrap();
        storage.save(&Session::new("session-b")).await.unwrap();
        storage.save(&Session::new("session-c")).await.unwrap();

        let mut ids = storage.list().await.unwrap();
        ids.sort();

        assert_eq!(ids.len(), 3);
        assert_eq!(ids, vec!["session-a", "session-b", "session-c"]);
    }

    #[tokio::test]
    async fn test_file_storage_list_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let ids = storage.list().await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn test_file_storage_list_nonexistent_dir() {
        let storage = FileStorage::new("/tmp/nonexistent_carve_test_dir");
        let ids = storage.list().await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn test_file_storage_creates_directory() {
        let temp_dir = TempDir::new().unwrap();
        let nested_path = temp_dir.path().join("nested").join("sessions");
        let storage = FileStorage::new(&nested_path);

        let session = Session::new("test-1");
        storage.save(&session).await.unwrap();

        assert!(nested_path.exists());
        assert!(storage.load("test-1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_file_storage_with_complex_session() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let session = Session::with_initial_state("complex-session", json!({
            "user": {"name": "Alice", "age": 30},
            "items": [1, 2, 3],
            "active": true
        }))
        .with_message(Message::user("Hello"))
        .with_message(Message::assistant("Hi there!"))
        .with_patch(TrackedPatch::new(
            Patch::new()
                .with_op(Op::set(path!("user").key("age"), json!(31)))
                .with_op(Op::append(path!("items"), json!(4))),
        ));

        storage.save(&session).await.unwrap();

        let loaded = storage.load("complex-session").await.unwrap().unwrap();
        assert_eq!(loaded.message_count(), 2);
        assert_eq!(loaded.patch_count(), 1);

        let state = loaded.rebuild_state().unwrap();
        assert_eq!(state["user"]["age"], 31);
    }

    #[tokio::test]
    async fn test_file_storage_update_session() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        // Save initial
        let session = Session::new("test-1").with_message(Message::user("First"));
        storage.save(&session).await.unwrap();

        // Update
        let session = session
            .with_message(Message::assistant("Second"))
            .with_message(Message::user("Third"));
        storage.save(&session).await.unwrap();

        // Verify
        let loaded = storage.load("test-1").await.unwrap().unwrap();
        assert_eq!(loaded.message_count(), 3);
    }

    #[tokio::test]
    async fn test_file_storage_ignores_non_json_files() {
        let temp_dir = TempDir::new().unwrap();

        // Create a non-JSON file
        tokio::fs::write(temp_dir.path().join("not-json.txt"), "hello")
            .await
            .unwrap();

        let storage = FileStorage::new(temp_dir.path());
        storage.save(&Session::new("valid")).await.unwrap();

        let ids = storage.list().await.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "valid");
    }

    #[test]
    fn test_storage_error_display() {
        let err = StorageError::NotFound("session-1".to_string());
        assert!(err.to_string().contains("not found"));
        assert!(err.to_string().contains("session-1"));

        let err = StorageError::Serialization("invalid json".to_string());
        assert!(err.to_string().contains("Serialization"));
    }

    #[test]
    fn test_file_storage_session_path() {
        let storage = FileStorage::new("/base/path");
        let path = storage.session_path("my-session");
        assert_eq!(path.to_string_lossy(), "/base/path/my-session.json");
    }
}
