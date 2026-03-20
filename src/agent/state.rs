//! Agent loop state slots — run lifecycle and step tracking.

use crate::contract::lifecycle::RunStatus;
use crate::state::StateSlot;
use serde::{Deserialize, Serialize};

/// Run lifecycle state stored in the state engine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLifecycleState {
    /// Current run id.
    pub run_id: String,
    /// Coarse lifecycle status.
    pub status: RunStatus,
    /// Optional terminal reason when `status=done`.
    pub done_reason: Option<String>,
    /// Last update timestamp (unix millis).
    pub updated_at: u64,
    /// Total steps completed.
    pub step_count: u32,
}

/// Update for RunLifecycleSlot.
pub enum RunLifecycleUpdate {
    Start {
        run_id: String,
        updated_at: u64,
    },
    StepCompleted {
        updated_at: u64,
    },
    SetWaiting {
        updated_at: u64,
    },
    Done {
        done_reason: String,
        updated_at: u64,
    },
}

/// StateSlot for run lifecycle tracking.
pub struct RunLifecycleSlot;

impl StateSlot for RunLifecycleSlot {
    const KEY: &'static str = "__runtime.run_lifecycle";

    type Value = RunLifecycleState;
    type Update = RunLifecycleUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            RunLifecycleUpdate::Start { run_id, updated_at } => {
                value.run_id = run_id;
                value.status = RunStatus::Running;
                value.done_reason = None;
                value.updated_at = updated_at;
                value.step_count = 0;
            }
            RunLifecycleUpdate::StepCompleted { updated_at } => {
                value.step_count += 1;
                value.updated_at = updated_at;
            }
            RunLifecycleUpdate::SetWaiting { updated_at } => {
                value.status = RunStatus::Waiting;
                value.updated_at = updated_at;
            }
            RunLifecycleUpdate::Done {
                done_reason,
                updated_at,
            } => {
                value.status = RunStatus::Done;
                value.done_reason = Some(done_reason);
                value.updated_at = updated_at;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn run_lifecycle_start_sets_running() {
        let mut state = RunLifecycleState::default();
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Start {
                run_id: "r1".into(),
                updated_at: 100,
            },
        );
        assert_eq!(state.run_id, "r1");
        assert_eq!(state.status, RunStatus::Running);
        assert_eq!(state.step_count, 0);
    }

    #[test]
    fn run_lifecycle_step_completed_increments() {
        let mut state = RunLifecycleState::default();
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Start {
                run_id: "r1".into(),
                updated_at: 100,
            },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::StepCompleted { updated_at: 200 },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::StepCompleted { updated_at: 300 },
        );
        assert_eq!(state.step_count, 2);
        assert_eq!(state.updated_at, 300);
    }

    #[test]
    fn run_lifecycle_done_sets_terminal() {
        let mut state = RunLifecycleState::default();
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Start {
                run_id: "r1".into(),
                updated_at: 100,
            },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Done {
                done_reason: "natural".into(),
                updated_at: 200,
            },
        );
        assert_eq!(state.status, RunStatus::Done);
        assert_eq!(state.done_reason.as_deref(), Some("natural"));
        assert!(state.status.is_terminal());
    }

    #[test]
    fn run_lifecycle_waiting_transition() {
        let mut state = RunLifecycleState::default();
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Start {
                run_id: "r1".into(),
                updated_at: 100,
            },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::SetWaiting { updated_at: 150 },
        );
        assert_eq!(state.status, RunStatus::Waiting);
    }

    #[test]
    fn run_lifecycle_full_sequence() {
        let mut state = RunLifecycleState::default();
        let t = now_ms();

        // Start
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Start {
                run_id: "run-42".into(),
                updated_at: t,
            },
        );
        assert_eq!(state.status, RunStatus::Running);

        // Steps
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::StepCompleted { updated_at: t + 1 },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::StepCompleted { updated_at: t + 2 },
        );
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::StepCompleted { updated_at: t + 3 },
        );
        assert_eq!(state.step_count, 3);

        // Done
        RunLifecycleSlot::apply(
            &mut state,
            RunLifecycleUpdate::Done {
                done_reason: "stopped:max_turns".into(),
                updated_at: t + 4,
            },
        );
        assert_eq!(state.status, RunStatus::Done);
        assert_eq!(state.step_count, 3);
    }

    #[test]
    fn run_lifecycle_state_serde_roundtrip() {
        let state = RunLifecycleState {
            run_id: "r1".into(),
            status: RunStatus::Done,
            done_reason: Some("natural".into()),
            updated_at: 12345,
            step_count: 3,
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: RunLifecycleState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, state);
    }
}
