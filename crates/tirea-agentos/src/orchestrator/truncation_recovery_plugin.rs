//! Truncation recovery behavior.
//!
//! When the LLM stops due to `MaxTokens` without emitting complete tool
//! calls, this plugin signals the loop to re-run inference with a
//! continuation prompt — letting the model resume from where it left off.
//!
//! Inspired by Claude Code's output-token recovery mechanism.

use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use tirea_contract::runtime::behavior::ReadOnlyContext;
use tirea_contract::runtime::inference::StopReason;
use tirea_contract::runtime::phase::{ActionSet, AfterInferenceAction};
use tirea_contract::thread::Message;

/// Behavior ID used for registration.
pub const TRUNCATION_RECOVERY_PLUGIN_ID: &str = "truncation_recovery";

/// Default continuation prompt sent to the model after truncation.
const DEFAULT_CONTINUATION_PROMPT: &str =
    "Your response was cut off because it exceeded the output token limit. \
     Please break your work into smaller pieces. Continue from where you left off.";

/// Plugin that detects output truncation and triggers automatic retry.
///
/// When `after_inference` sees `StopReason::MaxTokens` with no complete
/// tool calls, it emits [`AfterInferenceAction::RetryInference`] to
/// re-enter inference with the truncated response preserved in history.
#[derive(Debug)]
pub struct TruncationRecoveryPlugin {
    max_retries: usize,
    continuation_prompt: String,
    recovery_count: AtomicUsize,
}

impl TruncationRecoveryPlugin {
    /// Create with custom settings.
    pub fn new(max_retries: usize, continuation_prompt: impl Into<String>) -> Self {
        Self {
            max_retries,
            continuation_prompt: continuation_prompt.into(),
            recovery_count: AtomicUsize::new(0),
        }
    }

    /// Current recovery count (for observability / testing).
    pub fn recovery_count(&self) -> usize {
        self.recovery_count.load(Ordering::Relaxed)
    }
}

impl Default for TruncationRecoveryPlugin {
    fn default() -> Self {
        Self::new(3, DEFAULT_CONTINUATION_PROMPT)
    }
}

#[async_trait]
impl tirea_contract::runtime::AgentBehavior for TruncationRecoveryPlugin {
    fn id(&self) -> &str {
        TRUNCATION_RECOVERY_PLUGIN_ID
    }

    async fn after_inference(
        &self,
        ctx: &ReadOnlyContext<'_>,
    ) -> ActionSet<AfterInferenceAction> {
        let Some(result) = ctx.response() else {
            return ActionSet::empty();
        };

        // Three conditions for recovery:
        // 1. Model stopped due to max_tokens
        // 2. No complete tool calls (if there are, execute them first)
        // 3. Haven't exceeded max retries
        if result.stop_reason == Some(StopReason::MaxTokens)
            && result.tool_calls.is_empty()
            && self.recovery_count.load(Ordering::Relaxed) < self.max_retries
        {
            self.recovery_count.fetch_add(1, Ordering::Relaxed);

            // Include the truncated assistant response so the model sees
            // its own context, then add the continuation prompt.
            let messages = vec![
                Message::assistant(&result.text),
                Message::user(&self.continuation_prompt),
            ];

            ActionSet::single(AfterInferenceAction::RetryInference { messages })
        } else {
            ActionSet::empty()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::runtime::inference::{StopReason, StreamResult, TokenUsage, LLMResponse};
    use tirea_contract::runtime::phase::Phase;
    use tirea_contract::RunConfig;
    use tirea_state::DocCell;

    fn stream_result_with_stop(stop: StopReason) -> StreamResult {
        StreamResult {
            text: "partial output...".into(),
            tool_calls: vec![],
            usage: Some(TokenUsage {
                completion_tokens: Some(4096),
                ..Default::default()
            }),
            stop_reason: Some(stop),
        }
    }

    fn stream_result_with_tools() -> StreamResult {
        use tirea_contract::thread::ToolCall;
        StreamResult {
            text: "Using tool".into(),
            tool_calls: vec![ToolCall::new("c1", "search", json!({"q": "test"}))],
            usage: None,
            stop_reason: Some(StopReason::MaxTokens),
        }
    }

    #[tokio::test]
    async fn triggers_retry_on_max_tokens_without_tools() {
        let plugin = TruncationRecoveryPlugin::default();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let llm = LLMResponse::success(stream_result_with_stop(StopReason::MaxTokens));
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
            .with_llm_response(&llm);

        use tirea_contract::runtime::AgentBehavior;
        let actions = plugin.after_inference(&ctx).await;
        assert_eq!(actions.len(), 1);
        assert_eq!(plugin.recovery_count(), 1);
    }

    #[tokio::test]
    async fn no_retry_on_end_turn() {
        let plugin = TruncationRecoveryPlugin::default();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let llm = LLMResponse::success(stream_result_with_stop(StopReason::EndTurn));
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
            .with_llm_response(&llm);

        use tirea_contract::runtime::AgentBehavior;
        let actions = plugin.after_inference(&ctx).await;
        assert!(actions.is_empty());
        assert_eq!(plugin.recovery_count(), 0);
    }

    #[tokio::test]
    async fn no_retry_when_tool_calls_present() {
        let plugin = TruncationRecoveryPlugin::default();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let llm = LLMResponse::success(stream_result_with_tools());
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
            .with_llm_response(&llm);

        use tirea_contract::runtime::AgentBehavior;
        let actions = plugin.after_inference(&ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn respects_max_retries() {
        let plugin = TruncationRecoveryPlugin::new(2, "Continue.");
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let llm = LLMResponse::success(stream_result_with_stop(StopReason::MaxTokens));

        use tirea_contract::runtime::AgentBehavior;

        // First two retries succeed
        for _ in 0..2 {
            let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
                .with_llm_response(&llm);
            let actions = plugin.after_inference(&ctx).await;
            assert_eq!(actions.len(), 1);
        }

        // Third attempt is blocked
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
            .with_llm_response(&llm);
        let actions = plugin.after_inference(&ctx).await;
        assert!(actions.is_empty());
        assert_eq!(plugin.recovery_count(), 2);
    }

    #[tokio::test]
    async fn retry_messages_contain_truncated_response_and_prompt() {
        let plugin = TruncationRecoveryPlugin::new(3, "Please continue.");
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let llm = LLMResponse::success(stream_result_with_stop(StopReason::MaxTokens));
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc)
            .with_llm_response(&llm);

        use tirea_contract::runtime::AgentBehavior;
        let actions = plugin.after_inference(&ctx).await;
        let action = actions.into_vec().pop().unwrap();
        match action {
            AfterInferenceAction::RetryInference { messages } => {
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].content, "partial output...");
                assert_eq!(
                    messages[0].role,
                    tirea_contract::thread::Role::Assistant
                );
                assert_eq!(messages[1].content, "Please continue.");
                assert_eq!(
                    messages[1].role,
                    tirea_contract::thread::Role::User
                );
            }
            _ => panic!("expected RetryInference"),
        }
    }

    #[tokio::test]
    async fn no_action_without_llm_response() {
        let plugin = TruncationRecoveryPlugin::default();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::AfterInference, "t1", &[], &config, &doc);

        use tirea_contract::runtime::AgentBehavior;
        let actions = plugin.after_inference(&ctx).await;
        assert!(actions.is_empty());
    }
}
