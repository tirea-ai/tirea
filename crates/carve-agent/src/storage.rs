//! Storage backend traits for thread persistence.

use crate::thread::Thread;
use crate::types::{Message, Visibility};
use async_trait::async_trait;
use carve_state::TrackedPatch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "postgres")]
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::AsyncWriteExt;

// ============================================================================
// Pagination types
// ============================================================================

/// Sort order for paginated queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

/// Cursor-based pagination parameters for messages.
#[derive(Debug, Clone)]
pub struct MessageQuery {
    /// Return messages with cursor strictly greater than this value.
    pub after: Option<i64>,
    /// Return messages with cursor strictly less than this value.
    pub before: Option<i64>,
    /// Maximum number of messages to return (clamped to 1..=200).
    pub limit: usize,
    /// Sort order.
    pub order: SortOrder,
    /// Filter by message visibility. `None` means return all messages.
    /// Default: `Some(Visibility::All)` (only user-visible messages).
    pub visibility: Option<Visibility>,
    /// Filter by run ID. `None` means return messages from all runs.
    pub run_id: Option<String>,
}

impl Default for MessageQuery {
    fn default() -> Self {
        Self {
            after: None,
            before: None,
            limit: 50,
            order: SortOrder::Asc,
            visibility: Some(Visibility::All),
            run_id: None,
        }
    }
}

/// A message paired with its storage-assigned cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithCursor {
    pub cursor: i64,
    #[serde(flatten)]
    pub message: Message,
}

/// Paginated message response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePage {
    pub messages: Vec<MessageWithCursor>,
    pub has_more: bool,
    /// Cursor of the last item (use as `after` for next forward page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    /// Cursor of the first item (use as `before` for next backward page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_cursor: Option<i64>,
}

/// Pagination query for session lists.
#[derive(Debug, Clone)]
pub struct ThreadListQuery {
    /// Number of items to skip (0-based).
    pub offset: usize,
    /// Maximum number of items to return (clamped to 1..=200).
    pub limit: usize,
    /// Filter by resource_id (owner). `None` means no filtering.
    pub resource_id: Option<String>,
    /// Filter by parent_thread_id. `None` means no filtering.
    pub parent_thread_id: Option<String>,
}

impl Default for ThreadListQuery {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            resource_id: None,
            parent_thread_id: None,
        }
    }
}

/// Paginated session list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadListPage {
    pub items: Vec<String>,
    pub total: usize,
    pub has_more: bool,
}

/// Paginate a slice of messages in memory.
///
/// Cursor values correspond to the 0-based index in the original slice
/// (not the filtered slice), so cursors remain stable across visibility filters.
pub fn paginate_in_memory(messages: &[std::sync::Arc<Message>], query: &MessageQuery) -> MessagePage {
    let total = messages.len();
    if total == 0 {
        return MessagePage {
            messages: Vec::new(),
            has_more: false,
            next_cursor: None,
            prev_cursor: None,
        };
    }

    // Build (cursor, &Message) pairs filtered by after/before and visibility.
    let start = query.after.map(|c| (c + 1).max(0) as usize).unwrap_or(0);
    let end = query
        .before
        .map(|c| (c.max(0) as usize).min(total))
        .unwrap_or(total);

    if start >= total || start >= end {
        return MessagePage {
            messages: Vec::new(),
            has_more: false,
            next_cursor: None,
            prev_cursor: None,
        };
    }

    let mut items: Vec<(i64, &std::sync::Arc<Message>)> = messages[start..end]
        .iter()
        .enumerate()
        .filter(|(_, m)| match query.visibility {
            Some(vis) => m.visibility == vis,
            None => true,
        })
        .filter(|(_, m)| match &query.run_id {
            Some(rid) => {
                m.metadata.as_ref().and_then(|meta| meta.run_id.as_deref()) == Some(rid.as_str())
            }
            None => true,
        })
        .map(|(i, m)| ((start + i) as i64, m))
        .collect();

    if query.order == SortOrder::Desc {
        items.reverse();
    }

    let has_more = items.len() > query.limit;
    let limited: Vec<_> = items.into_iter().take(query.limit).collect();

    MessagePage {
        next_cursor: limited.last().map(|(c, _)| *c),
        prev_cursor: limited.first().map(|(c, _)| *c),
        messages: limited
            .into_iter()
            .map(|(c, m)| MessageWithCursor {
                cursor: c,
                message: (**m).clone(),
            })
            .collect(),
        has_more,
    }
}

/// Storage errors.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Thread not found.
    #[error("Thread not found: {0}")]
    NotFound(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Invalid thread ID (path traversal, control chars, etc.).
    #[error("Invalid thread id: {0}")]
    InvalidId(String),

    /// Optimistic concurrency conflict.
    #[error("Version conflict: expected {expected}, actual {actual}")]
    VersionConflict {
        expected: Version,
        actual: Version,
    },

    /// Thread already exists (for create operations).
    #[error("Thread already exists")]
    AlreadyExists,
}

// ============================================================================
// Delta-based storage types
// ============================================================================

/// Monotonically increasing version for optimistic concurrency.
pub type Version = u64;

/// Acknowledgement returned after a successful write.
#[derive(Debug, Clone, Copy)]
pub struct Committed {
    pub version: Version,
}

/// A thread together with its current storage version.
#[derive(Debug, Clone)]
pub struct ThreadHead {
    pub thread: Thread,
    pub version: Version,
}

/// Reason for a checkpoint (delta).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CheckpointReason {
    UserMessage,
    AssistantTurnCommitted,
    ToolResultsCommitted,
    RunFinished,
}

/// An incremental change to a thread produced by a single step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadDelta {
    /// Which run produced this delta.
    pub run_id: String,
    /// Parent run (for sub-agent deltas).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Why this delta was created.
    pub reason: CheckpointReason,
    /// New messages appended in this step.
    pub messages: Vec<Arc<Message>>,
    /// New patches appended in this step.
    pub patches: Vec<TrackedPatch>,
    /// If `Some`, a full state snapshot was taken (replaces base state).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Value>,
}

// ============================================================================
// New storage traits
// ============================================================================

/// Core thread storage — all backends must implement this.
#[async_trait]
pub trait ThreadStore: Send + Sync {
    /// Create a new thread. Returns `AlreadyExists` if the id is taken.
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError>;

    /// Append a delta to an existing thread.
    ///
    /// `base_version` is the version the caller last read; if it doesn't match
    /// the current version the backend returns `VersionConflict`.
    async fn append(
        &self,
        id: &str,
        base_version: Version,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError>;

    /// Load a thread and its current version.
    async fn load(&self, id: &str) -> Result<Option<ThreadHead>, StorageError>;

    /// Delete a thread.
    async fn delete(&self, id: &str) -> Result<(), StorageError>;

    /// Upsert a thread (delete + create). Convenience wrapper.
    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        let _ = self.delete(&thread.id).await;
        self.create(thread).await?;
        Ok(())
    }

    /// Load a thread without version info. Convenience wrapper.
    async fn load_thread(&self, id: &str) -> Result<Option<Thread>, StorageError> {
        Ok(self.load(id).await?.map(|h| h.thread))
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
        id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, StorageError> {
        let head = self
            .load(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
        Ok(paginate_in_memory(&head.thread.messages, query))
    }

    /// List threads with pagination.
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError>;

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
    async fn message_count(&self, id: &str) -> Result<usize, StorageError> {
        let head = self
            .load(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
        Ok(head.thread.messages.len())
    }
}

/// Sync operations — for backends with delta replay capability.
#[async_trait]
pub trait ThreadSync: ThreadStore {
    /// Load deltas appended after `after_version`.
    async fn load_deltas(
        &self,
        id: &str,
        after_version: Version,
    ) -> Result<Vec<ThreadDelta>, StorageError>;
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

    fn thread_path(&self, id: &str) -> Result<PathBuf, StorageError> {
        Self::validate_thread_id(id)?;
        Ok(self.base_path.join(format!("{}.json", id)))
    }

    /// Validate that a session ID is safe for use as a filename.
    /// Rejects path separators, `..`, and control characters.
    fn validate_thread_id(id: &str) -> Result<(), StorageError> {
        if id.is_empty() {
            return Err(StorageError::InvalidId(
                "thread id cannot be empty".to_string(),
            ));
        }
        if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
            return Err(StorageError::InvalidId(format!(
                "thread id contains invalid characters: {id:?}"
            )));
        }
        if id.chars().any(|c| c.is_control()) {
            return Err(StorageError::InvalidId(format!(
                "thread id contains control characters: {id:?}"
            )));
        }
        Ok(())
    }
}


#[async_trait]
impl ThreadStore for FileStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError> {
        let path = self.thread_path(&thread.id)?;
        if path.exists() {
            return Err(StorageError::AlreadyExists);
        }
        // Serialize with version=0 embedded
        let head = ThreadHead {
            thread: thread.clone(),
            version: 0,
        };
        self.save_head(&head).await?;
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        id: &str,
        base_version: Version,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError> {
        let head = self
            .load_head(id)
            .await?
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;

        if head.version != base_version {
            return Err(StorageError::VersionConflict {
                expected: base_version,
                actual: head.version,
            });
        }

        let mut thread = head.thread;
        apply_delta(&mut thread, delta);
        let new_version = head.version + 1;
        let new_head = ThreadHead {
            thread,
            version: new_version,
        };
        self.save_head(&new_head).await?;
        Ok(Committed {
            version: new_version,
        })
    }

    async fn load(&self, id: &str) -> Result<Option<ThreadHead>, StorageError> {
        self.load_head(id).await
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let path = self.thread_path(id)?;
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl ThreadQuery for FileStorage {
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
        // Read directory for all thread IDs
        let mut all = if !self.base_path.exists() {
            Vec::new()
        } else {
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
            ids
        };

        // Filter by resource_id if specified.
        if let Some(ref resource_id) = query.resource_id {
            let mut filtered = Vec::new();
            for id in &all {
                if let Some(head) = self.load(id).await? {
                    if head.thread.resource_id.as_deref() == Some(resource_id.as_str()) {
                        filtered.push(id.clone());
                    }
                }
            }
            all = filtered;
        }

        // Filter by parent_thread_id if specified.
        if let Some(ref parent_thread_id) = query.parent_thread_id {
            let mut filtered = Vec::new();
            for id in &all {
                if let Some(head) = self.load(id).await? {
                    if head.thread.parent_thread_id.as_deref() == Some(parent_thread_id.as_str()) {
                        filtered.push(id.clone());
                    }
                }
            }
            all = filtered;
        }

        all.sort();
        let total = all.len();
        let limit = query.limit.clamp(1, 200);
        let offset = query.offset.min(total);
        let end = (offset + limit + 1).min(total);
        let slice = &all[offset..end];
        let has_more = slice.len() > limit;
        let items: Vec<String> = slice.iter().take(limit).cloned().collect();
        Ok(ThreadListPage {
            items,
            total,
            has_more,
        })
    }
}

impl FileStorage {
    /// Load a thread head (thread + version) from file.
    async fn load_head(&self, id: &str) -> Result<Option<ThreadHead>, StorageError> {
        let path = self.thread_path(id)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        // Try to parse as ThreadHead first (new format with version).
        if let Ok(head) = serde_json::from_str::<VersionedThread>(&content) {
            let thread: Thread = serde_json::from_str(&content)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            Ok(Some(ThreadHead {
                thread,
                version: head._version.unwrap_or(0),
            }))
        } else {
            let thread: Thread = serde_json::from_str(&content)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            Ok(Some(ThreadHead { thread, version: 0 }))
        }
    }

    /// Save a thread head (thread + version) to file atomically.
    async fn save_head(&self, head: &ThreadHead) -> Result<(), StorageError> {
        if !self.base_path.exists() {
            tokio::fs::create_dir_all(&self.base_path).await?;
        }
        let path = self.thread_path(&head.thread.id)?;

        // Embed version into the JSON
        let mut v = serde_json::to_value(&head.thread)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("_version".to_string(), serde_json::json!(head.version));
        }
        let content =
            serde_json::to_string_pretty(&v).map_err(|e| StorageError::Serialization(e.to_string()))?;

        let tmp_path = self.base_path.join(format!(
            ".{}.{}.tmp",
            head.thread.id,
            uuid::Uuid::new_v4().simple()
        ));

        let write_result = async {
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            file.write_all(content.as_bytes()).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            match tokio::fs::rename(&tmp_path, &path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tokio::fs::remove_file(&path).await?;
                    tokio::fs::rename(&tmp_path, &path).await?;
                }
                Err(e) => return Err(e),
            }
            Ok::<(), std::io::Error>(())
        }
        .await;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(StorageError::Io(e));
        }
        Ok(())
    }
}

/// Helper for extracting the `_version` field from serialized thread JSON.
#[derive(Deserialize)]
struct VersionedThread {
    #[serde(default)]
    _version: Option<Version>,
}

/// In-memory storage entry.
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

/// Apply a delta to a thread in-place.
fn apply_delta(thread: &mut Thread, delta: &ThreadDelta) {
    thread.messages.extend(delta.messages.iter().cloned());
    thread.patches.extend(delta.patches.iter().cloned());
    if let Some(ref snapshot) = delta.snapshot {
        thread.state = snapshot.clone();
        thread.patches.clear();
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
        id: &str,
        base_version: Version,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError> {
        let mut entries = self.entries.write().await;
        let entry = entries
            .get_mut(id)
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;

        if entry.version != base_version {
            return Err(StorageError::VersionConflict {
                expected: base_version,
                actual: entry.version,
            });
        }

        apply_delta(&mut entry.thread, delta);
        entry.version += 1;
        entry.deltas.push(delta.clone());
        Ok(Committed {
            version: entry.version,
        })
    }

    async fn load(&self, id: &str) -> Result<Option<ThreadHead>, StorageError> {
        let entries = self.entries.read().await;
        Ok(entries.get(id).map(|e| ThreadHead {
            thread: e.thread.clone(),
            version: e.version,
        }))
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        let mut entries = self.entries.write().await;
        entries.remove(id);
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
    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, StorageError> {
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
        id: &str,
        after_version: Version,
    ) -> Result<Vec<ThreadDelta>, StorageError> {
        let entries = self.entries.read().await;
        let entry = entries
            .get(id)
            .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
        // Deltas are 1-indexed: delta[0] produced version 1, delta[1] produced version 2, etc.
        let skip = after_version as usize;
        Ok(entry.deltas[skip..].to_vec())
    }
}

// ============================================================================
// PostgreSQL Storage (feature = "postgres")
// ============================================================================

/// PostgreSQL-backed storage implementation.
///
/// Stores session metadata in `agent_sessions` and messages in a separate
/// `agent_messages` table for efficient cursor-based pagination.
///
/// Messages are **append-only**: `save()` only inserts new messages (identified
/// by `message_id`), never deletes existing ones.
///
/// ```sql
/// CREATE TABLE IF NOT EXISTS agent_sessions (
///     id         TEXT PRIMARY KEY,
///     data       JSONB NOT NULL,
///     updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
/// );
///
/// CREATE TABLE IF NOT EXISTS agent_messages (
///     seq        BIGSERIAL PRIMARY KEY,
///     session_id TEXT NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
///     message_id TEXT,
///     run_id     TEXT,
///     step_index INTEGER,
///     data       JSONB NOT NULL,
///     created_at TIMESTAMPTZ NOT NULL DEFAULT now()
/// );
/// ```
///
/// The tables are auto-created on first use (via `ensure_table()`), or you can
/// create them yourself with a migration.
#[cfg(feature = "postgres")]
pub struct PostgresStorage {
    pool: sqlx::PgPool,
    table: String,
    messages_table: String,
}

#[cfg(feature = "postgres")]
impl PostgresStorage {
    /// Create a new PostgreSQL storage using the given connection pool.
    ///
    /// Sessions are stored in the `agent_sessions` table by default,
    /// messages in `agent_messages`.
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self {
            pool,
            table: "agent_sessions".to_string(),
            messages_table: "agent_messages".to_string(),
        }
    }

    /// Create a new PostgreSQL storage with a custom table name.
    ///
    /// The messages table will be named `{table}_messages`.
    pub fn with_table(pool: sqlx::PgPool, table: impl Into<String>) -> Self {
        let table = table.into();
        let messages_table = format!("{}_messages", table);
        Self {
            pool,
            table,
            messages_table,
        }
    }

    /// Ensure the storage tables exist (idempotent).
    pub async fn ensure_table(&self) -> Result<(), StorageError> {
        let sql = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {threads} (
                id         TEXT PRIMARY KEY,
                data       JSONB NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE TABLE IF NOT EXISTS {messages} (
                seq        BIGSERIAL PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES {threads}(id) ON DELETE CASCADE,
                message_id TEXT,
                run_id     TEXT,
                step_index INTEGER,
                data       JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE INDEX IF NOT EXISTS idx_{messages}_session_seq
                ON {messages} (session_id, seq);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_{messages}_message_id
                ON {messages} (message_id) WHERE message_id IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_{messages}_session_run
                ON {messages} (session_id, run_id) WHERE run_id IS NOT NULL;
            "#,
            threads = self.table,
            messages = self.messages_table,
        );
        sqlx::query(&sql)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        Ok(())
    }

    fn sql_err(e: sqlx::Error) -> StorageError {
        StorageError::Io(std::io::Error::other(e.to_string()))
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl ThreadStore for PostgresStorage {
    async fn create(&self, thread: &Thread) -> Result<Committed, StorageError> {
        let mut v = serde_json::to_value(thread)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("messages".to_string(), serde_json::Value::Array(Vec::new()));
            obj.insert("_version".to_string(), serde_json::Value::Number(0.into()));
        }

        let sql = format!(
            "INSERT INTO {} (id, data, updated_at) VALUES ($1, $2, now())",
            self.table
        );
        sqlx::query(&sql)
            .bind(&thread.id)
            .bind(&v)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                if e.to_string().contains("duplicate key")
                    || e.to_string().contains("unique constraint")
                {
                    StorageError::AlreadyExists
                } else {
                    Self::sql_err(e)
                }
            })?;

        // Insert messages into separate table.
        let insert_sql = format!(
            "INSERT INTO {} (session_id, message_id, run_id, step_index, data) VALUES ($1, $2, $3, $4, $5)",
            self.messages_table,
        );
        for msg in &thread.messages {
            let data = serde_json::to_value(msg.as_ref())
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let message_id = msg.id.as_deref();
            let (run_id, step_index) = msg
                .metadata
                .as_ref()
                .map(|m| (m.run_id.as_deref(), m.step_index.map(|s| s as i32)))
                .unwrap_or((None, None));
            sqlx::query(&insert_sql)
                .bind(&thread.id)
                .bind(message_id)
                .bind(run_id)
                .bind(step_index)
                .bind(&data)
                .execute(&self.pool)
                .await
                .map_err(Self::sql_err)?;
        }

        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        id: &str,
        base_version: Version,
        delta: &ThreadDelta,
    ) -> Result<Committed, StorageError> {
        let mut tx = self.pool.begin().await.map_err(Self::sql_err)?;

        // Optimistic concurrency: check version.
        let sql = format!(
            "SELECT data FROM {} WHERE id = $1 FOR UPDATE",
            self.table
        );
        let row: Option<(serde_json::Value,)> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(Self::sql_err)?;

        let Some((mut v,)) = row else {
            return Err(StorageError::NotFound(id.to_string()));
        };

        let current_version = v
            .get("_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if current_version != base_version {
            return Err(StorageError::VersionConflict {
                expected: base_version,
                actual: current_version,
            });
        }

        let new_version = current_version + 1;

        // Apply snapshot or patches to stored data.
        if let Some(ref snapshot) = delta.snapshot {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("state".to_string(), snapshot.clone());
                obj.insert("patches".to_string(), serde_json::Value::Array(Vec::new()));
            }
        } else if !delta.patches.is_empty() {
            let patches_arr = v
                .get("patches")
                .cloned()
                .unwrap_or(serde_json::Value::Array(Vec::new()));
            let mut patches: Vec<serde_json::Value> = if let serde_json::Value::Array(arr) = patches_arr {
                arr
            } else {
                Vec::new()
            };
            for p in &delta.patches {
                if let Ok(pv) = serde_json::to_value(p) {
                    patches.push(pv);
                }
            }
            if let Some(obj) = v.as_object_mut() {
                obj.insert("patches".to_string(), serde_json::Value::Array(patches));
            }
        }

        if let Some(obj) = v.as_object_mut() {
            obj.insert("_version".to_string(), serde_json::Value::Number(new_version.into()));
        }

        let update_sql = format!(
            "UPDATE {} SET data = $1, updated_at = now() WHERE id = $2",
            self.table
        );
        sqlx::query(&update_sql)
            .bind(&v)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(Self::sql_err)?;

        // Append new messages.
        if !delta.messages.is_empty() {
            let insert_sql = format!(
                "INSERT INTO {} (session_id, message_id, run_id, step_index, data) VALUES ($1, $2, $3, $4, $5)",
                self.messages_table,
            );
            for msg in &delta.messages {
                let data = serde_json::to_value(msg.as_ref())
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                let message_id = msg.id.as_deref();
                let (run_id, step_index) = msg
                    .metadata
                    .as_ref()
                    .map(|m| (m.run_id.as_deref(), m.step_index.map(|s| s as i32)))
                    .unwrap_or((None, None));
                sqlx::query(&insert_sql)
                    .bind(id)
                    .bind(message_id)
                    .bind(run_id)
                    .bind(step_index)
                    .bind(&data)
                    .execute(&mut *tx)
                    .await
                    .map_err(Self::sql_err)?;
            }
        }

        tx.commit().await.map_err(Self::sql_err)?;
        Ok(Committed { version: new_version })
    }

    async fn load(&self, id: &str) -> Result<Option<ThreadHead>, StorageError> {
        let sql = format!("SELECT data FROM {} WHERE id = $1", self.table);
        let row: Option<(serde_json::Value,)> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;

        let Some((mut v,)) = row else {
            return Ok(None);
        };

        let version = v
            .get("_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Load messages from separate table.
        let msg_sql = format!(
            "SELECT data FROM {} WHERE session_id = $1 ORDER BY seq",
            self.messages_table
        );
        let msg_rows: Vec<(serde_json::Value,)> = sqlx::query_as(&msg_sql)
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(Self::sql_err)?;

        let messages: Vec<serde_json::Value> = msg_rows.into_iter().map(|(d,)| d).collect();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("messages".to_string(), serde_json::Value::Array(messages));
            obj.remove("_version");
        }

        let thread: Thread =
            serde_json::from_value(v).map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(Some(ThreadHead { thread, version }))
    }

    async fn delete(&self, id: &str) -> Result<(), StorageError> {
        // CASCADE will delete messages automatically.
        let sql = format!("DELETE FROM {} WHERE id = $1", self.table);
        sqlx::query(&sql)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        Ok(())
    }

    async fn save(&self, thread: &Thread) -> Result<(), StorageError> {
        // Serialize session skeleton (without messages).
        let mut v = serde_json::to_value(thread)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("messages".to_string(), serde_json::Value::Array(Vec::new()));
        }

        // Use a transaction to keep sessions and messages consistent.
        let mut tx = self.pool.begin().await.map_err(Self::sql_err)?;

        // Upsert session skeleton.
        let upsert_sql = format!(
            r#"
            INSERT INTO {} (id, data, updated_at)
            VALUES ($1, $2, now())
            ON CONFLICT (id) DO UPDATE
            SET data = EXCLUDED.data, updated_at = now()
            "#,
            self.table
        );
        sqlx::query(&upsert_sql)
            .bind(&thread.id)
            .bind(&v)
            .execute(&mut *tx)
            .await
            .map_err(Self::sql_err)?;

        // Incremental append: only INSERT messages not yet persisted.
        let existing_sql = format!(
            "SELECT message_id FROM {} WHERE session_id = $1 AND message_id IS NOT NULL",
            self.messages_table,
        );
        let existing_rows: Vec<(String,)> = sqlx::query_as(&existing_sql)
            .bind(&thread.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(Self::sql_err)?;
        let existing_ids: HashSet<String> = existing_rows.into_iter().map(|(id,)| id).collect();

        let new_messages: Vec<&Message> = thread
            .messages
            .iter()
            .filter(|m| m.id.as_ref().map_or(true, |id| !existing_ids.contains(id)))
            .map(|m| m.as_ref())
            .collect();

        let insert_sql = format!(
            "INSERT INTO {} (session_id, message_id, run_id, step_index, data) VALUES ($1, $2, $3, $4, $5)",
            self.messages_table,
        );
        for msg in &new_messages {
            let data = serde_json::to_value(msg)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            let message_id = msg.id.as_deref();
            let (run_id, step_index) = msg
                .metadata
                .as_ref()
                .map(|m| (m.run_id.as_deref(), m.step_index.map(|s| s as i32)))
                .unwrap_or((None, None));

            sqlx::query(&insert_sql)
                .bind(&thread.id)
                .bind(message_id)
                .bind(run_id)
                .bind(step_index)
                .bind(&data)
                .execute(&mut *tx)
                .await
                .map_err(Self::sql_err)?;
        }

        tx.commit().await.map_err(Self::sql_err)?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl ThreadQuery for PostgresStorage {
    async fn load_messages(
        &self,
        id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, StorageError> {
        // Check session exists.
        let exists_sql = format!("SELECT 1 FROM {} WHERE id = $1", self.table);
        let exists: Option<(i32,)> = sqlx::query_as(&exists_sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        if exists.is_none() {
            return Err(StorageError::NotFound(id.to_string()));
        }

        let limit = query.limit.min(200).max(1);
        // Fetch limit+1 rows to determine has_more.
        let fetch_limit = (limit + 1) as i64;

        // Visibility filter on JSONB data.
        let vis_clause = match query.visibility {
            Some(Visibility::All) => {
                " AND COALESCE(data->>'visibility', 'all') = 'all'".to_string()
            }
            Some(Visibility::Internal) => " AND data->>'visibility' = 'internal'".to_string(),
            None => String::new(),
        };

        // Run ID filter on the run_id column.
        let run_clause = if query.run_id.is_some() {
            " AND run_id = $4"
        } else {
            ""
        };

        let extra_param_idx = if query.run_id.is_some() { 5 } else { 4 };

        let (sql, cursor_val) = match query.order {
            SortOrder::Asc => {
                let cursor = query.after.unwrap_or(-1);
                let before_clause = if query.before.is_some() {
                    format!("AND seq < ${extra_param_idx}")
                } else {
                    String::new()
                };
                let sql = format!(
                    "SELECT seq, data FROM {} WHERE session_id = $1 AND seq > $2{}{} {} ORDER BY seq ASC LIMIT $3",
                    self.messages_table, vis_clause, run_clause, before_clause,
                );
                (sql, cursor)
            }
            SortOrder::Desc => {
                let cursor = query.before.unwrap_or(i64::MAX);
                let after_clause = if query.after.is_some() {
                    format!("AND seq > ${extra_param_idx}")
                } else {
                    String::new()
                };
                let sql = format!(
                    "SELECT seq, data FROM {} WHERE session_id = $1 AND seq < $2{}{} {} ORDER BY seq DESC LIMIT $3",
                    self.messages_table, vis_clause, run_clause, after_clause,
                );
                (sql, cursor)
            }
        };

        let rows: Vec<(i64, serde_json::Value)> = match query.order {
            SortOrder::Asc => {
                let mut q = sqlx::query_as(&sql)
                    .bind(id)
                    .bind(cursor_val)
                    .bind(fetch_limit);
                if let Some(ref rid) = query.run_id {
                    q = q.bind(rid);
                }
                if let Some(before) = query.before {
                    q = q.bind(before);
                }
                q.fetch_all(&self.pool).await.map_err(Self::sql_err)?
            }
            SortOrder::Desc => {
                let mut q = sqlx::query_as(&sql)
                    .bind(id)
                    .bind(cursor_val)
                    .bind(fetch_limit);
                if let Some(ref rid) = query.run_id {
                    q = q.bind(rid);
                }
                if let Some(after) = query.after {
                    q = q.bind(after);
                }
                q.fetch_all(&self.pool).await.map_err(Self::sql_err)?
            }
        };

        let has_more = rows.len() > limit;
        let limited: Vec<_> = rows.into_iter().take(limit).collect();

        let messages: Vec<MessageWithCursor> = limited
            .into_iter()
            .map(|(seq, data)| {
                let message: Message = serde_json::from_value(data)
                    .unwrap_or_else(|_| Message::system("[deserialization error]"));
                MessageWithCursor {
                    cursor: seq,
                    message,
                }
            })
            .collect();

        Ok(MessagePage {
            next_cursor: messages.last().map(|m| m.cursor),
            prev_cursor: messages.first().map(|m| m.cursor),
            messages,
            has_more,
        })
    }

    async fn message_count(&self, id: &str) -> Result<usize, StorageError> {
        // Check session exists.
        let exists_sql = format!("SELECT 1 FROM {} WHERE id = $1", self.table);
        let exists: Option<(i32,)> = sqlx::query_as(&exists_sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        if exists.is_none() {
            return Err(StorageError::NotFound(id.to_string()));
        }

        let sql = format!(
            "SELECT COUNT(*)::bigint FROM {} WHERE session_id = $1",
            self.messages_table
        );
        let row: (i64,) = sqlx::query_as(&sql)
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        Ok(row.0 as usize)
    }

    async fn list_threads(&self, query: &ThreadListQuery) -> Result<ThreadListPage, StorageError> {
        // Enhanced with parent_thread_id filter via JSONB.
        let limit = query.limit.clamp(1, 200);
        let fetch_limit = (limit + 1) as i64;
        let offset = query.offset as i64;

        let mut where_clauses = Vec::new();
        let mut param_idx = 3; // $1=limit, $2=offset

        if query.resource_id.is_some() {
            where_clauses.push(format!("data->>'resource_id' = ${param_idx}"));
            param_idx += 1;
        }
        if query.parent_thread_id.is_some() {
            where_clauses.push(format!("data->>'parent_thread_id' = ${param_idx}"));
            // param_idx += 1; // unused after this
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {}{}",
            self.table, where_sql
        );
        let sql = format!(
            "SELECT id FROM {}{} ORDER BY id LIMIT $1 OFFSET $2",
            self.table, where_sql
        );

        // Build and execute count query.
        let mut count_q = sqlx::query_as::<_, (i64,)>(&count_sql);
        // Note: count query does not use $1/$2 (limit/offset) — bind filter params directly.
        // We need to rebind for the count query which doesn't have $1/$2.
        // Actually, let's simplify: build count and data queries with proper param ordering.

        // Simpler approach: build WHERE clause starting from $1 for count, $3 for data.
        let mut filter_clauses_count = Vec::new();
        let mut count_param_idx = 1;
        if query.resource_id.is_some() {
            filter_clauses_count.push(format!("data->>'resource_id' = ${count_param_idx}"));
            count_param_idx += 1;
        }
        if query.parent_thread_id.is_some() {
            filter_clauses_count.push(format!("data->>'parent_thread_id' = ${count_param_idx}"));
            // count_param_idx += 1;
        }
        let where_count = if filter_clauses_count.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filter_clauses_count.join(" AND "))
        };

        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {}{}",
            self.table, where_count
        );
        let mut count_q = sqlx::query_as::<_, (i64,)>(&count_sql);
        if let Some(ref rid) = query.resource_id {
            count_q = count_q.bind(rid);
        }
        if let Some(ref pid) = query.parent_thread_id {
            count_q = count_q.bind(pid);
        }
        let (total,): (i64,) = count_q
            .fetch_one(&self.pool)
            .await
            .map_err(Self::sql_err)?;

        // Data query: $1=limit, $2=offset, $3+=filters
        let mut data_clauses = Vec::new();
        let mut data_param_idx = 3;
        if query.resource_id.is_some() {
            data_clauses.push(format!("data->>'resource_id' = ${data_param_idx}"));
            data_param_idx += 1;
        }
        if query.parent_thread_id.is_some() {
            data_clauses.push(format!("data->>'parent_thread_id' = ${data_param_idx}"));
            // data_param_idx += 1;
        }
        let where_data = if data_clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", data_clauses.join(" AND "))
        };
        let data_sql = format!(
            "SELECT id FROM {}{} ORDER BY id LIMIT $1 OFFSET $2",
            self.table, where_data
        );
        let mut data_q = sqlx::query_as::<_, (String,)>(&data_sql)
            .bind(fetch_limit)
            .bind(offset);
        if let Some(ref rid) = query.resource_id {
            data_q = data_q.bind(rid);
        }
        if let Some(ref pid) = query.parent_thread_id {
            data_q = data_q.bind(pid);
        }
        let rows: Vec<(String,)> = data_q
            .fetch_all(&self.pool)
            .await
            .map_err(Self::sql_err)?;

        let has_more = rows.len() > limit;
        let items: Vec<String> = rows.into_iter().take(limit).map(|(id,)| id).collect();

        Ok(ThreadListPage {
            items,
            total: total as usize,
            has_more,
        })
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
        let thread = Thread::new("test-1").with_message(Message::user("Hello"));

        storage.save(&thread).await.unwrap();
        let loaded = storage.load_thread("test-1").await.unwrap();

        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.message_count(), 1);
    }

    #[tokio::test]
    async fn test_memory_storage_load_not_found() {
        let storage = MemoryStorage::new();
        let loaded = storage.load_thread("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_delete() {
        let storage = MemoryStorage::new();
        let thread = Thread::new("test-1");

        storage.save(&thread).await.unwrap();
        assert!(storage.load_thread("test-1").await.unwrap().is_some());

        storage.delete("test-1").await.unwrap();
        assert!(storage.load_thread("test-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_memory_storage_list() {
        let storage = MemoryStorage::new();

        storage.save(&Thread::new("thread-1")).await.unwrap();
        storage.save(&Thread::new("thread-2")).await.unwrap();

        let mut ids = storage.list().await.unwrap();
        ids.sort();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"thread-1".to_string()));
        assert!(ids.contains(&"thread-2".to_string()));
    }

    #[tokio::test]
    async fn test_memory_storage_update_session() {
        let storage = MemoryStorage::new();

        // Save initial session
        let thread = Thread::new("test-1").with_message(Message::user("Hello"));
        storage.save(&thread).await.unwrap();

        // Update session
        let thread = thread.with_message(Message::assistant("Hi!"));
        storage.save(&thread).await.unwrap();

        // Load and verify
        let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
        assert_eq!(loaded.message_count(), 2);
    }

    #[tokio::test]
    async fn test_memory_storage_with_state_and_patches() {
        let storage = MemoryStorage::new();

        let thread = Thread::with_initial_state("test-1", json!({"counter": 0}))
            .with_message(Message::user("Increment"))
            .with_patch(TrackedPatch::new(
                Patch::new().with_op(Op::set(path!("counter"), json!(5))),
            ));

        storage.save(&thread).await.unwrap();

        let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
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
                    let thread = Thread::new(format!("thread-{}", i));
                    storage.save(&thread).await.unwrap();
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

        let thread = Thread::new("test-1").with_message(Message::user("Hello"));
        storage.save(&thread).await.unwrap();

        let loaded = storage.load_thread("test-1").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.message_count(), 1);
    }

    #[tokio::test]
    async fn test_file_storage_load_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let loaded = storage.load_thread("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_file_storage_delete() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let thread = Thread::new("test-1");
        storage.save(&thread).await.unwrap();
        assert!(storage.load_thread("test-1").await.unwrap().is_some());

        storage.delete("test-1").await.unwrap();
        assert!(storage.load_thread("test-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_file_storage_list() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        storage.save(&Thread::new("thread-a")).await.unwrap();
        storage.save(&Thread::new("thread-b")).await.unwrap();
        storage.save(&Thread::new("thread-c")).await.unwrap();

        let mut ids = storage.list().await.unwrap();
        ids.sort();

        assert_eq!(ids.len(), 3);
        assert_eq!(ids, vec!["thread-a", "thread-b", "thread-c"]);
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
        let nested_path = temp_dir.path().join("nested").join("threads");
        let storage = FileStorage::new(&nested_path);

        let thread = Thread::new("test-1");
        storage.save(&thread).await.unwrap();

        assert!(nested_path.exists());
        assert!(storage.load_thread("test-1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_file_storage_with_complex_session() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());

        let thread = Thread::with_initial_state(
            "complex-thread",
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

        storage.save(&thread).await.unwrap();

        let loaded = storage.load_thread("complex-thread").await.unwrap().unwrap();
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
        let thread = Thread::new("test-1").with_message(Message::user("First"));
        storage.save(&thread).await.unwrap();

        // Update
        let thread = thread
            .with_message(Message::assistant("Second"))
            .with_message(Message::user("Third"));
        storage.save(&thread).await.unwrap();

        // Verify
        let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
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
        storage.save(&Thread::new("valid")).await.unwrap();

        let ids = storage.list().await.unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "valid");
    }

    #[test]
    fn test_storage_error_display() {
        let err = StorageError::NotFound("thread-1".to_string());
        assert!(err.to_string().contains("not found"));
        assert!(err.to_string().contains("thread-1"));

        let err = StorageError::Serialization("invalid json".to_string());
        assert!(err.to_string().contains("Serialization"));
    }

    #[test]
    fn test_file_storage_thread_path() {
        let storage = FileStorage::new("/base/path");
        let path = storage.thread_path("my-thread").unwrap();
        assert_eq!(path.to_string_lossy(), "/base/path/my-thread.json");
    }

    #[test]
    fn test_file_storage_rejects_path_traversal() {
        let storage = FileStorage::new("/base/path");
        assert!(storage.thread_path("../../etc/passwd").is_err());
        assert!(storage.thread_path("foo/bar").is_err());
        assert!(storage.thread_path("foo\\bar").is_err());
        assert!(storage.thread_path("").is_err());
        assert!(storage.thread_path("foo\0bar").is_err());
    }

    // ========================================================================
    // Pagination tests
    // ========================================================================

    fn make_messages(n: usize) -> Vec<std::sync::Arc<Message>> {
        (0..n)
            .map(|i| std::sync::Arc::new(Message::user(format!("msg-{}", i))))
            .collect()
    }

    fn make_thread_with_messages(id: &str, n: usize) -> Thread {
        let mut thread = Thread::new(id);
        for msg in make_messages(n) {
            // Deref Arc to get Message for with_message
            thread = thread.with_message((*msg).clone());
        }
        thread
    }

    #[test]
    fn test_paginate_in_memory_basic() {
        let msgs = make_messages(10);
        let query = MessageQuery {
            limit: 3,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.messages[0].cursor, 0);
        assert_eq!(page.messages[1].cursor, 1);
        assert_eq!(page.messages[2].cursor, 2);
        assert_eq!(page.next_cursor, Some(2));
        assert_eq!(page.prev_cursor, Some(0));
    }

    #[test]
    fn test_paginate_in_memory_cursor_forward() {
        let msgs = make_messages(10);
        let query = MessageQuery {
            after: Some(2),
            limit: 3,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.messages[0].cursor, 3);
        assert_eq!(page.messages[1].cursor, 4);
        assert_eq!(page.messages[2].cursor, 5);
    }

    #[test]
    fn test_paginate_in_memory_cursor_backward() {
        let msgs = make_messages(10);
        let query = MessageQuery {
            before: Some(8),
            limit: 3,
            order: SortOrder::Desc,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 3);
        assert!(page.has_more);
        // Desc order: highest cursors first
        assert_eq!(page.messages[0].cursor, 7);
        assert_eq!(page.messages[1].cursor, 6);
        assert_eq!(page.messages[2].cursor, 5);
    }

    #[test]
    fn test_paginate_in_memory_empty() {
        let msgs: Vec<std::sync::Arc<Message>> = Vec::new();
        let query = MessageQuery::default();
        let page = paginate_in_memory(&msgs, &query);

        assert!(page.messages.is_empty());
        assert!(!page.has_more);
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.prev_cursor, None);
    }

    #[test]
    fn test_paginate_in_memory_beyond_end() {
        let msgs = make_messages(5);
        let query = MessageQuery {
            after: Some(10),
            limit: 3,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert!(page.messages.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn test_paginate_in_memory_exact_fit() {
        let msgs = make_messages(3);
        let query = MessageQuery {
            limit: 3,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 3);
        assert!(!page.has_more);
    }

    #[test]
    fn test_paginate_in_memory_last_page() {
        let msgs = make_messages(10);
        let query = MessageQuery {
            after: Some(7),
            limit: 5,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 2);
        assert!(!page.has_more);
        assert_eq!(page.messages[0].cursor, 8);
        assert_eq!(page.messages[1].cursor, 9);
    }

    #[test]
    fn test_paginate_in_memory_after_and_before() {
        let msgs = make_messages(10);
        let query = MessageQuery {
            after: Some(2),
            before: Some(7),
            limit: 10,
            ..Default::default()
        };
        let page = paginate_in_memory(&msgs, &query);

        assert_eq!(page.messages.len(), 4);
        assert!(!page.has_more);
        assert_eq!(page.messages[0].cursor, 3);
        assert_eq!(page.messages[3].cursor, 6);
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages() {
        let storage = MemoryStorage::new();
        let thread = make_thread_with_messages("test-1", 10);
        storage.save(&thread).await.unwrap();

        let query = MessageQuery {
            limit: 3,
            ..Default::default()
        };
        let page = ThreadQuery::load_messages(&storage, "test-1", &query).await.unwrap();

        assert_eq!(page.messages.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.messages[0].message.content, "msg-0");
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages_not_found() {
        let storage = MemoryStorage::new();
        let query = MessageQuery::default();
        let result = ThreadQuery::load_messages(&storage, "nonexistent", &query).await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_memory_storage_message_count() {
        let storage = MemoryStorage::new();
        let thread = make_thread_with_messages("test-1", 7);
        storage.save(&thread).await.unwrap();

        let count = storage.message_count("test-1").await.unwrap();
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn test_memory_storage_message_count_not_found() {
        let storage = MemoryStorage::new();
        let result = storage.message_count("nonexistent").await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_file_storage_load_messages() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let thread = make_thread_with_messages("test-1", 10);
        storage.save(&thread).await.unwrap();

        let query = MessageQuery {
            after: Some(4),
            limit: 3,
            ..Default::default()
        };
        let page = ThreadQuery::load_messages(&storage, "test-1", &query).await.unwrap();

        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[0].cursor, 5);
        assert_eq!(page.messages[0].message.content, "msg-5");
    }

    #[tokio::test]
    async fn test_file_storage_message_count() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let thread = make_thread_with_messages("test-1", 5);
        storage.save(&thread).await.unwrap();

        let count = storage.message_count("test-1").await.unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn test_message_page_serialization() {
        let page = MessagePage {
            messages: vec![MessageWithCursor {
                cursor: 0,
                message: Message::user("hello"),
            }],
            has_more: true,
            next_cursor: Some(0),
            prev_cursor: Some(0),
        };
        let json = serde_json::to_string(&page).unwrap();
        assert!(json.contains("\"cursor\":0"));
        assert!(json.contains("\"has_more\":true"));
        assert!(json.contains("\"next_cursor\":0"));
    }

    // ========================================================================
    // Visibility tests
    // ========================================================================

    fn make_mixed_visibility_thread(id: &str) -> Thread {
        Thread::new(id)
            .with_message(Message::user("user-0"))
            .with_message(Message::assistant("assistant-1"))
            .with_message(Message::internal_system("reminder-2"))
            .with_message(Message::user("user-3"))
            .with_message(Message::internal_system("reminder-4"))
            .with_message(Message::assistant("assistant-5"))
    }

    #[test]
    fn test_paginate_visibility_all_default() {
        let thread = make_mixed_visibility_thread("t");
        // Default query filters to Visibility::All (user-visible only).
        let query = MessageQuery {
            limit: 50,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        // Should exclude 2 internal messages (at indices 2 and 4).
        assert_eq!(page.messages.len(), 4);
        assert_eq!(page.messages[0].message.content, "user-0");
        assert_eq!(page.messages[1].message.content, "assistant-1");
        assert_eq!(page.messages[2].message.content, "user-3");
        assert_eq!(page.messages[3].message.content, "assistant-5");
    }

    #[test]
    fn test_paginate_visibility_internal_only() {
        let thread = make_mixed_visibility_thread("t");
        let query = MessageQuery {
            limit: 50,
            visibility: Some(Visibility::Internal),
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].message.content, "reminder-2");
        assert_eq!(page.messages[1].message.content, "reminder-4");
    }

    #[test]
    fn test_paginate_visibility_none_returns_all() {
        let thread = make_mixed_visibility_thread("t");
        let query = MessageQuery {
            limit: 50,
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        // Should return all 6 messages.
        assert_eq!(page.messages.len(), 6);
    }

    #[test]
    fn test_paginate_visibility_cursors_stable() {
        let thread = make_mixed_visibility_thread("t");
        // With visibility=All, cursors should correspond to original indices.
        let query = MessageQuery {
            limit: 50,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        // Cursors should be 0, 1, 3, 5 (original indices, skipping internal at 2, 4).
        assert_eq!(page.messages[0].cursor, 0);
        assert_eq!(page.messages[1].cursor, 1);
        assert_eq!(page.messages[2].cursor, 3);
        assert_eq!(page.messages[3].cursor, 5);
    }

    #[test]
    fn test_paginate_visibility_with_cursor_pagination() {
        let thread = make_mixed_visibility_thread("t");
        // Start after cursor 1 (assistant-1), visibility=All.
        let query = MessageQuery {
            after: Some(1),
            limit: 2,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        // Should return user-3 (cursor 3) and assistant-5 (cursor 5).
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].cursor, 3);
        assert_eq!(page.messages[0].message.content, "user-3");
        assert_eq!(page.messages[1].cursor, 5);
        assert_eq!(page.messages[1].message.content, "assistant-5");
        assert!(!page.has_more);
    }

    #[test]
    fn test_internal_system_message_serialization() {
        let msg = Message::internal_system("a reminder");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"visibility\":\"internal\""));

        // Round-trip
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.visibility, Visibility::Internal);
        assert_eq!(parsed.content, "a reminder");
    }

    #[test]
    fn test_user_message_omits_visibility_in_json() {
        let msg = Message::user("hello");
        let json = serde_json::to_string(&msg).unwrap();
        // Default visibility should be skipped in serialization.
        assert!(!json.contains("visibility"));

        // Deserializing without visibility field should default to All.
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.visibility, Visibility::All);
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages_filters_visibility() {
        let storage = MemoryStorage::new();
        let thread = make_mixed_visibility_thread("test-vis");
        storage.save(&thread).await.unwrap();

        // Default query (visibility = All)
        let page = ThreadQuery::load_messages(&storage, "test-vis", &MessageQuery::default())
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 4);

        // visibility = None (all messages)
        let query = MessageQuery {
            visibility: None,
            ..Default::default()
        };
        let page = ThreadQuery::load_messages(&storage, "test-vis", &query).await.unwrap();
        assert_eq!(page.messages.len(), 6);
    }

    // ========================================================================
    // Run ID filtering tests
    // ========================================================================

    use crate::types::MessageMetadata;

    fn make_multi_run_thread(id: &str) -> Thread {
        Thread::new(id)
            // User message (no run metadata)
            .with_message(Message::user("hello"))
            // Run A, step 0: assistant + tool
            .with_message(
                Message::assistant("thinking...").with_metadata(MessageMetadata {
                    run_id: Some("run-a".to_string()),
                    step_index: Some(0),
                }),
            )
            .with_message(
                Message::tool("tc1", "result").with_metadata(MessageMetadata {
                    run_id: Some("run-a".to_string()),
                    step_index: Some(0),
                }),
            )
            // Run A, step 1: assistant final
            .with_message(Message::assistant("done").with_metadata(MessageMetadata {
                run_id: Some("run-a".to_string()),
                step_index: Some(1),
            }))
            // User follow-up (no run metadata)
            .with_message(Message::user("more"))
            // Run B, step 0
            .with_message(Message::assistant("ok").with_metadata(MessageMetadata {
                run_id: Some("run-b".to_string()),
                step_index: Some(0),
            }))
    }

    #[test]
    fn test_paginate_filter_by_run_id() {
        let thread = make_multi_run_thread("t");

        // Filter to run-a only
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);
        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[0].message.content, "thinking...");
        assert_eq!(page.messages[2].message.content, "done");

        // Filter to run-b only
        let query = MessageQuery {
            run_id: Some("run-b".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].message.content, "ok");

        // No run filter: returns all 6
        let query = MessageQuery {
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);
        assert_eq!(page.messages.len(), 6);
    }

    #[test]
    fn test_paginate_run_id_with_cursor() {
        let thread = make_multi_run_thread("t");

        // Filter run-a, after cursor 1 (skip first run-a msg at cursor 1)
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            after: Some(1),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);
        assert_eq!(page.messages.len(), 2); // tool (cursor 2) + final (cursor 3)
    }

    #[test]
    fn test_paginate_nonexistent_run_id() {
        let thread = make_multi_run_thread("t");
        let query = MessageQuery {
            run_id: Some("run-z".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);
        assert!(page.messages.is_empty());
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages_by_run_id() {
        let storage = MemoryStorage::new();
        let thread = make_multi_run_thread("test-run");
        storage.save(&thread).await.unwrap();

        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = ThreadQuery::load_messages(&storage, "test-run", &query).await.unwrap();
        assert_eq!(page.messages.len(), 3);
    }

    #[test]
    fn test_message_metadata_preserved_in_pagination() {
        let thread = make_multi_run_thread("t");
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&thread.messages, &query);

        // Metadata should be preserved on the returned messages.
        let meta = page.messages[0].message.metadata.as_ref().unwrap();
        assert_eq!(meta.run_id.as_deref(), Some("run-a"));
        assert_eq!(meta.step_index, Some(0));
    }

    // ========================================================================
    // Thread list pagination tests
    // ========================================================================

    #[tokio::test]
    async fn test_list_paginated_default() {
        let storage = MemoryStorage::new();
        for i in 0..5 {
            storage
                .save(&Thread::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&ThreadListQuery::default())
            .await
            .unwrap();
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.total, 5);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn test_list_paginated_with_limit() {
        let storage = MemoryStorage::new();
        for i in 0..10 {
            storage
                .save(&Thread::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&ThreadListQuery {
                offset: 0,
                limit: 3,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.total, 10);
        assert!(page.has_more);
        // Items should be sorted.
        assert_eq!(page.items, vec!["s-00", "s-01", "s-02"]);
    }

    #[tokio::test]
    async fn test_list_paginated_with_offset() {
        let storage = MemoryStorage::new();
        for i in 0..5 {
            storage
                .save(&Thread::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&ThreadListQuery {
                offset: 3,
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 5);
        assert!(!page.has_more);
        assert_eq!(page.items, vec!["s-03", "s-04"]);
    }

    #[tokio::test]
    async fn test_list_paginated_offset_beyond_total() {
        let storage = MemoryStorage::new();
        for i in 0..3 {
            storage
                .save(&Thread::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&ThreadListQuery {
                offset: 100,
                limit: 10,
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.total, 3);
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn test_list_paginated_empty() {
        let storage = MemoryStorage::new();
        let page = storage
            .list_paginated(&ThreadListQuery::default())
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.total, 0);
        assert!(!page.has_more);
    }

    // ========================================================================
    // ThreadStore / ThreadQuery / ThreadSync tests
    // ========================================================================

    fn sample_delta(run_id: &str, reason: CheckpointReason) -> ThreadDelta {
        ThreadDelta {
            run_id: run_id.to_string(),
            parent_run_id: None,
            reason,
            messages: vec![Arc::new(Message::assistant("hello"))],
            patches: vec![],
            snapshot: None,
        }
    }

    #[tokio::test]
    async fn test_thread_store_create_and_load() {
        let store = MemoryStorage::new();
        let thread = Thread::new("t1").with_message(Message::user("hi"));
        let committed = store.create(&thread).await.unwrap();
        assert_eq!(committed.version, 0);

        let head = ThreadStore::load(&store, "t1").await.unwrap().unwrap();
        assert_eq!(head.version, 0);
        assert_eq!(head.thread.id, "t1");
        assert_eq!(head.thread.message_count(), 1);
    }

    #[tokio::test]
    async fn test_thread_store_create_already_exists() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();
        let err = store.create(&Thread::new("t1")).await.unwrap_err();
        assert!(matches!(err, StorageError::AlreadyExists));
    }

    #[tokio::test]
    async fn test_thread_store_append() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();

        let delta = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
        let committed = store.append("t1", 0, &delta).await.unwrap();
        assert_eq!(committed.version, 1);

        let head = ThreadStore::load(&store, "t1").await.unwrap().unwrap();
        assert_eq!(head.version, 1);
        assert_eq!(head.thread.message_count(), 1); // from delta
    }

    #[tokio::test]
    async fn test_thread_store_append_version_conflict() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();

        let delta = sample_delta("run-1", CheckpointReason::UserMessage);
        store.append("t1", 0, &delta).await.unwrap(); // version -> 1

        // Try to append with stale version
        let err = store.append("t1", 0, &delta).await.unwrap_err();
        assert!(matches!(
            err,
            StorageError::VersionConflict {
                expected: 0,
                actual: 1
            }
        ));
    }

    #[tokio::test]
    async fn test_thread_store_append_not_found() {
        let store = MemoryStorage::new();
        let delta = sample_delta("run-1", CheckpointReason::RunFinished);
        let err = store.append("missing", 0, &delta).await.unwrap_err();
        assert!(matches!(err, StorageError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_thread_store_delete() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();
        ThreadStore::delete(&store, "t1").await.unwrap();
        assert!(ThreadStore::load(&store, "t1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_thread_store_append_with_snapshot() {
        let store = MemoryStorage::new();
        let thread = Thread::with_initial_state("t1", json!({"counter": 0}));
        store.create(&thread).await.unwrap();

        let delta = ThreadDelta {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::RunFinished,
            messages: vec![],
            patches: vec![],
            snapshot: Some(json!({"counter": 42})),
        };
        store.append("t1", 0, &delta).await.unwrap();

        let head = ThreadStore::load(&store, "t1").await.unwrap().unwrap();
        assert_eq!(head.thread.state, json!({"counter": 42}));
        assert!(head.thread.patches.is_empty());
    }

    #[tokio::test]
    async fn test_thread_sync_load_deltas() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();

        let d1 = sample_delta("run-1", CheckpointReason::UserMessage);
        let d2 = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
        let d3 = sample_delta("run-1", CheckpointReason::RunFinished);
        store.append("t1", 0, &d1).await.unwrap();
        store.append("t1", 1, &d2).await.unwrap();
        store.append("t1", 2, &d3).await.unwrap();

        // All deltas
        let deltas = store.load_deltas("t1", 0).await.unwrap();
        assert_eq!(deltas.len(), 3);

        // Deltas after version 1
        let deltas = store.load_deltas("t1", 1).await.unwrap();
        assert_eq!(deltas.len(), 2);

        // Deltas after version 3 (none)
        let deltas = store.load_deltas("t1", 3).await.unwrap();
        assert_eq!(deltas.len(), 0);
    }

    #[tokio::test]
    async fn test_thread_query_list_threads() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("t1")).await.unwrap();
        store.create(&Thread::new("t2")).await.unwrap();

        let page = store.list_threads(&ThreadListQuery::default()).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 2);
    }

    #[tokio::test]
    async fn test_thread_query_list_threads_by_parent() {
        let store = MemoryStorage::new();
        store.create(&Thread::new("parent")).await.unwrap();
        store
            .create(&Thread::new("child-1").with_parent_thread_id("parent"))
            .await
            .unwrap();
        store
            .create(&Thread::new("child-2").with_parent_thread_id("parent"))
            .await
            .unwrap();
        store
            .create(&Thread::new("unrelated"))
            .await
            .unwrap();

        let query = ThreadListQuery {
            parent_thread_id: Some("parent".to_string()),
            ..Default::default()
        };
        let page = store.list_threads(&query).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.items.contains(&"child-1".to_string()));
        assert!(page.items.contains(&"child-2".to_string()));
    }

    #[tokio::test]
    async fn test_thread_query_load_messages() {
        let store = MemoryStorage::new();
        let thread = Thread::new("t1")
            .with_message(Message::user("hello"))
            .with_message(Message::assistant("hi"));
        store.create(&thread).await.unwrap();

        let page = ThreadQuery::load_messages(
            &store,
            "t1",
            &MessageQuery {
                limit: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page.messages.len(), 1);
        assert!(page.has_more);
    }

    #[tokio::test]
    async fn test_parent_thread_id_persisted() {
        let thread = Thread::new("child-1").with_parent_thread_id("parent-1");
        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(json_str.contains("parent_thread_id"));

        let restored: Thread = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.parent_thread_id.as_deref(), Some("parent-1"));
    }

    #[tokio::test]
    async fn test_parent_thread_id_none_omitted() {
        let thread = Thread::new("t1");
        let json_str = serde_json::to_string(&thread).unwrap();
        assert!(!json_str.contains("parent_thread_id"));
    }

    // ========================================================================
    // FileStorage ThreadStore tests
    // ========================================================================

    #[tokio::test]
    async fn test_file_thread_store_create_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStorage::new(temp_dir.path());
        let thread = Thread::new("t1").with_message(Message::user("hi"));
        let committed = store.create(&thread).await.unwrap();
        assert_eq!(committed.version, 0);

        let head = ThreadStore::load(&store, "t1").await.unwrap().unwrap();
        assert_eq!(head.version, 0);
        assert_eq!(head.thread.message_count(), 1);
    }

    #[tokio::test]
    async fn test_file_thread_store_append_and_version() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStorage::new(temp_dir.path());
        store.create(&Thread::new("t1")).await.unwrap();

        let delta = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
        let committed = store.append("t1", 0, &delta).await.unwrap();
        assert_eq!(committed.version, 1);

        let head = ThreadStore::load(&store, "t1").await.unwrap().unwrap();
        assert_eq!(head.version, 1);
        assert_eq!(head.thread.message_count(), 1);
    }

    #[tokio::test]
    async fn test_file_thread_store_version_conflict() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStorage::new(temp_dir.path());
        store.create(&Thread::new("t1")).await.unwrap();

        let delta = sample_delta("run-1", CheckpointReason::UserMessage);
        store.append("t1", 0, &delta).await.unwrap();

        let err = store.append("t1", 0, &delta).await.unwrap_err();
        assert!(matches!(err, StorageError::VersionConflict { .. }));
    }

    #[tokio::test]
    async fn test_file_thread_store_already_exists() {
        let temp_dir = TempDir::new().unwrap();
        let store = FileStorage::new(temp_dir.path());
        store.create(&Thread::new("t1")).await.unwrap();
        let err = store.create(&Thread::new("t1")).await.unwrap_err();
        assert!(matches!(err, StorageError::AlreadyExists));
    }
}
