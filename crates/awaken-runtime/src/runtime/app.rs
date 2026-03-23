use crate::state::{Snapshot, StateCommand, StateStore};
use awaken_contract::StateError;
use awaken_contract::model::Phase;

use crate::phase::{ExecutionEnv, PhaseContext, PhaseRunReport, PhaseRuntime, SubmitCommandReport};

#[derive(Clone)]
pub struct AppRuntime {
    store: StateStore,
    phase_runtime: PhaseRuntime,
}

impl AppRuntime {
    pub fn new() -> Result<Self, StateError> {
        let store = StateStore::new();
        let phase_runtime = PhaseRuntime::new(store.clone())?;
        Ok(Self {
            store,
            phase_runtime,
        })
    }

    pub fn store(&self) -> &StateStore {
        &self.store
    }

    pub fn phase_runtime(&self) -> &PhaseRuntime {
        &self.phase_runtime
    }

    pub fn snapshot(&self) -> Snapshot {
        self.store.snapshot()
    }

    pub fn revision(&self) -> u64 {
        self.store.revision()
    }

    pub async fn submit_command(
        &self,
        env: &ExecutionEnv,
        command: StateCommand,
    ) -> Result<SubmitCommandReport, StateError> {
        self.phase_runtime.submit_command(env, command).await
    }

    pub async fn run_phase(
        &self,
        env: &ExecutionEnv,
        phase: Phase,
    ) -> Result<PhaseRunReport, StateError> {
        self.phase_runtime.run_phase(env, phase).await
    }

    pub async fn run_phase_with_context(
        &self,
        env: &ExecutionEnv,
        ctx: PhaseContext,
    ) -> Result<PhaseRunReport, StateError> {
        self.phase_runtime.run_phase_with_context(env, ctx).await
    }

    pub async fn run_phase_with_limit(
        &self,
        env: &ExecutionEnv,
        phase: Phase,
        max_rounds: usize,
    ) -> Result<PhaseRunReport, StateError> {
        self.phase_runtime
            .run_phase_with_limit(env, phase, max_rounds)
            .await
    }
}
