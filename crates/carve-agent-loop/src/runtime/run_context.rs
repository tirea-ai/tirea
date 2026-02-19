use crate::contracts::state::ThreadChangeSet;
use crate::contracts::storage::VersionPrecondition;
use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub type RunCancellationToken = CancellationToken;

/// Error returned by state commit sinks.
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct StateCommitError {
    pub message: String,
}

impl StateCommitError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Sink for committed thread deltas.
#[async_trait]
pub trait StateCommitter: Send + Sync {
    /// Commit a single change set for a thread.
    ///
    /// Returns the committed storage version after the write succeeds.
    async fn commit(
        &self,
        thread_id: &str,
        changeset: ThreadChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<u64, StateCommitError>;
}

impl std::fmt::Debug for dyn StateCommitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<StateCommitter>")
    }
}

/// Scope key: caller session id visible to tools.
pub const TOOL_SCOPE_CALLER_THREAD_ID_KEY: &str = "__agent_tool_caller_thread_id";
/// Scope key: caller agent id visible to tools.
pub const TOOL_SCOPE_CALLER_AGENT_ID_KEY: &str = "__agent_tool_caller_agent_id";
/// Scope key: caller state snapshot visible to tools.
pub const TOOL_SCOPE_CALLER_STATE_KEY: &str = "__agent_tool_caller_state";
/// Scope key: caller message snapshot visible to tools.
pub const TOOL_SCOPE_CALLER_MESSAGES_KEY: &str = "__agent_tool_caller_messages";
