use super::*;
use crate::engine::stop_conditions::StopReason;
use serde_json::{json, Value};

/// Aggregated token usage for one loop run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
}

/// Aggregated runtime metrics for one loop run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoopStats {
    pub duration_ms: u64,
    pub steps: usize,
    pub llm_calls: usize,
    pub llm_retries: usize,
    pub tool_calls: usize,
    pub tool_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LoopFailure {
    Llm(String),
    State(String),
}

/// Unified terminal state for loop execution.
#[derive(Debug)]
pub struct LoopOutcome {
    pub run_ctx: crate::contracts::RunContext,
    pub termination: TerminationReason,
    pub response: Option<String>,
    pub usage: LoopUsage,
    pub stats: LoopStats,
    #[allow(dead_code)]
    pub(super) failure: Option<LoopFailure>,
}

impl LoopOutcome {
    /// Build a `RunFinish.result` payload from the unified outcome.
    pub fn run_finish_result(&self) -> Option<Value> {
        if !matches!(self.termination, TerminationReason::NaturalEnd) {
            return None;
        }
        self.response
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|text| json!({ "response": text }))
    }

    /// Project unified outcome into stream `RunFinish` event.
    pub fn to_run_finish_event(self, run_id: String) -> AgentEvent {
        AgentEvent::RunFinish {
            thread_id: self.run_ctx.thread_id().to_string(),
            run_id,
            result: self.run_finish_result(),
            termination: self.termination,
        }
    }
}

/// Error type for agent loop operations.
#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("State error: {0}")]
    StateError(String),
    /// The agent loop terminated normally due to a stop condition.
    ///
    /// This is not an error but a structured stop with a reason. The run context
    /// is included so callers can inspect final state.
    #[error("Agent stopped: {reason:?}")]
    Stopped {
        run_ctx: Box<crate::contracts::RunContext>,
        reason: StopReason,
    },
    /// Run suspended waiting for external resolution of a tool call.
    ///
    /// The returned run context includes any patches applied up to the suspension point
    /// (including persisted suspended tool calls).
    #[error(
        "Run suspended on tool call: {call_id} ({tool_name})",
        call_id = suspended_call.call_id,
        tool_name = suspended_call.tool_name
    )]
    Suspended {
        run_ctx: Box<crate::contracts::RunContext>,
        suspended_call: Box<SuspendedCall>,
    },
    /// External cancellation signal requested run termination.
    #[error("Run cancelled")]
    Cancelled {
        run_ctx: Box<crate::contracts::RunContext>,
    },
}

impl AgentLoopError {
    /// Normalize loop errors into lifecycle termination semantics.
    pub fn termination_reason(&self) -> TerminationReason {
        match self {
            Self::Stopped { reason, .. } => TerminationReason::Stopped(match reason {
                StopReason::MaxRoundsReached => {
                    crate::contracts::StoppedReason::new("max_rounds_reached")
                }
                StopReason::TimeoutReached => {
                    crate::contracts::StoppedReason::new("timeout_reached")
                }
                StopReason::TokenBudgetExceeded => {
                    crate::contracts::StoppedReason::new("token_budget_exceeded")
                }
                StopReason::ToolCalled(tool_name) => {
                    crate::contracts::StoppedReason::with_detail("tool_called", tool_name.clone())
                }
                StopReason::ContentMatched(pattern) => {
                    crate::contracts::StoppedReason::with_detail("content_matched", pattern.clone())
                }
                StopReason::ConsecutiveErrorsExceeded => {
                    crate::contracts::StoppedReason::new("consecutive_errors_exceeded")
                }
                StopReason::LoopDetected => crate::contracts::StoppedReason::new("loop_detected"),
                StopReason::Custom(reason) => {
                    crate::contracts::StoppedReason::with_detail("custom", reason.clone())
                }
            }),
            Self::Cancelled { .. } => TerminationReason::Cancelled,
            Self::Suspended { .. } => TerminationReason::Suspended,
            Self::LlmError(_) | Self::StateError(_) => TerminationReason::Error,
        }
    }
}

impl From<crate::contracts::runtime::ToolExecutorError> for AgentLoopError {
    fn from(value: crate::contracts::runtime::ToolExecutorError) -> Self {
        match value {
            crate::contracts::runtime::ToolExecutorError::Cancelled { thread_id } => {
                Self::Cancelled {
                    run_ctx: Box::new(crate::contracts::RunContext::new(
                        thread_id,
                        serde_json::json!({}),
                        vec![],
                        crate::contracts::RunConfig::default(),
                    )),
                }
            }
            crate::contracts::runtime::ToolExecutorError::Failed { message } => {
                Self::StateError(message)
            }
        }
    }
}

/// Helper to create a tool map from an iterator of tools.
pub fn tool_map<I, T>(tools: I) -> HashMap<String, Arc<dyn Tool>>
where
    I: IntoIterator<Item = T>,
    T: Tool + 'static,
{
    tools
        .into_iter()
        .map(|t| {
            let name = t.descriptor().id.clone();
            (name, Arc::new(t) as Arc<dyn Tool>)
        })
        .collect()
}

/// Helper to create a tool map from Arc<dyn Tool>.
pub fn tool_map_from_arc<I>(tools: I) -> HashMap<String, Arc<dyn Tool>>
where
    I: IntoIterator<Item = Arc<dyn Tool>>,
{
    tools
        .into_iter()
        .map(|t| (t.descriptor().id.clone(), t))
        .collect()
}
