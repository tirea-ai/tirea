//! Agent runtime: top-level orchestrator for run management, routing, and control.

mod active_registry;
mod control;
mod run_request;
mod runner;

use std::sync::Arc;

use crate::contract::storage::ThreadRunStore;
use crate::contract::suspension::ToolCallResume;
use crate::error::StateError;
use futures::channel::mpsc;

use crate::runtime::cancellation::CancellationToken;
use crate::runtime::resolver::AgentResolver;

pub use run_request::{RunInput, RunOptions, RunRequest};

use active_registry::{ActiveRunRegistry, RunEntry};

// ---------------------------------------------------------------------------
// RunHandle
// ---------------------------------------------------------------------------

/// External control handle for a running agent loop.
///
/// Returned by `AgentRuntime`. Enables cancellation and
/// live decision injection.
#[derive(Clone)]
pub struct RunHandle {
    pub run_id: String,
    pub thread_id: String,
    pub agent_id: String,
    cancellation_token: CancellationToken,
    decision_tx: mpsc::UnboundedSender<(String, ToolCallResume)>,
}

impl RunHandle {
    /// Cancel the running agent loop cooperatively.
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Send a tool call decision to the running loop.
    pub fn send_decision(
        &self,
        call_id: String,
        resume: ToolCallResume,
    ) -> Result<(), mpsc::TrySendError<(String, ToolCallResume)>> {
        self.decision_tx.unbounded_send((call_id, resume))
    }
}

// ---------------------------------------------------------------------------
// AgentRuntime
// ---------------------------------------------------------------------------

/// Top-level agent runtime. Manages runs across threads.
///
/// Provides methods for cancelling and sending decisions
/// to active agent runs. Enforces one active run per thread.
pub struct AgentRuntime {
    pub(crate) resolver: Arc<dyn AgentResolver>,
    pub(crate) storage: Option<Arc<dyn ThreadRunStore>>,
    pub(crate) active_runs: ActiveRunRegistry,
}

impl AgentRuntime {
    pub fn new(resolver: Arc<dyn AgentResolver>) -> Self {
        Self {
            resolver,
            storage: None,
            active_runs: ActiveRunRegistry::new(),
        }
    }

    #[must_use]
    pub fn with_thread_run_store(mut self, store: Arc<dyn ThreadRunStore>) -> Self {
        self.storage = Some(store);
        self
    }

    pub fn resolver(&self) -> &dyn AgentResolver {
        self.resolver.as_ref()
    }

    pub fn thread_run_store(&self) -> Option<&dyn ThreadRunStore> {
        self.storage.as_deref()
    }

    /// Create a run handle pair (handle + internal channels).
    ///
    /// Returns (RunHandle for caller, CancellationToken for loop, decision_rx for loop).
    pub(crate) fn create_run_channels(
        &self,
        run_id: String,
        thread_id: String,
        agent_id: String,
    ) -> (
        RunHandle,
        CancellationToken,
        mpsc::UnboundedReceiver<(String, ToolCallResume)>,
    ) {
        let token = CancellationToken::new();
        let (tx, rx) = mpsc::unbounded();

        let handle = RunHandle {
            run_id,
            thread_id,
            agent_id,
            cancellation_token: token.clone(),
            decision_tx: tx,
        };

        (handle, token, rx)
    }

    /// Register an active run. Returns error if thread already has one.
    pub(crate) fn register_run(
        &self,
        thread_id: &str,
        handle: RunHandle,
    ) -> Result<(), StateError> {
        if self.active_runs.has_active_run(thread_id) {
            return Err(StateError::ThreadAlreadyRunning {
                thread_id: thread_id.to_string(),
            });
        }
        self.active_runs.insert(
            thread_id.to_string(),
            RunEntry {
                run_id: handle.run_id.clone(),
                agent_id: handle.agent_id.clone(),
                handle,
            },
        );
        Ok(())
    }

    /// Unregister an active run when it completes.
    pub(crate) fn unregister_run(&self, thread_id: &str) {
        self.active_runs.remove(thread_id);
    }
}
