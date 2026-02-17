use serde::{Deserialize, Serialize};

/// Why stop-condition evaluation requested loop termination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum StopReason {
    /// Maximum tool-call rounds reached.
    MaxRoundsReached,
    /// Total elapsed time exceeded the configured limit.
    TimeoutReached,
    /// Cumulative token usage exceeded the configured budget.
    TokenBudgetExceeded,
    /// A specific tool was called that triggers termination.
    ToolCalled(String),
    /// LLM output matched a stop pattern.
    ContentMatched(String),
    /// Too many consecutive tool execution failures.
    ConsecutiveErrorsExceeded,
    /// Identical tool call patterns detected across rounds.
    LoopDetected,
    /// Custom stop reason from a user-defined condition.
    Custom(String),
}

/// Why a run terminated.
///
/// This is the top-level lifecycle exit reason for a run. A stop-condition hit is
/// represented by `Stopped(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TerminationReason {
    /// LLM returned a response with no tool calls.
    NaturalEnd,
    /// A plugin requested inference skip.
    PluginRequested,
    /// A configured stop condition fired.
    Stopped(StopReason),
    /// External run cancellation signal was received.
    Cancelled,
    /// Run paused waiting for external interaction input.
    PendingInteraction,
    /// Run ended due to an error path.
    Error,
}
