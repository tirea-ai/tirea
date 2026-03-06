//! Truncation recovery logic for the agent loop.
//!
//! When the LLM stops due to `MaxTokens` without emitting complete tool
//! calls, this module provides helpers to inject a continuation prompt and
//! re-enter inference.

use super::run_state::LoopRunState;
use crate::contracts::runtime::inference::{StopReason, StreamResult};
use crate::contracts::thread::{Message, Visibility};

/// Maximum number of truncation recovery retries per run.
const MAX_RETRIES: usize = 3;

/// Continuation prompt sent to the model after truncation.
const CONTINUATION_PROMPT: &str =
    "Your response was cut off because it exceeded the output token limit. \
     Please break your work into smaller pieces. Continue from where you left off.";

/// Check if truncation recovery should retry inference.
///
/// Returns `true` (and increments the retry counter) when all three
/// conditions are met:
/// 1. Model stopped due to `MaxTokens`
/// 2. No complete tool calls in the response
/// 3. Haven't exceeded max retries
pub(super) fn should_retry(result: &StreamResult, run_state: &mut LoopRunState) -> bool {
    if result.stop_reason == Some(StopReason::MaxTokens)
        && result.tool_calls.is_empty()
        && run_state.truncation_retries < MAX_RETRIES
    {
        run_state.truncation_retries += 1;
        true
    } else {
        false
    }
}

/// Build the continuation prompt message (Internal visibility).
pub(super) fn continuation_message() -> Message {
    let mut msg = Message::user(CONTINUATION_PROMPT);
    msg.visibility = Visibility::Internal;
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::runtime::inference::TokenUsage;
    use crate::contracts::thread::ToolCall;
    use serde_json::json;

    // =====================================================================
    // Helpers
    // =====================================================================

    fn max_tokens_result() -> StreamResult {
        StreamResult {
            text: "partial output...".into(),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                completion_tokens: Some(4096),
                ..Default::default()
            }),
            stop_reason: Some(StopReason::MaxTokens),
        }
    }

    fn end_turn_result() -> StreamResult {
        StreamResult {
            text: "done".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        }
    }

    fn max_tokens_with_tools() -> StreamResult {
        StreamResult {
            text: "Using tool".into(),
            tool_calls: vec![ToolCall::new("c1", "search", json!({"q": "test"}))],
            usage: None,
            stop_reason: Some(StopReason::MaxTokens),
        }
    }

    fn tool_use_result() -> StreamResult {
        StreamResult {
            text: String::new(),
            tool_calls: vec![ToolCall::new("c1", "read_file", json!({"path": "/tmp"}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        }
    }

    fn stop_sequence_result() -> StreamResult {
        StreamResult {
            text: "stopped at sequence".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::StopSequence),
        }
    }

    fn no_stop_reason_result() -> StreamResult {
        StreamResult {
            text: "unknown stop".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
        }
    }

    fn empty_text_max_tokens() -> StreamResult {
        StreamResult {
            text: String::new(),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                completion_tokens: Some(4096),
                ..Default::default()
            }),
            stop_reason: Some(StopReason::MaxTokens),
        }
    }

    // =====================================================================
    // Core should_retry tests
    // =====================================================================

    #[test]
    fn triggers_retry_on_max_tokens_without_tools() {
        let mut state = LoopRunState::new();
        assert!(should_retry(&max_tokens_result(), &mut state));
        assert_eq!(state.truncation_retries, 1);
    }

    #[test]
    fn no_retry_on_end_turn() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&end_turn_result(), &mut state));
        assert_eq!(state.truncation_retries, 0);
    }

    #[test]
    fn no_retry_when_tool_calls_present() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&max_tokens_with_tools(), &mut state));
        assert_eq!(state.truncation_retries, 0);
    }

    #[test]
    fn no_retry_on_tool_use_stop() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&tool_use_result(), &mut state));
        assert_eq!(state.truncation_retries, 0);
    }

    #[test]
    fn no_retry_on_stop_sequence() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&stop_sequence_result(), &mut state));
        assert_eq!(state.truncation_retries, 0);
    }

    #[test]
    fn no_retry_when_stop_reason_is_none() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&no_stop_reason_result(), &mut state));
        assert_eq!(state.truncation_retries, 0);
    }

    #[test]
    fn retries_on_empty_text_max_tokens() {
        let mut state = LoopRunState::new();
        assert!(should_retry(&empty_text_max_tokens(), &mut state));
        assert_eq!(state.truncation_retries, 1);
    }

    // =====================================================================
    // Counter behavior
    // =====================================================================

    #[test]
    fn respects_max_retries() {
        let mut state = LoopRunState::new();
        for i in 0..MAX_RETRIES {
            assert!(
                should_retry(&max_tokens_result(), &mut state),
                "retry {i} should succeed"
            );
        }
        assert!(
            !should_retry(&max_tokens_result(), &mut state),
            "retry after max should fail"
        );
        assert_eq!(state.truncation_retries, MAX_RETRIES);
    }

    #[test]
    fn max_retries_is_three() {
        assert_eq!(MAX_RETRIES, 3);
    }

    #[test]
    fn counter_not_incremented_on_non_retry() {
        let mut state = LoopRunState::new();
        assert!(!should_retry(&end_turn_result(), &mut state));
        assert!(!should_retry(&tool_use_result(), &mut state));
        assert!(!should_retry(&stop_sequence_result(), &mut state));
        assert!(!should_retry(&no_stop_reason_result(), &mut state));
        assert!(!should_retry(&max_tokens_with_tools(), &mut state));
        assert_eq!(
            state.truncation_retries, 0,
            "counter should remain 0 after non-retry calls"
        );
    }

    #[test]
    fn counter_increments_only_on_actual_retry() {
        let mut state = LoopRunState::new();
        // Non-retry calls
        should_retry(&end_turn_result(), &mut state);
        should_retry(&tool_use_result(), &mut state);
        assert_eq!(state.truncation_retries, 0);

        // Actual retry
        should_retry(&max_tokens_result(), &mut state);
        assert_eq!(state.truncation_retries, 1);

        // Non-retry again
        should_retry(&end_turn_result(), &mut state);
        assert_eq!(state.truncation_retries, 1);

        // Another retry
        should_retry(&max_tokens_result(), &mut state);
        assert_eq!(state.truncation_retries, 2);
    }

    // =====================================================================
    // Mixed sequences
    // =====================================================================

    #[test]
    fn truncation_then_normal_end() {
        let mut state = LoopRunState::new();
        // First call truncated
        assert!(should_retry(&max_tokens_result(), &mut state));
        assert_eq!(state.truncation_retries, 1);

        // Model responds normally on retry
        assert!(!should_retry(&end_turn_result(), &mut state));
        // Counter stays at 1 (not reset)
        assert_eq!(state.truncation_retries, 1);
    }

    #[test]
    fn truncation_then_tool_use() {
        let mut state = LoopRunState::new();
        assert!(should_retry(&max_tokens_result(), &mut state));
        // On retry, model emits tool calls
        assert!(!should_retry(&tool_use_result(), &mut state));
        assert_eq!(state.truncation_retries, 1);
    }

    #[test]
    fn exhaust_retries_then_truncation_is_refused() {
        let mut state = LoopRunState::new();
        for _ in 0..MAX_RETRIES {
            assert!(should_retry(&max_tokens_result(), &mut state));
        }
        // Even with MaxTokens + no tools, retry is refused
        assert!(!should_retry(&max_tokens_result(), &mut state));
        assert!(!should_retry(&max_tokens_result(), &mut state));
        assert_eq!(state.truncation_retries, MAX_RETRIES);
    }

    // =====================================================================
    // continuation_message tests
    // =====================================================================

    #[test]
    fn continuation_message_is_internal() {
        let msg = continuation_message();
        assert_eq!(msg.visibility, Visibility::Internal);
        assert_eq!(msg.role, crate::contracts::thread::Role::User);
        assert!(!msg.content.is_empty());
    }

    #[test]
    fn continuation_message_mentions_token_limit() {
        let msg = continuation_message();
        assert!(
            msg.content.contains("output token limit"),
            "should explain truncation cause"
        );
    }

    #[test]
    fn continuation_message_asks_to_continue() {
        let msg = continuation_message();
        assert!(
            msg.content.contains("Continue"),
            "should instruct model to continue"
        );
    }

    #[test]
    fn continuation_message_is_deterministic() {
        let msg1 = continuation_message();
        let msg2 = continuation_message();
        assert_eq!(msg1.content, msg2.content);
        assert_eq!(msg1.visibility, msg2.visibility);
        assert_eq!(msg1.role, msg2.role);
    }
}
