use crate::state::AgentState;
use crate::state::Version;
use crate::Message;
use crate::Visibility;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub visibility: Option<Visibility>,
    /// Filter by run ID. `None` means return all runs.
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
    /// Cursor of the last item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    /// Cursor of the first item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_cursor: Option<i64>,
}

/// Pagination query for AgentState lists.
#[derive(Debug, Clone)]
pub struct AgentStateListQuery {
    /// Number of items to skip (0-based).
    pub offset: usize,
    /// Maximum number of items to return (clamped to 1..=200).
    pub limit: usize,
    /// Filter by resource_id (owner). `None` means no filtering.
    pub resource_id: Option<String>,
    /// Filter by parent_thread_id. `None` means no filtering.
    pub parent_thread_id: Option<String>,
}

impl Default for AgentStateListQuery {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 50,
            resource_id: None,
            parent_thread_id: None,
        }
    }
}

/// Paginated AgentState list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStateListPage {
    pub items: Vec<String>,
    pub total: usize,
    pub has_more: bool,
}

/// Cursor-based in-memory pagination helper.
pub fn paginate_in_memory(
    messages: &[std::sync::Arc<Message>],
    query: &MessageQuery,
) -> MessagePage {
    let limit = query.limit.clamp(1, 200);
    let total = messages.len();
    if total == 0 {
        return MessagePage {
            messages: Vec::new(),
            has_more: false,
            next_cursor: None,
            prev_cursor: None,
        };
    }

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

    let has_more = items.len() > limit;
    let limited: Vec<_> = items.into_iter().take(limit).collect();

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

/// Storage-level errors.
#[derive(Debug, Error)]
pub enum AgentStateStoreError {
    /// AgentState not found.
    #[error("AgentState not found: {0}")]
    NotFound(String),

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Invalid AgentState id (path traversal, control chars, etc.).
    #[error("Invalid thread id: {0}")]
    InvalidId(String),

    /// AgentState already exists.
    #[error("AgentState already exists")]
    AlreadyExists,

    /// Optimistic concurrency check failed.
    #[error("Version conflict: expected {expected}, actual {actual}")]
    VersionConflict { expected: Version, actual: Version },
}

/// Version check policy for append operations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionPrecondition {
    /// Skip version check before commit.
    #[default]
    Any,
    /// Require an exact version match before commit.
    Exact(Version),
}

/// Commit acknowledgement returned after successful write.
#[derive(Debug, Clone, Copy)]
pub struct Committed {
    pub version: Version,
}

/// AgentState plus current storage version.
#[derive(Debug, Clone)]
pub struct AgentStateHead {
    pub agent_state: AgentState,
    pub version: Version,
}
