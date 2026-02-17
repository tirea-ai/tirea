use crate::contracts::state::AgentChangeSet;
use crate::contracts::storage::VersionPrecondition;
use async_trait::async_trait;
use std::sync::Arc;
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
        changeset: AgentChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<u64, StateCommitError>;
}

/// Optional lifecycle context for a streaming agent run.
///
/// Run-specific data (run_id, parent_run_id, etc.) should be set on
/// `thread.scope` before starting the loop.
#[derive(Clone, Default)]
pub struct RunContext {
    /// Cancellation token for cooperative loop termination.
    ///
    /// When cancelled, this is the **run cancellation signal**:
    /// the loop stops at the next check point and emits `RunFinish` with
    /// `TerminationReason::Cancelled`.
    pub cancellation_token: Option<RunCancellationToken>,
    /// Optional state committer for durable state change sets.
    ///
    /// When configured, the loop emits committed `AgentChangeSet` records in
    /// order and waits for `commit()` success before continuing.
    pub state_committer: Option<Arc<dyn StateCommitter>>,
}

impl std::fmt::Debug for RunContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunContext")
            .field(
                "cancellation_token",
                &self.cancellation_token.as_ref().map(|_| "<set>"),
            )
            .field(
                "state_committer",
                &self.state_committer.as_ref().map(|_| "<set>"),
            )
            .finish()
    }
}

impl RunContext {
    pub fn run_cancellation_token(&self) -> Option<&RunCancellationToken> {
        self.cancellation_token.as_ref()
    }

    pub fn state_committer(&self) -> Option<&Arc<dyn StateCommitter>> {
        self.state_committer.as_ref()
    }

    pub fn with_state_committer(mut self, committer: Arc<dyn StateCommitter>) -> Self {
        self.state_committer = Some(committer);
        self
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
