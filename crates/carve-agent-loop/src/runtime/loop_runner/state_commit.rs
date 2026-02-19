use super::{AgentLoopError, StateCommitError, StateCommitter};
use crate::contracts::state::CheckpointReason;
use crate::contracts::storage::VersionPrecondition;
use crate::contracts::AgentChangeSet;
use crate::contracts::RunContext;
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
        precondition: VersionPrecondition,
    ) -> Result<u64, StateCommitError> {
        let next_version = match precondition {
            VersionPrecondition::Any => 1,
            VersionPrecondition::Exact(version) => version.saturating_add(1),
        };
        self.tx
            .send(changeset)
            .map_err(|e| StateCommitError::new(format!("channel state commit failed: {e}")))?;
        Ok(next_version)
    }
}

pub(super) async fn commit_pending_delta(
    run_ctx: &mut RunContext,
    reason: CheckpointReason,
    force: bool,
    run_id: &str,
    parent_run_id: Option<&str>,
    state_committer: Option<&Arc<dyn StateCommitter>>,
) -> Result<(), AgentLoopError> {
    let Some(committer) = state_committer else {
        return Ok(());
    };

    let delta = run_ctx.take_delta();
    if !force && delta.is_empty() {
        return Ok(());
    }

    let changeset = AgentChangeSet::from_parts(
        run_id.to_string(),
        parent_run_id.map(str::to_string),
        reason,
        delta.messages,
        delta.patches,
        None,
    );
    let precondition = VersionPrecondition::Exact(run_ctx.version());
    let committed_version = committer
        .commit(run_ctx.thread_id(), changeset, precondition)
        .await
        .map_err(|e| AgentLoopError::StateError(format!("state commit failed: {e}")))?;
    run_ctx.set_version(committed_version, Some(super::current_unix_millis()));
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
        run_ctx: &mut RunContext,
        reason: CheckpointReason,
        force: bool,
    ) -> Result<(), AgentLoopError> {
        commit_pending_delta(
            run_ctx,
            reason,
            force,
            self.run_id,
            self.parent_run_id,
            self.state_committer,
        )
        .await
    }
}
