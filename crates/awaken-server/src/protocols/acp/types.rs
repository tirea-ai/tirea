//! ACP protocol types.

use serde::{Deserialize, Serialize};

/// ACP tool call status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    InProgress,
    Completed,
    Denied,
    Errored,
}

/// ACP stop reason for session completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    Cancelled,
    Error,
    Suspended,
}

/// Permission options for tool approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOption {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_status_serde_roundtrip() {
        for status in [
            ToolCallStatus::InProgress,
            ToolCallStatus::Completed,
            ToolCallStatus::Denied,
            ToolCallStatus::Errored,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let parsed: ToolCallStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, status);
        }
    }

    #[test]
    fn stop_reason_serde_roundtrip() {
        for reason in [
            StopReason::EndTurn,
            StopReason::MaxTokens,
            StopReason::Cancelled,
            StopReason::Error,
            StopReason::Suspended,
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            let parsed: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, reason);
        }
    }

    #[test]
    fn permission_option_serde_roundtrip() {
        for opt in [
            PermissionOption::AllowOnce,
            PermissionOption::AllowAlways,
            PermissionOption::RejectOnce,
            PermissionOption::RejectAlways,
        ] {
            let json = serde_json::to_string(&opt).unwrap();
            let parsed: PermissionOption = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, opt);
        }
    }
}
