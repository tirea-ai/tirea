use super::{AgentLoopError, StateCommitError, StateCommitter};
use crate::contracts::conversation::Thread;
use crate::contracts::context::AgentChangeSet;
use crate::contracts::storage::{CheckpointReason, ThreadDelta};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone)]
pub struct ChannelStateCommitter {
    tx: tokio::sync::mpsc::UnboundedSender<AgentChangeSet>,
}

impl ChannelStateCommitter {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<AgentChangeSet>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl StateCommitter for ChannelStateCommitter {
    async fn commit(
        &self,
        _thread_id: &str,
        changeset: AgentChangeSet,
    ) -> Result<u64, StateCommitError> {
        let next_version = changeset.expected_version.saturating_add(1);
        self.tx
            .send(changeset)
            .map_err(|e| StateCommitError::new(format!("channel state commit failed: {e}")))?;
        Ok(next_version)
    }
}

pub(super) async fn commit_pending_delta(
    thread: &mut Thread,
    reason: CheckpointReason,
    force: bool,
    run_id: &str,
    parent_run_id: Option<&str>,
    state_committer: Option<&Arc<dyn StateCommitter>>,
) -> Result<(), AgentLoopError> {
    let Some(committer) = state_committer else {
        return Ok(());
    };

    let pending = thread.take_pending();
    if !force && pending.is_empty() {
        return Ok(());
    }

    let changeset = AgentChangeSet {
        expected_version: super::thread_state_version(thread),
        delta: ThreadDelta {
            run_id: run_id.to_string(),
            parent_run_id: parent_run_id.map(str::to_string),
            reason,
            messages: pending.messages,
            patches: pending.patches,
            snapshot: None,
        },
    };
    let committed_version = committer
        .commit(&thread.id, changeset)
        .await
        .map_err(|e| AgentLoopError::StateError(format!("state commit failed: {e}")))?;
    super::set_thread_state_version(thread, committed_version);
    Ok(())
}

pub(super) struct PendingDeltaCommitContext<'a> {
    run_id: &'a str,
    parent_run_id: Option<&'a str>,
    state_committer: Option<&'a Arc<dyn StateCommitter>>,
}

impl<'a> PendingDeltaCommitContext<'a> {
    pub(super) fn new(
        run_id: &'a str,
        parent_run_id: Option<&'a str>,
        state_committer: Option<&'a Arc<dyn StateCommitter>>,
    ) -> Self {
        Self {
            run_id,
            parent_run_id,
            state_committer,
        }
    }

    pub(super) async fn commit(
        &self,
        thread: &mut Thread,
        reason: CheckpointReason,
        force: bool,
    ) -> Result<(), AgentLoopError> {
        commit_pending_delta(
            thread,
            reason,
            force,
            self.run_id,
            self.parent_run_id,
            self.state_committer,
        )
        .await
    }
}
