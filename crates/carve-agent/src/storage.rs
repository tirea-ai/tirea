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

    /// Invalid session ID (path traversal, control chars, etc.).
    #[error("Invalid session id: {0}")]
    InvalidId(String),
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

    fn session_path(&self, id: &str) -> Result<PathBuf, StorageError> {
        Self::validate_session_id(id)?;
        Ok(self.base_path.join(format!("{}.json", id)))
    }

    /// Validate that a session ID is safe for use as a filename.
    /// Rejects path separators, `..`, and control characters.
    fn validate_session_id(id: &str) -> Result<(), StorageError> {
        if id.is_empty() {
            return Err(StorageError::InvalidId(
                "session id cannot be empty".to_string(),
            ));
        }
        if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
            return Err(StorageError::InvalidId(format!(
                "session id contains invalid characters: {id:?}"
            )));
        }
        if id.chars().any(|c| c.is_control()) {
            return Err(StorageError::InvalidId(format!(
                "session id contains control characters: {id:?}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl Storage for FileStorage {
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError> {
        let path = self.session_path(id)?;

        if !path.exists() {
            return Ok(None);
        }

        let content = tokio::fs::read_to_string(&path).await?;
        let session: Session = serde_json::from_str(&content)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        Ok(Some(session))
    }

    async fn save(&self, session: &Session) -> Result<(), StorageError> {
        // Ensure directory exists
        if !self.base_path.exists() {
            tokio::fs::create_dir_all(&self.base_path).await?;
        }

        let path = self.session_path(&session.id)?;
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;

        tokio::fs::write(&path, content).await?;

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let path = self.session_path(id)?;

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

// ============================================================================
// PostgreSQL Storage (feature = "postgres")
// ============================================================================

/// PostgreSQL-backed storage implementation.
///
/// Stores sessions as JSON in a table with the schema:
///
/// ```sql
/// CREATE TABLE IF NOT EXISTS agent_sessions (
///     id   TEXT PRIMARY KEY,
///     data JSONB NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
/// );
/// ```
///
/// The table is auto-created on first use (via `ensure_table()`), or you can
/// create it yourself with a migration.
#[cfg(feature = "postgres")]
pub struct PostgresStorage {
    pool: sqlx::PgPool,
    table: String,
}

#[cfg(feature = "postgres")]
impl PostgresStorage {
    /// Create a new PostgreSQL storage using the given connection pool.
    ///
    /// Sessions are stored in the `agent_sessions` table by default.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            table: "agent_sessions".to_string(),
        }
    }

    /// Create a new PostgreSQL storage with a custom table name.
    pub fn with_table(pool: sqlx::PgPool, table: impl Into<String>) -> Self {
        Self {
            pool,
            table: table.into(),
        }
    }

    /// Ensure the storage table exists (idempotent).
    pub async fn ensure_table(&self) -> Result<(), StorageError> {
        let sql = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                id         TEXT PRIMARY KEY,
                data       JSONB NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
            self.table
        );
        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl Storage for PostgresStorage {
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError> {
        let sql = format!("SELECT data FROM {} WHERE id = $1", self.table);
        let row: Option<(serde_json::Value,)> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;

        match row {
            Some((v,)) => {
                let session: Session = serde_json::from_value(v)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    async fn save(&self, session: &Session) -> Result<(), StorageError> {
        let v = serde_json::to_value(session)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        let sql = format!(
            r#"
            INSERT INTO {} (id, data, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (id) DO UPDATE
            SET data = EXCLUDED.data, updated_at = now()
            "#,
            self.table
        );
        sqlx::query(&sql)
            .bind(&session.id)
            .bind(v)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let sql = format!("DELETE FROM {} WHERE id = $1", self.table);
        sqlx::query(&sql)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    async fn list(&self) -> Result<Vec<String>, StorageError> {
        let sql = format!("SELECT id FROM {} ORDER BY id", self.table);
        let rows: Vec<(String,)> = sqlx::query_as(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
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

        let session = Session::with_initial_state(
            "complex-session",
            json!({
                "user": {"name": "Alice", "age": 30},
                "items": [1, 2, 3],
                "active": true
            }),
        )
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
        let path = storage.session_path("my-session").unwrap();
        assert_eq!(path.to_string_lossy(), "/base/path/my-session.json");
    }

    #[test]
    fn test_file_storage_rejects_path_traversal() {
        let storage = FileStorage::new("/base/path");
        assert!(storage.session_path("../../etc/passwd").is_err());
        assert!(storage.session_path("foo/bar").is_err());
        assert!(storage.session_path("foo\\bar").is_err());
        assert!(storage.session_path("").is_err());
        assert!(storage.session_path("foo\0bar").is_err());
    }
}
