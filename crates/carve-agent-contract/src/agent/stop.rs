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
//! use carve_agent::engine::stop_conditions::{StopCondition, StopCheckContext, StopReason};
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

use crate::state::{AgentState, ToolCall};
pub use crate::StopReason;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// StopReason
// ---------------------------------------------------------------------------

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
    /// The current thread, providing access to conversation history and state.
    ///
    /// Custom stop conditions can inspect `thread.messages` for patterns,
    /// check `thread.message_count()`, or examine the accumulated state.
    pub thread: &'a AgentState,
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

/// Internal test helper; runtime loop owns public condition evaluation.
#[cfg(test)]
pub(crate) fn check_stop_conditions(
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
// StopConditionSpec – serializable declarative stop conditions
// ---------------------------------------------------------------------------

/// Declarative, serializable representation of a built-in stop condition.
///
/// Analogous to `plugin_ids` for plugins: store specs in configuration,
/// resolve to `Arc<dyn StopCondition>` at runtime via [`into_condition`].
///
/// ```
/// use carve_agent_contract::agent::stop::StopConditionSpec;
///
/// let spec = StopConditionSpec::MaxRounds { rounds: 5 };
/// let json = serde_json::to_string(&spec).unwrap();
/// assert_eq!(json, r#"{"type":"max_rounds","rounds":5}"#);
///
/// let condition = spec.into_condition();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

impl StopConditionSpec {
    /// Convert this spec into a live `Arc<dyn StopCondition>`.
    pub fn into_condition(self) -> Arc<dyn StopCondition> {
        match self {
            Self::MaxRounds { rounds } => Arc::new(MaxRounds(rounds)),
            Self::Timeout { seconds } => Arc::new(Timeout(Duration::from_secs(seconds))),
            Self::TokenBudget { max_total } => Arc::new(TokenBudget { max_total }),
            Self::ConsecutiveErrors { max } => Arc::new(ConsecutiveErrors(max)),
            Self::StopOnTool { tool_name } => Arc::new(StopOnTool(tool_name)),
            Self::ContentMatch { pattern } => Arc::new(ContentMatch(pattern)),
            Self::LoopDetection { window } => Arc::new(LoopDetection { window }),
        }
    }
}

/// Convert built-in conditions back to specs for introspection.
impl From<&MaxRounds> for StopConditionSpec {
    fn from(c: &MaxRounds) -> Self {
        Self::MaxRounds { rounds: c.0 }
    }
}

impl From<&Timeout> for StopConditionSpec {
    fn from(c: &Timeout) -> Self {
        Self::Timeout {
            seconds: c.0.as_secs(),
        }
    }
}

impl From<&TokenBudget> for StopConditionSpec {
    fn from(c: &TokenBudget) -> Self {
        Self::TokenBudget {
            max_total: c.max_total,
        }
    }
}

impl From<&ConsecutiveErrors> for StopConditionSpec {
    fn from(c: &ConsecutiveErrors) -> Self {
        Self::ConsecutiveErrors { max: c.0 }
    }
}

impl From<&StopOnTool> for StopConditionSpec {
    fn from(c: &StopOnTool) -> Self {
        Self::StopOnTool {
            tool_name: c.0.clone(),
        }
    }
}

impl From<&ContentMatch> for StopConditionSpec {
    fn from(c: &ContentMatch) -> Self {
        Self::ContentMatch {
            pattern: c.0.clone(),
        }
    }
}

impl From<&LoopDetection> for StopConditionSpec {
    fn from(c: &LoopDetection) -> Self {
        Self::LoopDetection { window: c.window }
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

    static TEST_SESSION: LazyLock<AgentState> = LazyLock::new(|| AgentState::new("test"));

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
            thread: &TEST_SESSION,
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
        let cond = spec.into_condition();
        assert_eq!(cond.id(), "max_rounds");
        let mut ctx = empty_context();
        ctx.rounds = 3;
        assert_eq!(cond.check(&ctx), Some(StopReason::MaxRoundsReached));
    }

    #[test]
    fn stop_condition_spec_into_condition_timeout() {
        let spec = StopConditionSpec::Timeout { seconds: 10 };
        let cond = spec.into_condition();
        assert_eq!(cond.id(), "timeout");
        let mut ctx = empty_context();
        ctx.elapsed = Duration::from_secs(10);
        assert_eq!(cond.check(&ctx), Some(StopReason::TimeoutReached));
    }

    #[test]
    fn stop_condition_spec_into_condition_token_budget() {
        let spec = StopConditionSpec::TokenBudget { max_total: 100 };
        let cond = spec.into_condition();
        let mut ctx = empty_context();
        ctx.total_input_tokens = 60;
        ctx.total_output_tokens = 50;
        assert_eq!(cond.check(&ctx), Some(StopReason::TokenBudgetExceeded));
    }

    #[test]
    fn stop_condition_spec_into_condition_stop_on_tool() {
        let spec = StopConditionSpec::StopOnTool {
            tool_name: "finish".to_string(),
        };
        let cond = spec.into_condition();
        let calls = vec![make_tool_call("finish")];
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

    #[test]
    fn stop_condition_spec_from_builtin_roundtrip() {
        let spec = StopConditionSpec::from(&MaxRounds(5));
        assert_eq!(spec, StopConditionSpec::MaxRounds { rounds: 5 });

        let spec = StopConditionSpec::from(&Timeout(Duration::from_secs(30)));
        assert_eq!(spec, StopConditionSpec::Timeout { seconds: 30 });

        let spec = StopConditionSpec::from(&TokenBudget { max_total: 1000 });
        assert_eq!(spec, StopConditionSpec::TokenBudget { max_total: 1000 });

        let spec = StopConditionSpec::from(&ConsecutiveErrors(3));
        assert_eq!(spec, StopConditionSpec::ConsecutiveErrors { max: 3 });

        let spec = StopConditionSpec::from(&StopOnTool("done".to_string()));
        assert_eq!(
            spec,
            StopConditionSpec::StopOnTool {
                tool_name: "done".to_string()
            }
        );

        let spec = StopConditionSpec::from(&ContentMatch("FINAL".to_string()));
        assert_eq!(
            spec,
            StopConditionSpec::ContentMatch {
                pattern: "FINAL".to_string()
            }
        );

        let spec = StopConditionSpec::from(&LoopDetection { window: 4 });
        assert_eq!(spec, StopConditionSpec::LoopDetection { window: 4 });
    }
}
