use serde::{Deserialize, Serialize};

/// Why the agent loop stopped (normal termination, not errors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum StopReason {
    /// LLM returned a response with no tool calls.
    NaturalEnd,
    /// A plugin set `skip_inference = true`.
    PluginRequested,
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
    /// Run cancellation signal received.
    Cancelled,
    /// Custom stop reason from a user-defined condition.
    Custom(String),
}
