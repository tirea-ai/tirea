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

/// Declarative stop-condition configuration used by loop runtimes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StopConditionSpec {
    /// Stop after a fixed number of tool-call rounds.
    MaxRounds { rounds: usize },
    /// Stop after a wall-clock duration (in seconds) elapses.
    Timeout { seconds: u64 },
    /// Stop when cumulative token usage exceeds a budget. 0 = unlimited.
    TokenBudget { max_total: usize },
    /// Stop after N consecutive rounds where all tool executions failed. 0 = disabled.
    ConsecutiveErrors { max: usize },
    /// Stop when a specific tool is called by the LLM.
    StopOnTool { tool_name: String },
    /// Stop when LLM output text contains a literal pattern.
    ContentMatch { pattern: String },
    /// Stop when identical tool call patterns repeat within a sliding window.
    LoopDetection { window: usize },
}
