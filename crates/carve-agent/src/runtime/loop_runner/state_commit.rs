use super::{AgentLoopError, StateCommitError, StateCommitter};
use crate::thread::Thread;
use crate::thread_store::{CheckpointReason, ThreadDelta};
use async_trait::async_trait;
use std::sync::Arc;

#[derive(Clone)]
pub struct ChannelStateCommitter {
    tx: tokio::sync::mpsc::UnboundedSender<ThreadDelta>,
}

impl ChannelStateCommitter {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<ThreadDelta>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl StateCommitter for ChannelStateCommitter {
    async fn commit(&self, _thread_id: &str, delta: ThreadDelta) -> Result<(), StateCommitError> {
        self.tx
            .send(delta)
            .map_err(|e| StateCommitError::new(format!("channel state commit failed: {e}")))
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

    let delta = ThreadDelta {
        run_id: run_id.to_string(),
        parent_run_id: parent_run_id.map(str::to_string),
        reason,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    committer
        .commit(&thread.id, delta)
        .await
        .map_err(|e| AgentLoopError::StateError(format!("state commit failed: {e}")))
}
