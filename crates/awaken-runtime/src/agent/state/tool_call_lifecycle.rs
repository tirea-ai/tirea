use crate::state::{MergeStrategy, StateKey};
use awaken_contract::contract::suspension::ToolCallStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Per-tool-call lifecycle state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallState {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub status: ToolCallStatus,
    pub updated_at: u64,
}

/// Keyed collection of tool call states for the current step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallStateMap {
    pub calls: HashMap<String, ToolCallState>,
}

/// Update for the tool call states key.
pub enum ToolCallStatesUpdate {
    /// Upsert a tool call's lifecycle state (validates transition).
    Upsert {
        call_id: String,
        tool_name: String,
        arguments: Value,
        status: ToolCallStatus,
        updated_at: u64,
    },
    /// Clear all tool call states (at step boundary).
    Clear,
}

/// State key for tool call lifecycle tracking within a step.
pub struct ToolCallStates;

impl StateKey for ToolCallStates {
    const KEY: &'static str = "__runtime.tool_call_states";
    const MERGE: MergeStrategy = MergeStrategy::Commutative;

    type Value = ToolCallStateMap;
    type Update = ToolCallStatesUpdate;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        match update {
            ToolCallStatesUpdate::Upsert {
                call_id,
                tool_name,
                arguments,
                status,
                updated_at,
            } => {
                let existing = value.calls.get(&call_id);
                let current_status = existing.map(|s| s.status).unwrap_or(ToolCallStatus::New);

                assert!(
                    current_status.can_transition_to(status),
                    "invalid tool call transition from {current_status:?} to {status:?} for call {call_id}",
                );

                value.calls.insert(
                    call_id.clone(),
                    ToolCallState {
                        call_id,
                        tool_name,
                        arguments,
                        status,
                        updated_at,
                    },
                );
            }
            ToolCallStatesUpdate::Clear => {
                value.calls.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert(
        states: &mut ToolCallStateMap,
        call_id: &str,
        tool: &str,
        status: ToolCallStatus,
        ts: u64,
    ) {
        ToolCallStates::apply(
            states,
            ToolCallStatesUpdate::Upsert {
                call_id: call_id.into(),
                tool_name: tool.into(),
                arguments: serde_json::json!({}),
                status,
                updated_at: ts,
            },
        );
    }

    #[test]
    fn tool_call_new_to_running() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Running);
    }

    #[test]
    fn tool_call_running_to_succeeded() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Succeeded, 200);
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Succeeded);
    }

    #[test]
    fn tool_call_running_to_failed() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Failed, 200);
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Failed);
    }

    #[test]
    fn tool_call_running_to_suspended_to_resuming() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Suspended, 200);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Resuming, 300);
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Resuming);
    }

    #[test]
    fn tool_call_suspended_to_cancelled() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Suspended, 200);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Cancelled, 300);
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Cancelled);
        assert!(states.calls["c1"].status.is_terminal());
    }

    #[test]
    #[should_panic(expected = "invalid tool call transition")]
    fn tool_call_rejects_succeeded_to_running() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Succeeded, 200);
        // Terminal -> Running is invalid
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 300);
    }

    #[test]
    #[should_panic(expected = "invalid tool call transition")]
    fn tool_call_rejects_failed_to_running() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Failed, 200);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 300);
    }

    #[test]
    fn tool_call_multiple_calls_independent() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c2", "calc", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Succeeded, 200);
        upsert(&mut states, "c2", "calc", ToolCallStatus::Failed, 200);

        assert_eq!(states.calls["c1"].status, ToolCallStatus::Succeeded);
        assert_eq!(states.calls["c2"].status, ToolCallStatus::Failed);
    }

    #[test]
    fn tool_call_clear_removes_all() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c2", "calc", ToolCallStatus::Running, 100);
        ToolCallStates::apply(&mut states, ToolCallStatesUpdate::Clear);
        assert!(states.calls.is_empty());
    }

    #[test]
    fn tool_call_state_serde_roundtrip() {
        let mut states = ToolCallStateMap::default();
        upsert(&mut states, "c1", "echo", ToolCallStatus::Running, 100);
        upsert(&mut states, "c1", "echo", ToolCallStatus::Succeeded, 200);
        let json = serde_json::to_string(&states).unwrap();
        let parsed: ToolCallStateMap = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, states);
    }

    #[test]
    fn tool_call_full_lifecycle_suspend_resume_succeed() {
        let mut states = ToolCallStateMap::default();
        // New -> Running
        upsert(&mut states, "c1", "dangerous", ToolCallStatus::Running, 100);
        // Running -> Suspended
        upsert(
            &mut states,
            "c1",
            "dangerous",
            ToolCallStatus::Suspended,
            200,
        );
        // Suspended -> Resuming
        upsert(
            &mut states,
            "c1",
            "dangerous",
            ToolCallStatus::Resuming,
            300,
        );
        // Resuming -> Running
        upsert(&mut states, "c1", "dangerous", ToolCallStatus::Running, 400);
        // Running -> Succeeded
        upsert(
            &mut states,
            "c1",
            "dangerous",
            ToolCallStatus::Succeeded,
            500,
        );
        assert_eq!(states.calls["c1"].status, ToolCallStatus::Succeeded);
        assert_eq!(states.calls["c1"].updated_at, 500);
    }
}
