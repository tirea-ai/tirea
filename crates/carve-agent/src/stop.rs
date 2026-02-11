//! Stop conditions for the agent loop.
//!
//! This module provides composable stop conditions that control when the agent
//! loop terminates. Stop conditions are checked after each round of tool execution.
//!
//! # Built-in Conditions
//!
//! - [`MaxRounds`]: Stop after N tool-call rounds
//! - [`Timeout`]: Stop after a duration elapses
//! - [`TokenBudget`]: Stop when cumulative token usage exceeds a limit
//! - [`ConsecutiveErrors`]: Stop after N consecutive tool failures
//! - [`StopOnTool`]: Stop when a specific tool is called
//! - [`ContentMatch`]: Stop when LLM output contains a pattern
//! - [`LoopDetection`]: Stop when identical tool call patterns repeat
//!
//! # Custom Conditions
//!
//! Implement the [`StopCondition`] trait for custom logic:
//!
//! ```ignore
//! use carve_agent::stop::{StopCondition, StopCheckContext, StopReason};
//!
//! struct CostLimit { max_cents: usize }
//!
//! impl StopCondition for CostLimit {
//!     fn id(&self) -> &str { "cost_limit" }
//!     fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
//!         let estimated_cents = ctx.total_input_tokens / 1000 + ctx.total_output_tokens / 500;
//!         if estimated_cents >= self.max_cents {
//!             Some(StopReason::Custom("Cost limit exceeded".into()))
//!         } else {
//!             None
//!         }
//!     }
//! }
//! ```

use crate::session::Session;
use crate::types::ToolCall;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// StopReason
// ---------------------------------------------------------------------------

/// Why the agent loop stopped (normal termination, not errors).
///
/// This distinguishes legitimate stop conditions from actual errors like
/// `AgentLoopError::LlmError` or `AgentLoopError::StateError`.
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
    /// External cancellation signal received.
    Cancelled,
    /// Custom stop reason from a user-defined [`StopCondition`].
    Custom(String),
}

// ---------------------------------------------------------------------------
// StopCheckContext
// ---------------------------------------------------------------------------

/// Snapshot of loop state provided to stop condition checks.
///
/// Passed to [`StopCondition::check`] after each round so conditions can
/// inspect cumulative metrics and the most recent LLM response.
pub struct StopCheckContext<'a> {
    /// Number of completed tool-call rounds.
    pub rounds: usize,
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
    /// The current session, providing access to conversation history and state.
    ///
    /// Custom stop conditions can inspect `session.messages` for patterns,
    /// check `session.message_count()`, or examine the accumulated state.
    pub session: &'a Session,
}

// ---------------------------------------------------------------------------
// StopCondition trait
// ---------------------------------------------------------------------------

/// Trait for composable agent loop stop conditions.
///
/// Implementations are stored as `Arc<dyn StopCondition>` in [`AgentDefinition`]
/// and checked after each tool-call round.
pub trait StopCondition: Send + Sync {
    /// Unique identifier for this condition (used in logging/debugging).
    fn id(&self) -> &str;

    /// Check whether the loop should stop.
    ///
    /// Returns `Some(StopReason)` to terminate the loop, or `None` to continue.
    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason>;
}

/// Check all conditions in order, returning the first match.
pub fn check_stop_conditions(
    conditions: &[Arc<dyn StopCondition>],
    ctx: &StopCheckContext,
) -> Option<StopReason> {
    for condition in conditions {
        if let Some(reason) = condition.check(ctx) {
            return Some(reason);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Built-in conditions
// ---------------------------------------------------------------------------

/// Stop after a fixed number of tool-call rounds.
pub struct MaxRounds(pub usize);

impl StopCondition for MaxRounds {
    fn id(&self) -> &str {
        "max_rounds"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        if ctx.rounds >= self.0 {
            Some(StopReason::MaxRoundsReached)
        } else {
            None
        }
    }
}

/// Stop after a wall-clock duration elapses.
pub struct Timeout(pub Duration);

impl StopCondition for Timeout {
    fn id(&self) -> &str {
        "timeout"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        if ctx.elapsed >= self.0 {
            Some(StopReason::TimeoutReached)
        } else {
            None
        }
    }
}

/// Stop when cumulative token usage exceeds a budget.
pub struct TokenBudget {
    /// Maximum total tokens (input + output). 0 = unlimited.
    pub max_total: usize,
}

impl StopCondition for TokenBudget {
    fn id(&self) -> &str {
        "token_budget"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        if self.max_total > 0
            && (ctx.total_input_tokens + ctx.total_output_tokens) >= self.max_total
        {
            Some(StopReason::TokenBudgetExceeded)
        } else {
            None
        }
    }
}

/// Stop after N consecutive rounds where all tool executions failed.
pub struct ConsecutiveErrors(pub usize);

impl StopCondition for ConsecutiveErrors {
    fn id(&self) -> &str {
        "consecutive_errors"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        if self.0 > 0 && ctx.consecutive_errors >= self.0 {
            Some(StopReason::ConsecutiveErrorsExceeded)
        } else {
            None
        }
    }
}

/// Stop when a specific tool is called by the LLM.
pub struct StopOnTool(pub String);

impl StopCondition for StopOnTool {
    fn id(&self) -> &str {
        "stop_on_tool"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        for call in ctx.last_tool_calls {
            if call.name == self.0 {
                return Some(StopReason::ToolCalled(self.0.clone()));
            }
        }
        None
    }
}

/// Stop when LLM output text contains a literal pattern.
pub struct ContentMatch(pub String);

impl StopCondition for ContentMatch {
    fn id(&self) -> &str {
        "content_match"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        if !self.0.is_empty() && ctx.last_text.contains(&self.0) {
            Some(StopReason::ContentMatched(self.0.clone()))
        } else {
            None
        }
    }
}

/// Stop when the same tool call pattern repeats within a sliding window.
///
/// Compares the sorted tool names of the most recent round against previous
/// rounds within `window` size. If the same set appears twice consecutively,
/// the loop is considered stuck.
pub struct LoopDetection {
    /// Number of recent rounds to compare. Minimum 2.
    pub window: usize,
}

impl StopCondition for LoopDetection {
    fn id(&self) -> &str {
        "loop_detection"
    }

    fn check(&self, ctx: &StopCheckContext) -> Option<StopReason> {
        let window = self.window.max(2);
        let history = ctx.tool_call_history;
        if history.len() < 2 {
            return None;
        }

        // Look at the last `window` entries for consecutive duplicates.
        let recent: Vec<_> = history.iter().rev().take(window).collect();
        for pair in recent.windows(2) {
            if pair[0] == pair[1] {
                return Some(StopReason::LoopDetected);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::LazyLock;

    static TEST_SESSION: LazyLock<Session> = LazyLock::new(|| Session::new("test"));

    fn empty_context() -> StopCheckContext<'static> {
        static EMPTY_TOOL_CALLS: &[ToolCall] = &[];
        static EMPTY_HISTORY: VecDeque<Vec<String>> = VecDeque::new();
        StopCheckContext {
            rounds: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            consecutive_errors: 0,
            elapsed: Duration::ZERO,
            last_tool_calls: EMPTY_TOOL_CALLS,
            last_text: "",
            tool_call_history: &EMPTY_HISTORY,
            session: &TEST_SESSION,
        }
    }

    fn make_tool_call(name: &str) -> ToolCall {
        ToolCall {
            id: "tc-1".to_string(),
            name: name.to_string(),
            arguments: json!({}),
        }
    }

    // -- MaxRounds --

    #[test]
    fn max_rounds_none_when_under_limit() {
        let cond = MaxRounds(3);
        let mut ctx = empty_context();
        ctx.rounds = 2;
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn max_rounds_triggers_at_limit() {
        let cond = MaxRounds(3);
        let mut ctx = empty_context();
        ctx.rounds = 3;
        assert_eq!(cond.check(&ctx), Some(StopReason::MaxRoundsReached));
    }

    #[test]
    fn max_rounds_triggers_above_limit() {
        let cond = MaxRounds(3);
        let mut ctx = empty_context();
        ctx.rounds = 5;
        assert_eq!(cond.check(&ctx), Some(StopReason::MaxRoundsReached));
    }

    // -- Timeout --

    #[test]
    fn timeout_none_when_under() {
        let cond = Timeout(Duration::from_secs(10));
        let mut ctx = empty_context();
        ctx.elapsed = Duration::from_secs(5);
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn timeout_triggers_at_limit() {
        let cond = Timeout(Duration::from_secs(10));
        let mut ctx = empty_context();
        ctx.elapsed = Duration::from_secs(10);
        assert_eq!(cond.check(&ctx), Some(StopReason::TimeoutReached));
    }

    // -- TokenBudget --

    #[test]
    fn token_budget_none_when_under() {
        let cond = TokenBudget { max_total: 1000 };
        let mut ctx = empty_context();
        ctx.total_input_tokens = 400;
        ctx.total_output_tokens = 500;
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn token_budget_triggers_at_limit() {
        let cond = TokenBudget { max_total: 1000 };
        let mut ctx = empty_context();
        ctx.total_input_tokens = 600;
        ctx.total_output_tokens = 400;
        assert_eq!(cond.check(&ctx), Some(StopReason::TokenBudgetExceeded));
    }

    #[test]
    fn token_budget_zero_means_unlimited() {
        let cond = TokenBudget { max_total: 0 };
        let mut ctx = empty_context();
        ctx.total_input_tokens = 999_999;
        ctx.total_output_tokens = 999_999;
        assert!(cond.check(&ctx).is_none());
    }

    // -- ConsecutiveErrors --

    #[test]
    fn consecutive_errors_none_when_under() {
        let cond = ConsecutiveErrors(3);
        let mut ctx = empty_context();
        ctx.consecutive_errors = 2;
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn consecutive_errors_triggers_at_limit() {
        let cond = ConsecutiveErrors(3);
        let mut ctx = empty_context();
        ctx.consecutive_errors = 3;
        assert_eq!(
            cond.check(&ctx),
            Some(StopReason::ConsecutiveErrorsExceeded)
        );
    }

    #[test]
    fn consecutive_errors_zero_means_disabled() {
        let cond = ConsecutiveErrors(0);
        let mut ctx = empty_context();
        ctx.consecutive_errors = 100;
        assert!(cond.check(&ctx).is_none());
    }

    // -- StopOnTool --

    #[test]
    fn stop_on_tool_none_when_not_called() {
        let cond = StopOnTool("finish".to_string());
        let calls = vec![make_tool_call("search")];
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_tool_calls: &calls,
            tool_call_history: &history,
            ..empty_context()
        };
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn stop_on_tool_triggers_when_called() {
        let cond = StopOnTool("finish".to_string());
        let calls = vec![make_tool_call("search"), make_tool_call("finish")];
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_tool_calls: &calls,
            tool_call_history: &history,
            ..empty_context()
        };
        assert_eq!(
            cond.check(&ctx),
            Some(StopReason::ToolCalled("finish".to_string()))
        );
    }

    // -- ContentMatch --

    #[test]
    fn content_match_none_when_absent() {
        let cond = ContentMatch("FINAL_ANSWER".to_string());
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_text: "Here is some text",
            tool_call_history: &history,
            ..empty_context()
        };
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn content_match_triggers_when_present() {
        let cond = ContentMatch("FINAL_ANSWER".to_string());
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_text: "The result is: FINAL_ANSWER: 42",
            tool_call_history: &history,
            ..empty_context()
        };
        assert_eq!(
            cond.check(&ctx),
            Some(StopReason::ContentMatched("FINAL_ANSWER".to_string()))
        );
    }

    #[test]
    fn content_match_empty_pattern_never_triggers() {
        let cond = ContentMatch(String::new());
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_text: "anything",
            tool_call_history: &history,
            ..empty_context()
        };
        assert!(cond.check(&ctx).is_none());
    }

    // -- LoopDetection --

    #[test]
    fn loop_detection_none_when_insufficient_history() {
        let cond = LoopDetection { window: 3 };
        let mut history = VecDeque::new();
        history.push_back(vec!["search".to_string()]);
        let ctx = StopCheckContext {
            tool_call_history: &history,
            ..empty_context()
        };
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn loop_detection_none_when_different_patterns() {
        let cond = LoopDetection { window: 3 };
        let mut history = VecDeque::new();
        history.push_back(vec!["search".to_string()]);
        history.push_back(vec!["calculate".to_string()]);
        history.push_back(vec!["write".to_string()]);
        let ctx = StopCheckContext {
            tool_call_history: &history,
            ..empty_context()
        };
        assert!(cond.check(&ctx).is_none());
    }

    #[test]
    fn loop_detection_triggers_on_consecutive_duplicate() {
        let cond = LoopDetection { window: 3 };
        let mut history = VecDeque::new();
        history.push_back(vec!["search".to_string()]);
        history.push_back(vec!["calculate".to_string()]);
        history.push_back(vec!["calculate".to_string()]);
        let ctx = StopCheckContext {
            tool_call_history: &history,
            ..empty_context()
        };
        assert_eq!(cond.check(&ctx), Some(StopReason::LoopDetected));
    }

    // -- check_stop_conditions --

    #[test]
    fn check_stop_conditions_returns_first_match() {
        let conditions: Vec<Arc<dyn StopCondition>> = vec![
            Arc::new(MaxRounds(5)),
            Arc::new(Timeout(Duration::from_secs(10))),
        ];
        let mut ctx = empty_context();
        ctx.rounds = 5;
        ctx.elapsed = Duration::from_secs(15);
        // MaxRounds is first, so it should win
        assert_eq!(
            check_stop_conditions(&conditions, &ctx),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[test]
    fn check_stop_conditions_returns_none_when_all_pass() {
        let conditions: Vec<Arc<dyn StopCondition>> = vec![
            Arc::new(MaxRounds(10)),
            Arc::new(Timeout(Duration::from_secs(60))),
        ];
        let mut ctx = empty_context();
        ctx.rounds = 3;
        ctx.elapsed = Duration::from_secs(5);
        assert!(check_stop_conditions(&conditions, &ctx).is_none());
    }

    #[test]
    fn check_stop_conditions_empty_always_none() {
        let conditions: Vec<Arc<dyn StopCondition>> = vec![];
        let ctx = empty_context();
        assert!(check_stop_conditions(&conditions, &ctx).is_none());
    }

    // -- StopReason serialization --

    #[test]
    fn stop_reason_serialization_roundtrip() {
        let reasons = vec![
            StopReason::NaturalEnd,
            StopReason::PluginRequested,
            StopReason::MaxRoundsReached,
            StopReason::TimeoutReached,
            StopReason::TokenBudgetExceeded,
            StopReason::ToolCalled("finish".to_string()),
            StopReason::ContentMatched("DONE".to_string()),
            StopReason::ConsecutiveErrorsExceeded,
            StopReason::LoopDetected,
            StopReason::Cancelled,
            StopReason::Custom("my_reason".to_string()),
        ];
        for reason in reasons {
            let json = serde_json::to_string(&reason).unwrap();
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(reason, back);
        }
    }

    // -- Custom StopCondition --

    struct AlwaysStop;
    impl StopCondition for AlwaysStop {
        fn id(&self) -> &str {
            "always_stop"
        }
        fn check(&self, _ctx: &StopCheckContext) -> Option<StopReason> {
            Some(StopReason::Custom("always".to_string()))
        }
    }

    #[test]
    fn custom_stop_condition_works() {
        let conditions: Vec<Arc<dyn StopCondition>> = vec![Arc::new(AlwaysStop)];
        let ctx = empty_context();
        assert_eq!(
            check_stop_conditions(&conditions, &ctx),
            Some(StopReason::Custom("always".to_string()))
        );
    }
}
