use crate::io::ToolCallDecision;
use crate::thread::Message;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unified runtime request for all external protocols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    /// Target agent identifier.
    pub agent_id: String,
    /// Thread (conversation) ID. `None` -> auto-generate.
    pub thread_id: Option<String>,
    /// Run ID. `None` -> auto-generate.
    pub run_id: Option<String>,
    /// Parent run ID for nested/delegated execution.
    pub parent_run_id: Option<String>,
    /// Resource this thread belongs to (for listing/querying).
    pub resource_id: Option<String>,
    /// Frontend state snapshot.
    pub state: Option<Value>,
    /// Messages to append before running.
    pub messages: Vec<Message>,
    /// Decisions to enqueue before loop start.
    #[serde(default)]
    pub initial_decisions: Vec<ToolCallDecision>,
}

/// Unified upstream message type for the runtime endpoint.
#[derive(Debug, Clone)]
pub enum RuntimeInput {
    /// Start a new run with the given request.
    Run(RunRequest),
    /// A tool-call decision forwarded to the running loop.
    Decision(ToolCallDecision),
    /// Explicit application-level cancellation.
    Cancel,
}
