//! Permission extension contracts.
//!
//! This module defines the persisted permission state and policy enum used by
//! permission plugins/tools.

use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// State path for permission configuration inside `AgentState.state`.
pub const PERMISSION_STATE_PATH: &str = "permissions";

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

/// Persisted permission state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct PermissionState {
    /// Default behavior for tools not explicitly configured.
    pub default_behavior: ToolPermissionBehavior,
    /// Per-tool permission overrides.
    pub tools: HashMap<String, ToolPermissionBehavior>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_permission_behavior_default_is_ask() {
        let behavior: ToolPermissionBehavior = Default::default();
        assert_eq!(behavior, ToolPermissionBehavior::Ask);
    }

    #[test]
    fn tool_permission_behavior_serde_roundtrip() {
        let allow = ToolPermissionBehavior::Allow;
        let json = serde_json::to_string(&allow).unwrap();
        assert_eq!(json, "\"allow\"");

        let parsed: ToolPermissionBehavior = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ToolPermissionBehavior::Allow);
    }
}
