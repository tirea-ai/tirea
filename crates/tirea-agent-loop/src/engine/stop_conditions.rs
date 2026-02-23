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
//! Implement the [`StopPolicy`] trait for custom logic:
//!
//! ```ignore
//! use tirea::contracts::runtime::{StopPolicy, StopPolicyInput, StopReason};
//!
//! struct CostLimit { max_cents: usize }
//!
//! impl StopPolicy for CostLimit {
//!     fn id(&self) -> &str { "cost_limit" }
//!     fn evaluate(&self, input: &StopPolicyInput<'_>) -> Option<StopReason> {
//!         let estimated_cents =
//!             input.stats.total_input_tokens / 1000 + input.stats.total_output_tokens / 500;
//!         if estimated_cents >= self.max_cents {
//!             Some(StopReason::Custom("Cost limit exceeded".into()))
//!         } else {
//!             None
//!         }
//!     }
//! }
//! ```

use crate::contracts::runtime::{StopPolicy, StopPolicyInput, StopPolicyStats};
use crate::contracts::thread::ToolCall;
use crate::contracts::RunContext;
use crate::contracts::StopConditionSpec;
pub use crate::contracts::StopReason;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// StopReason
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// StopCheckContext
// ---------------------------------------------------------------------------

/// Snapshot of loop state provided to stop checks.
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
    /// The current run context, providing access to conversation history and state.
    ///
    /// Custom stop conditions can inspect messages for patterns or examine
    /// the accumulated state via `rebuild_state()`.
    pub run_ctx: &'a RunContext,
}

impl<'a> StopCheckContext<'a> {
    /// Convert legacy context shape to canonical stop-policy input.
    pub fn as_policy_input(&'a self) -> StopPolicyInput<'a> {
        StopPolicyInput {
            run_ctx: self.run_ctx,
            stats: StopPolicyStats {
                step: self.rounds,
                step_tool_call_count: self.last_tool_calls.len(),
                total_tool_call_count: self.tool_call_history.iter().map(std::vec::Vec::len).sum(),
                total_input_tokens: self.total_input_tokens,
                total_output_tokens: self.total_output_tokens,
                consecutive_errors: self.consecutive_errors,
                elapsed: self.elapsed,
                last_tool_calls: self.last_tool_calls,
                last_text: self.last_text,
                tool_call_history: self.tool_call_history,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Stop policy helpers
// ---------------------------------------------------------------------------

fn stop_check_context_from_policy_input<'a>(
    input: &'a StopPolicyInput<'a>,
) -> StopCheckContext<'a> {
    StopCheckContext {
        rounds: input.stats.step,
        total_input_tokens: input.stats.total_input_tokens,
        total_output_tokens: input.stats.total_output_tokens,
        consecutive_errors: input.stats.consecutive_errors,
        elapsed: input.stats.elapsed,
        last_tool_calls: input.stats.last_tool_calls,
        last_text: input.stats.last_text,
        tool_call_history: input.stats.tool_call_history,
        run_ctx: input.run_ctx,
    }
}

macro_rules! impl_stop_policy_via_check {
    ($ty:ty, $id:literal) => {
        impl StopPolicy for $ty {
            fn id(&self) -> &str {
                $id
            }

            fn evaluate(&self, input: &StopPolicyInput<'_>) -> Option<StopReason> {
                let ctx = stop_check_context_from_policy_input(input);
                self.check(&ctx)
            }
        }
    };
}

/// Evaluate canonical stop policies in declaration order and return the first match.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn check_stop_policies(
    conditions: &[Arc<dyn StopPolicy>],
    input: &StopPolicyInput<'_>,
) -> Option<StopReason> {
    for condition in conditions {
        if let Some(reason) = StopPolicy::evaluate(condition.as_ref(), input) {
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

impl MaxRounds {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        if ctx.rounds >= self.0 {
            Some(StopReason::MaxRoundsReached)
        } else {
            None
        }
    }
}

impl_stop_policy_via_check!(MaxRounds, "max_rounds");

/// Stop after a wall-clock duration elapses.
pub struct Timeout(pub Duration);

impl Timeout {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        if ctx.elapsed >= self.0 {
            Some(StopReason::TimeoutReached)
        } else {
            None
        }
    }
}

impl_stop_policy_via_check!(Timeout, "timeout");

/// Stop when cumulative token usage exceeds a budget.
pub struct TokenBudget {
    /// Maximum total tokens (input + output). 0 = unlimited.
    pub max_total: usize,
}

impl TokenBudget {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        if self.max_total > 0
            && (ctx.total_input_tokens + ctx.total_output_tokens) >= self.max_total
        {
            Some(StopReason::TokenBudgetExceeded)
        } else {
            None
        }
    }
}

impl_stop_policy_via_check!(TokenBudget, "token_budget");

/// Stop after N consecutive rounds where all tool executions failed.
pub struct ConsecutiveErrors(pub usize);

impl ConsecutiveErrors {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        if self.0 > 0 && ctx.consecutive_errors >= self.0 {
            Some(StopReason::ConsecutiveErrorsExceeded)
        } else {
            None
        }
    }
}

impl_stop_policy_via_check!(ConsecutiveErrors, "consecutive_errors");

/// Stop when a specific tool is called by the LLM.
pub struct StopOnTool(pub String);

impl StopOnTool {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        for call in ctx.last_tool_calls {
            if call.name == self.0 {
                return Some(StopReason::ToolCalled(self.0.clone()));
            }
        }
        None
    }
}

impl_stop_policy_via_check!(StopOnTool, "stop_on_tool");

/// Stop when LLM output text contains a literal pattern.
pub struct ContentMatch(pub String);

impl ContentMatch {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
        if !self.0.is_empty() && ctx.last_text.contains(&self.0) {
            Some(StopReason::ContentMatched(self.0.clone()))
        } else {
            None
        }
    }
}

impl_stop_policy_via_check!(ContentMatch, "content_match");

/// Stop when the same tool call pattern repeats within a sliding window.
///
/// Compares the sorted tool names of the most recent round against previous
/// rounds within `window` size. If the same set appears twice consecutively,
/// the loop is considered stuck.
pub struct LoopDetection {
    /// Number of recent rounds to compare. Minimum 2.
    pub window: usize,
}

impl LoopDetection {
    fn check(&self, ctx: &StopCheckContext<'_>) -> Option<StopReason> {
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

impl_stop_policy_via_check!(LoopDetection, "loop_detection");

// ---------------------------------------------------------------------------
// StopConditionSpec resolution
// ---------------------------------------------------------------------------

/// Resolve contract-level declarative stop condition spec to runtime evaluator.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn condition_from_spec(spec: StopConditionSpec) -> Arc<dyn StopPolicy> {
    match spec {
        StopConditionSpec::MaxRounds { rounds } => Arc::new(MaxRounds(rounds)),
        StopConditionSpec::Timeout { seconds } => Arc::new(Timeout(Duration::from_secs(seconds))),
        StopConditionSpec::TokenBudget { max_total } => Arc::new(TokenBudget { max_total }),
        StopConditionSpec::ConsecutiveErrors { max } => Arc::new(ConsecutiveErrors(max)),
        StopConditionSpec::StopOnTool { tool_name } => Arc::new(StopOnTool(tool_name)),
        StopConditionSpec::ContentMatch { pattern } => Arc::new(ContentMatch(pattern)),
        StopConditionSpec::LoopDetection { window } => Arc::new(LoopDetection { window }),
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
    use tirea_contract::RunConfig;

    static TEST_RUN_CTX: LazyLock<RunContext> =
        LazyLock::new(|| RunContext::new("test", json!({}), vec![], RunConfig::default()));

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
            run_ctx: &TEST_RUN_CTX,
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
        let conditions: Vec<Arc<dyn StopPolicy>> = vec![
            Arc::new(MaxRounds(5)),
            Arc::new(Timeout(Duration::from_secs(10))),
        ];
        let mut ctx = empty_context();
        ctx.rounds = 5;
        ctx.elapsed = Duration::from_secs(15);
        // MaxRounds is first, so it should win
        assert_eq!(
            check_stop_policies(&conditions, &ctx.as_policy_input()),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[test]
    fn check_stop_conditions_returns_none_when_all_pass() {
        let conditions: Vec<Arc<dyn StopPolicy>> = vec![
            Arc::new(MaxRounds(10)),
            Arc::new(Timeout(Duration::from_secs(60))),
        ];
        let mut ctx = empty_context();
        ctx.rounds = 3;
        ctx.elapsed = Duration::from_secs(5);
        assert!(check_stop_policies(&conditions, &ctx.as_policy_input()).is_none());
    }

    #[test]
    fn check_stop_conditions_empty_always_none() {
        let conditions: Vec<Arc<dyn StopPolicy>> = vec![];
        let ctx = empty_context();
        assert!(check_stop_policies(&conditions, &ctx.as_policy_input()).is_none());
    }

    // -- StopReason serialization --

    #[test]
    fn stop_reason_serialization_roundtrip() {
        let reasons = vec![
            StopReason::MaxRoundsReached,
            StopReason::TimeoutReached,
            StopReason::TokenBudgetExceeded,
            StopReason::ToolCalled("finish".to_string()),
            StopReason::ContentMatched("DONE".to_string()),
            StopReason::ConsecutiveErrorsExceeded,
            StopReason::LoopDetected,
            StopReason::Custom("my_reason".to_string()),
        ];
        for reason in reasons {
            let json = serde_json::to_string(&reason).unwrap();
            let back: StopReason = serde_json::from_str(&json).unwrap();
            assert_eq!(reason, back);
        }
    }

    // -- Custom StopPolicy --

    struct AlwaysStop;
    impl StopPolicy for AlwaysStop {
        fn id(&self) -> &str {
            "always_stop"
        }
        fn evaluate(&self, _input: &StopPolicyInput<'_>) -> Option<StopReason> {
            Some(StopReason::Custom("always".to_string()))
        }
    }

    #[test]
    fn custom_stop_policy_works() {
        let conditions: Vec<Arc<dyn StopPolicy>> = vec![Arc::new(AlwaysStop)];
        let ctx = empty_context();
        assert_eq!(
            check_stop_policies(&conditions, &ctx.as_policy_input()),
            Some(StopReason::Custom("always".to_string()))
        );
    }

    // -- StopConditionSpec --

    #[test]
    fn stop_condition_spec_serialization_roundtrip() {
        let specs = vec![
            StopConditionSpec::MaxRounds { rounds: 5 },
            StopConditionSpec::Timeout { seconds: 30 },
            StopConditionSpec::TokenBudget { max_total: 1000 },
            StopConditionSpec::ConsecutiveErrors { max: 3 },
            StopConditionSpec::StopOnTool {
                tool_name: "finish".to_string(),
            },
            StopConditionSpec::ContentMatch {
                pattern: "DONE".to_string(),
            },
            StopConditionSpec::LoopDetection { window: 4 },
        ];
        for spec in specs {
            let json = serde_json::to_string(&spec).unwrap();
            let back: StopConditionSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(spec, back);
        }
    }

    #[test]
    fn stop_condition_spec_json_format() {
        let spec = StopConditionSpec::MaxRounds { rounds: 5 };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, r#"{"type":"max_rounds","rounds":5}"#);

        let spec = StopConditionSpec::StopOnTool {
            tool_name: "done".to_string(),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, r#"{"type":"stop_on_tool","tool_name":"done"}"#);
    }

    #[test]
    fn stop_condition_spec_into_condition_max_rounds() {
        let spec = StopConditionSpec::MaxRounds { rounds: 3 };
        let cond = condition_from_spec(spec);
        assert_eq!(cond.id(), "max_rounds");
        let mut ctx = empty_context();
        ctx.rounds = 3;
        assert_eq!(
            cond.evaluate(&ctx.as_policy_input()),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[test]
    fn stop_condition_spec_into_condition_timeout() {
        let spec = StopConditionSpec::Timeout { seconds: 10 };
        let cond = condition_from_spec(spec);
        assert_eq!(cond.id(), "timeout");
        let mut ctx = empty_context();
        ctx.elapsed = Duration::from_secs(10);
        assert_eq!(
            cond.evaluate(&ctx.as_policy_input()),
            Some(StopReason::TimeoutReached)
        );
    }

    #[test]
    fn stop_condition_spec_into_condition_token_budget() {
        let spec = StopConditionSpec::TokenBudget { max_total: 100 };
        let cond = condition_from_spec(spec);
        let mut ctx = empty_context();
        ctx.total_input_tokens = 60;
        ctx.total_output_tokens = 50;
        assert_eq!(
            cond.evaluate(&ctx.as_policy_input()),
            Some(StopReason::TokenBudgetExceeded)
        );
    }

    #[test]
    fn stop_condition_spec_into_condition_stop_on_tool() {
        let spec = StopConditionSpec::StopOnTool {
            tool_name: "finish".to_string(),
        };
        let cond = condition_from_spec(spec);
        let calls = vec![make_tool_call("finish")];
        let history = VecDeque::new();
        let ctx = StopCheckContext {
            last_tool_calls: &calls,
            tool_call_history: &history,
            ..empty_context()
        };
        assert_eq!(
            cond.evaluate(&ctx.as_policy_input()),
            Some(StopReason::ToolCalled("finish".to_string()))
        );
    }
}
