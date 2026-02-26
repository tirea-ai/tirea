use crate::runtime::state_paths::RUN_LIFECYCLE_STATE_PATH;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Generic stopped payload emitted when a plugin decides to terminate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoppedReason {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StoppedReason {
    #[must_use]
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }
}

/// Why a run terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TerminationReason {
    /// LLM returned a response with no tool calls.
    NaturalEnd,
    /// A plugin requested inference skip.
    PluginRequested,
    /// A configured stop condition fired.
    Stopped(StoppedReason),
    /// External run cancellation signal was received.
    Cancelled,
    /// Run paused waiting for external suspended tool-call resolution.
    Suspended,
    /// Run ended due to an error path.
    Error,
}

impl TerminationReason {
    #[must_use]
    pub fn stopped(code: impl Into<String>) -> Self {
        Self::Stopped(StoppedReason::new(code))
    }

    #[must_use]
    pub fn stopped_with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Stopped(StoppedReason::with_detail(code, detail))
    }
}

/// Coarse run lifecycle status persisted in thread state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycleStatus {
    /// Run is actively executing.
    Running,
    /// Run is waiting for external decisions.
    Waiting,
    /// Run has reached a terminal state.
    Done,
}

impl RunLifecycleStatus {
    /// Canonical run-lifecycle state machine used by runtime tests.
    pub const ASCII_STATE_MACHINE: &str = r#"start
  |
  v
running -------> done
  |
  v
waiting -------> done
  |
  +-----------> running"#;

    /// Whether this lifecycle status is terminal.
    pub fn is_terminal(self) -> bool {
        matches!(self, RunLifecycleStatus::Done)
    }

    /// Validate lifecycle transition from `self` to `next`.
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }

        match self {
            RunLifecycleStatus::Running => {
                matches!(next, RunLifecycleStatus::Waiting | RunLifecycleStatus::Done)
            }
            RunLifecycleStatus::Waiting => {
                matches!(next, RunLifecycleStatus::Running | RunLifecycleStatus::Done)
            }
            RunLifecycleStatus::Done => false,
        }
    }
}

/// Minimal durable run lifecycle envelope stored at `state["__run"]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunLifecycleState {
    /// Current run id associated with this lifecycle record.
    pub id: String,
    /// Coarse lifecycle status.
    pub status: RunLifecycleStatus,
    /// Optional terminal reason when `status=done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_reason: Option<String>,
    /// Last update timestamp (unix millis).
    pub updated_at: u64,
}

/// Parse persisted run lifecycle from a rebuilt state snapshot.
pub fn run_lifecycle_from_state(state: &Value) -> Option<RunLifecycleState> {
    state
        .get(RUN_LIFECYCLE_STATE_PATH)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_lifecycle_roundtrip_from_state() {
        let state = serde_json::json!({
            "__run": {
                "id": "run_1",
                "status": "running",
                "updated_at": 42
            }
        });

        let lifecycle = run_lifecycle_from_state(&state).expect("run lifecycle");
        assert_eq!(lifecycle.id, "run_1");
        assert_eq!(lifecycle.status, RunLifecycleStatus::Running);
        assert_eq!(lifecycle.done_reason, None);
        assert_eq!(lifecycle.updated_at, 42);
    }

    #[test]
    fn run_lifecycle_status_transitions_match_state_machine() {
        assert!(RunLifecycleStatus::Running.can_transition_to(RunLifecycleStatus::Waiting));
        assert!(RunLifecycleStatus::Running.can_transition_to(RunLifecycleStatus::Done));
        assert!(RunLifecycleStatus::Waiting.can_transition_to(RunLifecycleStatus::Running));
        assert!(RunLifecycleStatus::Waiting.can_transition_to(RunLifecycleStatus::Done));
        assert!(RunLifecycleStatus::Running.can_transition_to(RunLifecycleStatus::Running));
    }

    #[test]
    fn run_lifecycle_status_rejects_done_reopen_transitions() {
        assert!(!RunLifecycleStatus::Done.can_transition_to(RunLifecycleStatus::Running));
        assert!(!RunLifecycleStatus::Done.can_transition_to(RunLifecycleStatus::Waiting));
    }

    #[test]
    fn run_lifecycle_ascii_state_machine_contains_all_states() {
        let diagram = RunLifecycleStatus::ASCII_STATE_MACHINE;
        assert!(diagram.contains("running"));
        assert!(diagram.contains("waiting"));
        assert!(diagram.contains("done"));
        assert!(diagram.contains("start"));
    }
}
