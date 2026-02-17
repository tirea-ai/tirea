use crate::runtime::termination::StopReason;
use crate::state::{AgentState, ToolCall};
use std::collections::VecDeque;
use std::time::Duration;

/// Aggregated runtime stats consumed by stop policies.
pub struct StopPolicyStats<'a> {
    /// Number of completed steps.
    pub step: usize,
    /// Tool calls emitted by the current step.
    pub step_tool_call_count: usize,
    /// Total tool calls across the whole run.
    pub total_tool_call_count: usize,
    /// Cumulative input tokens across all LLM calls.
    pub total_input_tokens: usize,
    /// Cumulative output tokens across all LLM calls.
    pub total_output_tokens: usize,
    /// Number of consecutive rounds where all tools failed.
    pub consecutive_errors: usize,
    /// Time elapsed since the loop started.
    pub elapsed: Duration,
    /// Tool calls from the most recent LLM response.
    pub last_tool_calls: &'a [ToolCall],
    /// Text from the most recent LLM response.
    pub last_text: &'a str,
    /// History of tool call names per round (most recent last), for loop detection.
    pub tool_call_history: &'a VecDeque<Vec<String>>,
}

/// Canonical stop-policy input: persisted state + runtime stats.
pub struct StopPolicyInput<'a> {
    /// Current agent state snapshot.
    pub agent_state: &'a AgentState,
    /// Runtime run stats.
    pub stats: StopPolicyStats<'a>,
}

/// Preferred stop-policy contract.
pub trait StopPolicy: Send + Sync {
    /// Unique identifier for this policy.
    fn id(&self) -> &str;

    /// Evaluate stop decision from canonical input.
    fn evaluate(&self, input: &StopPolicyInput<'_>) -> Option<StopReason>;
}
