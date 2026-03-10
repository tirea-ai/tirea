//! Sub-agent types for agent run orchestration.

use serde::{Deserialize, Serialize};

/// Status of a sub-agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubAgentStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl SubAgentStatus {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}
