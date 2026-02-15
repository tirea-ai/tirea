use super::*;

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
pub fn paginate_in_memory(
    messages: &[std::sync::Arc<Message>],
    query: &MessageQuery,
) -> MessagePage {
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
pub enum ThreadStoreError {
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

impl ThreadDelta {
    /// Apply this delta to a thread in place.
    pub fn apply_to(&self, thread: &mut Thread) {
        thread.messages.extend(self.messages.iter().cloned());
        thread.patches.extend(self.patches.iter().cloned());
        if let Some(ref snapshot) = self.snapshot {
            thread.state = snapshot.clone();
            thread.patches.clear();
        }
    }
}
