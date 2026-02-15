//! Common state types for the plugin system.
//!
//! These types are used across multiple plugins and extension traits.

use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use carve_thread_model::{Thread, ToolCall};

/// Tool permission behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermissionBehavior {
    /// Tool is allowed without confirmation.
    Allow,
    /// Tool requires user confirmation before execution.
    #[default]
    Ask,
    /// Tool is denied (will not execute).
    Deny,
}

pub use crate::{Interaction, InteractionResponse};

/// Agent-owned state stored in the session document.
///
/// This is used for cross-step and cross-run flow control that should be persisted
/// via patches (not ephemeral in-memory variables).
pub const AGENT_STATE_PATH: &str = "agent";

/// Interaction action used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";

/// Interaction ID prefix used for agent run recovery confirmation.
pub const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

/// Status of an `agent_run` entry persisted in [`AgentState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// Persisted snapshot for an `agent_run` execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunState {
    /// Stable run id returned to the caller.
    pub run_id: String,
    /// Parent caller run id (from caller runtime `run_id`), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Target agent id delegated by `agent_run`.
    pub target_agent_id: String,
    /// Current run status.
    pub status: AgentRunStatus,
    /// Last assistant message from the child agent (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<String>,
    /// Error message (if the run failed or was force-stopped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Last known child session snapshot for resume/recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<Thread>,
}

/// Inference error emitted by the loop and consumed by telemetry plugins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInferenceError {
    /// Stable error class used for metrics/telemetry dimensions.
    #[serde(rename = "type")]
    pub error_type: String,
    /// Human-readable error message.
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct AgentState {
    /// Pending interaction that must be resolved by the client before the run can continue.
    #[carve(default = "None")]
    pub pending_interaction: Option<Interaction>,
    /// Persisted sub-agent runs keyed by `run_id`.
    #[carve(default = "HashMap::new()")]
    pub agent_runs: HashMap<String, AgentRunState>,
    /// Replay queue for tool calls that should run at session start.
    #[carve(default = "Vec::new()")]
    pub replay_tool_calls: Vec<ToolCall>,
    /// Interaction responses that should be emitted as runtime events.
    ///
    /// These are produced by interaction-handling plugins and drained by the runtime.
    #[carve(default = "Vec::new()")]
    pub interaction_resolutions: Vec<InteractionResponse>,
    /// User messages requested by tools, keyed by tool call id.
    ///
    /// The runtime appends these messages in tool-call order after tool execution.
    #[carve(default = "HashMap::new()")]
    pub append_user_messages: HashMap<String, Vec<String>>,
    /// Inference error envelope for AfterInference cleanup flow.
    #[carve(default = "None")]
    pub inference_error: Option<AgentInferenceError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_json::Value;

    #[test]
    fn test_tool_permission_behavior_default() {
        let behavior: ToolPermissionBehavior = Default::default();
        assert_eq!(behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn test_tool_permission_behavior_serialization() {
        let allow = ToolPermissionBehavior::Allow;
        let json = serde_json::to_string(&allow).unwrap();
        assert_eq!(json, "\"allow\"");

        let parsed: ToolPermissionBehavior = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ToolPermissionBehavior::Allow);
    }

    #[test]
    fn test_interaction_new() {
        let interaction = Interaction::new("int_1", "confirm");
        assert_eq!(interaction.id, "int_1");
        assert_eq!(interaction.action, "confirm");
        assert!(interaction.message.is_empty());
        assert_eq!(interaction.parameters, Value::Null);
        assert!(interaction.response_schema.is_none());
    }

    #[test]
    fn test_interaction_with_message() {
        let interaction = Interaction::new("int_1", "confirm").with_message("Are you sure?");
        assert_eq!(interaction.message, "Are you sure?");
    }

    #[test]
    fn test_interaction_with_parameters() {
        let interaction =
            Interaction::new("int_1", "input").with_parameters(json!({ "default": "Enter name" }));
        assert_eq!(interaction.parameters["default"], "Enter name");
    }

    #[test]
    fn test_interaction_with_response_schema() {
        let schema = json!({ "type": "boolean" });
        let interaction = Interaction::new("int_1", "confirm").with_response_schema(schema.clone());
        assert_eq!(interaction.response_schema, Some(schema));
    }

    #[test]
    fn test_interaction_builder_chain() {
        let interaction = Interaction::new("int_1", "select")
            .with_message("Choose one:")
            .with_parameters(json!({
                "choices": [
                    { "value": "yes", "label": "Yes" },
                    { "value": "no", "label": "No" }
                ]
            }))
            .with_response_schema(json!({ "type": "string" }));

        assert_eq!(interaction.id, "int_1");
        assert_eq!(interaction.action, "select");
        assert_eq!(interaction.message, "Choose one:");
        assert!(interaction.parameters["choices"].is_array());
        assert!(interaction.response_schema.is_some());
    }

    #[test]
    fn test_interaction_serialization_minimal() {
        let interaction = Interaction::new("int_1", "confirm");
        let json = serde_json::to_string(&interaction).unwrap();

        // Empty fields should be skipped
        assert!(!json.contains("message"));
        assert!(!json.contains("parameters"));
        assert!(!json.contains("response_schema"));

        // Required fields should be present
        assert!(json.contains(r#""id":"int_1""#));
        assert!(json.contains(r#""action":"confirm""#));
    }

    #[test]
    fn test_interaction_serialization_full() {
        let interaction = Interaction::new("int_1", "input")
            .with_message("Enter name:")
            .with_parameters(json!({ "default": "John" }));

        let json = serde_json::to_string(&interaction).unwrap();
        let parsed: Interaction = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, interaction.id);
        assert_eq!(parsed.action, interaction.action);
        assert_eq!(parsed.message, interaction.message);
        assert_eq!(parsed.parameters, interaction.parameters);
    }

    #[test]
    fn test_interaction_deserialization_minimal() {
        let json = r#"{"id":"int_1","action":"confirm"}"#;
        let interaction: Interaction = serde_json::from_str(json).unwrap();

        assert_eq!(interaction.id, "int_1");
        assert_eq!(interaction.action, "confirm");
        assert!(interaction.message.is_empty());
        assert_eq!(interaction.parameters, Value::Null);
    }

    #[test]
    fn test_interaction_response_new() {
        let response = InteractionResponse::new("int_1", json!(true));
        assert_eq!(response.interaction_id, "int_1");
        assert_eq!(response.result, json!(true));
    }

    #[test]
    fn test_interaction_response_serialization() {
        let response = InteractionResponse::new("int_1", json!({ "success": true }));
        let json = serde_json::to_string(&response).unwrap();
        let parsed: InteractionResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.interaction_id, "int_1");
        assert_eq!(parsed.result["success"], true);
    }

    #[test]
    fn test_interaction_response_various_results() {
        // Boolean result
        let r1 = InteractionResponse::new("id1", json!(true));
        assert_eq!(r1.result, json!(true));

        // String result
        let r2 = InteractionResponse::new("id2", json!("selected_value"));
        assert_eq!(r2.result, json!("selected_value"));

        // Object result
        let r3 = InteractionResponse::new("id3", json!({ "status": "ok", "data": [1,2,3] }));
        assert_eq!(r3.result["status"], "ok");
    }

    #[test]
    fn test_agent_run_status_serialization() {
        let status = AgentRunStatus::Stopped;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"stopped\"");
        assert_eq!(status.as_str(), "stopped");
    }

    #[test]
    fn test_agent_state_defaults_agent_runs_to_empty_map() {
        let state = AgentState::default();
        assert!(state.pending_interaction.is_none());
        assert!(state.agent_runs.is_empty());
        assert!(state.replay_tool_calls.is_empty());
        assert!(state.interaction_resolutions.is_empty());
        assert!(state.append_user_messages.is_empty());
        assert!(state.inference_error.is_none());
    }

    #[test]
    fn test_agent_run_state_serialization_with_session() {
        let child = Thread::new("child-1").with_message(carve_thread_model::Message::user("seed"));
        let run = AgentRunState {
            run_id: "run-1".to_string(),
            parent_run_id: Some("parent-run-1".to_string()),
            target_agent_id: "worker".to_string(),
            status: AgentRunStatus::Running,
            assistant: None,
            error: None,
            thread: Some(child),
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["run_id"], "run-1");
        assert_eq!(json["parent_run_id"], "parent-run-1");
        assert_eq!(json["target_agent_id"], "worker");
        assert_eq!(json["status"], "running");
        assert_eq!(json["thread"]["id"], "child-1");
    }
}
