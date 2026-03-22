//! RunRequest, RunInput, and RunOptions structs.

use awaken_contract::contract::inference::InferenceOverride;
use awaken_contract::contract::message::Message;
use awaken_contract::contract::suspension::ToolCallResume;

/// Unified request for starting or resuming a run.
pub struct RunRequest {
    /// Main business input for this run.
    pub input: RunInput,
    /// Runtime control options (session/routing/overrides/resume).
    pub options: RunOptions,
}

/// Primary run input payload.
#[derive(Default)]
pub struct RunInput {
    /// New messages to append before running.
    pub messages: Vec<Message>,
}

/// Runtime control options for run execution.
pub struct RunOptions {
    /// Thread ID. Existing → load history; new → create.
    pub thread_id: String,
    /// Target agent ID. `None` = use default or infer from thread state.
    pub agent_id: Option<String>,
    /// Runtime parameter overrides for this run.
    pub overrides: Option<InferenceOverride>,
    /// Resume decisions for suspended tool calls. Empty = fresh run.
    pub decisions: Vec<(String, ToolCallResume)>,
}

impl RunRequest {
    /// Build a message-first request with default options.
    pub fn new(thread_id: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            input: RunInput { messages },
            options: RunOptions {
                thread_id: thread_id.into(),
                agent_id: None,
                overrides: None,
                decisions: Vec::new(),
            },
        }
    }

    #[must_use]
    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.options.agent_id = Some(agent_id.into());
        self
    }

    #[must_use]
    pub fn with_overrides(mut self, overrides: InferenceOverride) -> Self {
        self.options.overrides = Some(overrides);
        self
    }

    #[must_use]
    pub fn with_decisions(mut self, decisions: Vec<(String, ToolCallResume)>) -> Self {
        self.options.decisions = decisions;
        self
    }
}
