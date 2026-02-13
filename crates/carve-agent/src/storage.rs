//! Storage backend trait for session persistence.

use crate::session::Session;
use crate::types::{Message, Visibility};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
#[cfg(feature = "postgres")]
use std::collections::HashSet;
use std::path::PathBuf;
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
pub struct SessionListQuery {
    /// Number of items to skip (0-based).
    pub offset: usize,
    /// Maximum number of items to return (clamped to 1..=200).
    pub limit: usize,
    /// Filter by resource_id (owner). `None` means no filtering.
    pub resource_id: Option<String>,
}

impl Default for SessionListQuery {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            resource_id: None,
        }
    }
}

/// Paginated session list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListPage {
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

    /// Load a paginated slice of messages for a session.
    ///
    /// Default implementation loads the full session and paginates in-memory.
    async fn load_messages(
        &self,
        session_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, StorageError> {
        let session = self
            .load(session_id)
            .await?
            .ok_or_else(|| StorageError::NotFound(session_id.to_string()))?;
        Ok(paginate_in_memory(&session.messages, query))
    }

    /// Get total message count for a session.
    ///
    /// Default implementation loads the full session and returns message count.
    async fn message_count(&self, session_id: &str) -> Result<usize, StorageError> {
        let session = self
            .load(session_id)
            .await?
            .ok_or_else(|| StorageError::NotFound(session_id.to_string()))?;
        Ok(session.messages.len())
    }

    /// List sessions with pagination.
    ///
    /// Default implementation calls `list()` and paginates in-memory.
    async fn list_paginated(
        &self,
        query: &SessionListQuery,
    ) -> Result<SessionListPage, StorageError> {
        let mut all = self.list().await?;

        // Filter by resource_id if specified.
        if let Some(ref resource_id) = query.resource_id {
            let mut filtered = Vec::new();
            for id in &all {
                if let Some(session) = self.load(id).await? {
                    if session.resource_id.as_deref() == Some(resource_id.as_str()) {
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
        Ok(SessionListPage {
            items,
            total,
            has_more,
        })
    }
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
        let tmp_path = self.base_path.join(format!(
            ".{}.{}.tmp",
            session.id,
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
            CREATE TABLE IF NOT EXISTS {sessions} (
                id         TEXT PRIMARY KEY,
                data       JSONB NOT NULL,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE TABLE IF NOT EXISTS {messages} (
                seq        BIGSERIAL PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES {sessions}(id) ON DELETE CASCADE,
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
            sessions = self.table,
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
impl Storage for PostgresStorage {
    async fn load(&self, id: &str) -> Result<Option<Session>, StorageError> {
        // Load session skeleton (data has messages: []).
        let sql = format!("SELECT data FROM {} WHERE id = $1", self.table);
        let row: Option<(serde_json::Value,)> = sqlx::query_as(&sql)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;

        let Some((mut v,)) = row else {
            return Ok(None);
        };

        // Load messages from separate table and inject into session JSON.
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
        }

        let session: Session =
            serde_json::from_value(v).map_err(|e| StorageError::Serialization(e.to_string()))?;
        Ok(Some(session))
    }

    async fn save(&self, session: &Session) -> Result<(), StorageError> {
        // Serialize session skeleton (without messages).
        let mut v = serde_json::to_value(session)
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
            .bind(&session.id)
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
            .bind(&session.id)
            .fetch_all(&mut *tx)
            .await
            .map_err(Self::sql_err)?;
        let existing_ids: HashSet<String> = existing_rows.into_iter().map(|(id,)| id).collect();

        let new_messages: Vec<&Message> = session
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
                .bind(&session.id)
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

    async fn list(&self) -> Result<Vec<String>, StorageError> {
        let sql = format!("SELECT id FROM {} ORDER BY id", self.table);
        let rows: Vec<(String,)> = sqlx::query_as(&sql)
            .fetch_all(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    async fn load_messages(
        &self,
        session_id: &str,
        query: &MessageQuery,
    ) -> Result<MessagePage, StorageError> {
        // Check session exists.
        let exists_sql = format!("SELECT 1 FROM {} WHERE id = $1", self.table);
        let exists: Option<(i32,)> = sqlx::query_as(&exists_sql)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        if exists.is_none() {
            return Err(StorageError::NotFound(session_id.to_string()));
        }

        let limit = query.limit.min(200).max(1);
        // Fetch limit+1 rows to determine has_more.
        let fetch_limit = (limit + 1) as i64;

        // Visibility filter on JSONB data.
        // Messages without a "visibility" field default to "all".
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

        // Build query with dynamic param offsets.
        // Params: $1=session_id, $2=cursor, $3=limit, $4=run_id (opt), $N=before/after (opt).
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
                    .bind(session_id)
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
                    .bind(session_id)
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

    async fn message_count(&self, session_id: &str) -> Result<usize, StorageError> {
        // Check session exists.
        let exists_sql = format!("SELECT 1 FROM {} WHERE id = $1", self.table);
        let exists: Option<(i32,)> = sqlx::query_as(&exists_sql)
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        if exists.is_none() {
            return Err(StorageError::NotFound(session_id.to_string()));
        }

        let sql = format!(
            "SELECT COUNT(*)::bigint FROM {} WHERE session_id = $1",
            self.messages_table
        );
        let row: (i64,) = sqlx::query_as(&sql)
            .bind(session_id)
            .fetch_one(&self.pool)
            .await
            .map_err(Self::sql_err)?;
        Ok(row.0 as usize)
    }

    async fn list_paginated(
        &self,
        query: &SessionListQuery,
    ) -> Result<SessionListPage, StorageError> {
        let limit = query.limit.clamp(1, 200);
        let fetch_limit = (limit + 1) as i64;
        let offset = query.offset as i64;

        // resource_id filter via JSONB.
        let resource_clause = if query.resource_id.is_some() {
            " WHERE data->>'resource_id' = $3"
        } else {
            ""
        };

        let count_sql = format!(
            "SELECT COUNT(*)::bigint FROM {}{}",
            self.table, resource_clause
        );
        let total: i64 = if let Some(ref rid) = query.resource_id {
            let (total,): (i64,) = sqlx::query_as(&count_sql)
                .bind(fetch_limit)
                .bind(offset)
                .bind(rid)
                .fetch_one(&self.pool)
                .await
                .map_err(Self::sql_err)?;
            total
        } else {
            let (total,): (i64,) = sqlx::query_as(&count_sql)
                .fetch_one(&self.pool)
                .await
                .map_err(Self::sql_err)?;
            total
        };

        let sql = format!(
            "SELECT id FROM {}{} ORDER BY id LIMIT $1 OFFSET $2",
            self.table, resource_clause
        );
        let rows: Vec<(String,)> = if let Some(ref rid) = query.resource_id {
            sqlx::query_as(&sql)
                .bind(fetch_limit)
                .bind(offset)
                .bind(rid)
                .fetch_all(&self.pool)
                .await
                .map_err(Self::sql_err)?
        } else {
            sqlx::query_as(&sql)
                .bind(fetch_limit)
                .bind(offset)
                .fetch_all(&self.pool)
                .await
                .map_err(Self::sql_err)?
        };

        let has_more = rows.len() > limit;
        let items: Vec<String> = rows.into_iter().take(limit).map(|(id,)| id).collect();

        Ok(SessionListPage {
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

    // ========================================================================
    // Pagination tests
    // ========================================================================

    fn make_messages(n: usize) -> Vec<std::sync::Arc<Message>> {
        (0..n)
            .map(|i| std::sync::Arc::new(Message::user(format!("msg-{}", i))))
            .collect()
    }

    fn make_session_with_messages(id: &str, n: usize) -> Session {
        let mut session = Session::new(id);
        for msg in make_messages(n) {
            // Deref Arc to get Message for with_message
            session = session.with_message((*msg).clone());
        }
        session
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
        let session = make_session_with_messages("test-1", 10);
        storage.save(&session).await.unwrap();

        let query = MessageQuery {
            limit: 3,
            ..Default::default()
        };
        let page = storage.load_messages("test-1", &query).await.unwrap();

        assert_eq!(page.messages.len(), 3);
        assert!(page.has_more);
        assert_eq!(page.messages[0].message.content, "msg-0");
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages_not_found() {
        let storage = MemoryStorage::new();
        let query = MessageQuery::default();
        let result = storage.load_messages("nonexistent", &query).await;
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_memory_storage_message_count() {
        let storage = MemoryStorage::new();
        let session = make_session_with_messages("test-1", 7);
        storage.save(&session).await.unwrap();

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
        let session = make_session_with_messages("test-1", 10);
        storage.save(&session).await.unwrap();

        let query = MessageQuery {
            after: Some(4),
            limit: 3,
            ..Default::default()
        };
        let page = storage.load_messages("test-1", &query).await.unwrap();

        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[0].cursor, 5);
        assert_eq!(page.messages[0].message.content, "msg-5");
    }

    #[tokio::test]
    async fn test_file_storage_message_count() {
        let temp_dir = TempDir::new().unwrap();
        let storage = FileStorage::new(temp_dir.path());
        let session = make_session_with_messages("test-1", 5);
        storage.save(&session).await.unwrap();

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

    fn make_mixed_visibility_session(id: &str) -> Session {
        Session::new(id)
            .with_message(Message::user("user-0"))
            .with_message(Message::assistant("assistant-1"))
            .with_message(Message::internal_system("reminder-2"))
            .with_message(Message::user("user-3"))
            .with_message(Message::internal_system("reminder-4"))
            .with_message(Message::assistant("assistant-5"))
    }

    #[test]
    fn test_paginate_visibility_all_default() {
        let session = make_mixed_visibility_session("t");
        // Default query filters to Visibility::All (user-visible only).
        let query = MessageQuery {
            limit: 50,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

        // Should exclude 2 internal messages (at indices 2 and 4).
        assert_eq!(page.messages.len(), 4);
        assert_eq!(page.messages[0].message.content, "user-0");
        assert_eq!(page.messages[1].message.content, "assistant-1");
        assert_eq!(page.messages[2].message.content, "user-3");
        assert_eq!(page.messages[3].message.content, "assistant-5");
    }

    #[test]
    fn test_paginate_visibility_internal_only() {
        let session = make_mixed_visibility_session("t");
        let query = MessageQuery {
            limit: 50,
            visibility: Some(Visibility::Internal),
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].message.content, "reminder-2");
        assert_eq!(page.messages[1].message.content, "reminder-4");
    }

    #[test]
    fn test_paginate_visibility_none_returns_all() {
        let session = make_mixed_visibility_session("t");
        let query = MessageQuery {
            limit: 50,
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

        // Should return all 6 messages.
        assert_eq!(page.messages.len(), 6);
    }

    #[test]
    fn test_paginate_visibility_cursors_stable() {
        let session = make_mixed_visibility_session("t");
        // With visibility=All, cursors should correspond to original indices.
        let query = MessageQuery {
            limit: 50,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

        // Cursors should be 0, 1, 3, 5 (original indices, skipping internal at 2, 4).
        assert_eq!(page.messages[0].cursor, 0);
        assert_eq!(page.messages[1].cursor, 1);
        assert_eq!(page.messages[2].cursor, 3);
        assert_eq!(page.messages[3].cursor, 5);
    }

    #[test]
    fn test_paginate_visibility_with_cursor_pagination() {
        let session = make_mixed_visibility_session("t");
        // Start after cursor 1 (assistant-1), visibility=All.
        let query = MessageQuery {
            after: Some(1),
            limit: 2,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

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
        let session = make_mixed_visibility_session("test-vis");
        storage.save(&session).await.unwrap();

        // Default query (visibility = All)
        let page = storage
            .load_messages("test-vis", &MessageQuery::default())
            .await
            .unwrap();
        assert_eq!(page.messages.len(), 4);

        // visibility = None (all messages)
        let query = MessageQuery {
            visibility: None,
            ..Default::default()
        };
        let page = storage.load_messages("test-vis", &query).await.unwrap();
        assert_eq!(page.messages.len(), 6);
    }

    // ========================================================================
    // Run ID filtering tests
    // ========================================================================

    use crate::types::MessageMetadata;

    fn make_multi_run_session(id: &str) -> Session {
        Session::new(id)
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
        let session = make_multi_run_session("t");

        // Filter to run-a only
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);
        assert_eq!(page.messages.len(), 3);
        assert_eq!(page.messages[0].message.content, "thinking...");
        assert_eq!(page.messages[2].message.content, "done");

        // Filter to run-b only
        let query = MessageQuery {
            run_id: Some("run-b".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].message.content, "ok");

        // No run filter: returns all 6
        let query = MessageQuery {
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);
        assert_eq!(page.messages.len(), 6);
    }

    #[test]
    fn test_paginate_run_id_with_cursor() {
        let session = make_multi_run_session("t");

        // Filter run-a, after cursor 1 (skip first run-a msg at cursor 1)
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            after: Some(1),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);
        assert_eq!(page.messages.len(), 2); // tool (cursor 2) + final (cursor 3)
    }

    #[test]
    fn test_paginate_nonexistent_run_id() {
        let session = make_multi_run_session("t");
        let query = MessageQuery {
            run_id: Some("run-z".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);
        assert!(page.messages.is_empty());
    }

    #[tokio::test]
    async fn test_memory_storage_load_messages_by_run_id() {
        let storage = MemoryStorage::new();
        let session = make_multi_run_session("test-run");
        storage.save(&session).await.unwrap();

        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = storage.load_messages("test-run", &query).await.unwrap();
        assert_eq!(page.messages.len(), 3);
    }

    #[test]
    fn test_message_metadata_preserved_in_pagination() {
        let session = make_multi_run_session("t");
        let query = MessageQuery {
            run_id: Some("run-a".to_string()),
            visibility: None,
            ..Default::default()
        };
        let page = paginate_in_memory(&session.messages, &query);

        // Metadata should be preserved on the returned messages.
        let meta = page.messages[0].message.metadata.as_ref().unwrap();
        assert_eq!(meta.run_id.as_deref(), Some("run-a"));
        assert_eq!(meta.step_index, Some(0));
    }

    // ========================================================================
    // Session list pagination tests
    // ========================================================================

    #[tokio::test]
    async fn test_list_paginated_default() {
        let storage = MemoryStorage::new();
        for i in 0..5 {
            storage
                .save(&Session::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&SessionListQuery::default())
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
                .save(&Session::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&SessionListQuery {
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
                .save(&Session::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&SessionListQuery {
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
                .save(&Session::new(format!("s-{i:02}")))
                .await
                .unwrap();
        }
        let page = storage
            .list_paginated(&SessionListQuery {
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
            .list_paginated(&SessionListQuery::default())
            .await
            .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.total, 0);
        assert!(!page.has_more);
    }
}
