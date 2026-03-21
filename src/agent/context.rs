//! Built-in context management: hard truncation, LLM compaction, summarizer trait.

use async_trait::async_trait;

use crate::contract::executor::LlmExecutor;
use crate::contract::inference::ContextWindowPolicy;
use crate::contract::message::{Message, Role, Visibility};
use crate::contract::tool::ToolDescriptor;
use crate::contract::transform::{
    InferenceRequestTransform, TransformOutput, estimate_message_tokens, estimate_tokens,
    estimate_tool_tokens, patch_dangling_tool_calls,
};

// ---------------------------------------------------------------------------
// ContextSummarizer trait
// ---------------------------------------------------------------------------

/// Abstraction for generating conversation summaries during compaction.
///
/// The framework provides token estimation, boundary finding, and transcript rendering.
/// Implementors decide the summarization strategy (prompt, model, parameters).
#[async_trait]
pub trait ContextSummarizer: Send + Sync {
    /// Generate a summary from a conversation transcript.
    ///
    /// - `transcript`: rendered text of the messages to summarize (Internal messages already filtered)
    /// - `previous_summary`: if a prior compaction summary exists, passed here for cumulative updates
    /// - `executor`: LLM executor to use for summarization
    async fn summarize(
        &self,
        transcript: &str,
        previous_summary: Option<&str>,
        executor: &dyn LlmExecutor,
    ) -> Result<String, String>;
}

/// Default summarizer with a domain-agnostic prompt.
///
/// Uses cumulative summarization: if a previous summary exists, the prompt asks
/// the LLM to update it with the new conversation span rather than re-summarize everything.
pub struct DefaultSummarizer {
    /// Maximum output tokens for the summary.
    pub max_tokens: u32,
}

impl Default for DefaultSummarizer {
    fn default() -> Self {
        Self { max_tokens: 1024 }
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = "\
You maintain a durable conversation summary for an agent runtime. \
Produce a concise but lossless working summary for future turns. \
Preserve user goals, constraints, preferences, decisions, completed work, \
important findings, identifiers, and unresolved follow-ups. \
Output plain text only; do not mention the summarization process.";

#[async_trait]
impl ContextSummarizer for DefaultSummarizer {
    async fn summarize(
        &self,
        transcript: &str,
        previous_summary: Option<&str>,
        executor: &dyn LlmExecutor,
    ) -> Result<String, String> {
        let user_prompt = match previous_summary {
            Some(prev) if !prev.trim().is_empty() => format!(
                "Update the cumulative summary with the new conversation span.\n\n\
                 <existing-summary>\n{}\n</existing-summary>\n\n\
                 <new-conversation>\n{}\n</new-conversation>",
                prev.trim(),
                transcript.trim(),
            ),
            _ => format!(
                "Summarize the following conversation:\n\n\
                 <conversation>\n{}\n</conversation>",
                transcript.trim(),
            ),
        };

        let request = crate::contract::executor::InferenceRequest {
            model: String::new(), // executor decides the model
            messages: vec![
                Message::system(DEFAULT_SYSTEM_PROMPT),
                Message::user(user_prompt),
            ],
            tools: vec![],
            system: vec![],
            overrides: Some(crate::contract::inference::InferenceOverride {
                max_tokens: Some(self.max_tokens),
                ..Default::default()
            }),
        };

        let result = executor
            .execute(request)
            .await
            .map_err(|e| format!("summarization failed: {e}"))?;

        let text = result.text();
        if text.is_empty() {
            return Err("empty summary".into());
        }
        Ok(text)
    }
}

/// Minimum token savings required to justify a compaction LLM call.
pub const MIN_COMPACTION_GAIN_TOKENS: usize = 1024;

/// Built-in request transform: truncate messages to fit the token budget.
///
/// Preserves all system messages and the most recent conversation messages.
/// Adjusts split points to avoid orphaning tool call/result pairs.
pub struct ContextTransform {
    policy: ContextWindowPolicy,
}

impl ContextTransform {
    pub fn new(policy: ContextWindowPolicy) -> Self {
        Self { policy }
    }
}

impl InferenceRequestTransform for ContextTransform {
    fn transform(
        &self,
        mut messages: Vec<Message>,
        tool_descriptors: &[ToolDescriptor],
    ) -> TransformOutput {
        let tool_tokens = estimate_tool_tokens(tool_descriptors);
        let available = self
            .policy
            .max_context_tokens
            .saturating_sub(self.policy.max_output_tokens)
            .saturating_sub(tool_tokens);

        let total = estimate_tokens(&messages);
        if total <= available {
            return TransformOutput { messages };
        }

        // Split into system prefix and history
        let system_end = messages
            .iter()
            .position(|m| m.role != Role::System)
            .unwrap_or(messages.len());

        let system_tokens: usize = messages[..system_end]
            .iter()
            .map(estimate_message_tokens)
            .sum();
        let history_budget = available.saturating_sub(system_tokens);

        // Find split point: walk backward from end, accumulating tokens
        let history = &messages[system_end..];
        let split = find_split_point(history, history_budget, self.policy.min_recent_messages);
        let absolute_split = system_end + split;

        // Remove truncated messages
        if absolute_split > system_end {
            messages.drain(system_end..absolute_split);
        }

        // Repair dangling tool calls after truncation
        patch_dangling_tool_calls(&mut messages);

        TransformOutput { messages }
    }
}

/// Find the split point in history that fits the token budget.
///
/// Always keeps at least `min_recent` messages from the end.
/// Adjusts boundaries to avoid splitting tool call/result pairs.
fn find_split_point(history: &[Message], budget_tokens: usize, min_recent: usize) -> usize {
    if history.is_empty() {
        return 0;
    }

    let must_keep = min_recent.min(history.len());
    let must_keep_start = history.len().saturating_sub(must_keep);

    let mut used_tokens = 0usize;
    let mut candidate_split = history.len();

    for i in (0..history.len()).rev() {
        let msg_tokens = estimate_message_tokens(&history[i]);
        let new_total = used_tokens + msg_tokens;

        if i >= must_keep_start {
            used_tokens = new_total;
            candidate_split = i;
            continue;
        }

        if new_total > budget_tokens {
            break;
        }

        used_tokens = new_total;
        candidate_split = i;
    }

    adjust_split_for_tool_pairs(history, candidate_split)
}

/// Adjust split to avoid orphaning tool call/result pairs.
fn adjust_split_for_tool_pairs(history: &[Message], mut split: usize) -> usize {
    // If first kept message is Tool, move split backward to include paired Assistant
    while split > 0 && history[split].role == Role::Tool {
        split -= 1;
    }

    // If last dropped message is Assistant with tool_calls,
    // move split forward to drop orphaned tool results
    if split > 0 {
        let last_dropped = &history[split - 1];
        if last_dropped.role == Role::Assistant && last_dropped.tool_calls.is_some() {
            while split < history.len() && history[split].role == Role::Tool {
                split += 1;
            }
        }
    }

    split
}

// ---------------------------------------------------------------------------
// LLM compaction
// ---------------------------------------------------------------------------

/// Find a safe compaction boundary in the message history.
///
/// Returns the index of the last message that can be safely compacted
/// (all tool call/result pairs are complete before this point).
pub fn find_compaction_boundary(
    messages: &[std::sync::Arc<Message>],
    start: usize,
    end: usize,
) -> Option<usize> {
    use std::collections::HashSet;

    let mut open_calls = HashSet::<String>::new();
    let mut best_boundary = None;

    for (idx, msg) in messages.iter().enumerate().skip(start).take(end - start) {
        if let Some(ref calls) = msg.tool_calls {
            for call in calls {
                open_calls.insert(call.id.clone());
            }
        }

        if msg.role == Role::Tool {
            if let Some(ref call_id) = msg.tool_call_id {
                open_calls.remove(call_id);
            }
        }

        // Safe boundary: all tool calls resolved and next isn't a tool result
        let next_is_tool = messages
            .get(idx + 1)
            .is_some_and(|next| next.role == Role::Tool);

        if open_calls.is_empty() && !next_is_tool {
            best_boundary = Some(idx);
        }
    }

    best_boundary
}

/// Render messages as a text transcript for LLM summarization.
///
/// Filters out `Visibility::Internal` messages — system-injected context that
/// gets re-injected each turn should not be included in the summary.
pub fn render_transcript(messages: &[std::sync::Arc<Message>]) -> String {
    messages
        .iter()
        .filter(|m| m.visibility != Visibility::Internal)
        .filter_map(|m| {
            let text = m.text();
            if text.is_empty() {
                return None;
            }
            let role = match m.role {
                Role::System => "System",
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::Tool => "Tool",
            };
            Some(format!("[{role}]: {text}"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Extract a previous compaction summary from the message list.
///
/// Looks for the first `internal_system` message containing `<conversation-summary>` tags.
pub fn extract_previous_summary(messages: &[std::sync::Arc<Message>]) -> Option<String> {
    for msg in messages {
        if msg.role != Role::System || msg.visibility != Visibility::Internal {
            continue;
        }
        let text = msg.text();
        if let Some(start) = text.find("<conversation-summary>") {
            if let Some(end) = text.find("</conversation-summary>") {
                let inner = &text[start + "<conversation-summary>".len()..end];
                let trimmed = inner.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Load-time trim
// ---------------------------------------------------------------------------

/// Trim loaded messages to the latest compaction boundary.
///
/// If the message list contains a `<conversation-summary>` internal_system message,
/// all messages before it are dropped. The summary message becomes the first message.
/// This avoids loading already-summarized history into the context window.
///
/// Idempotent: if no summary exists or messages are already trimmed, this is a no-op.
pub fn trim_to_compaction_boundary(messages: &mut Vec<std::sync::Arc<Message>>) {
    // Find the last summary message (in case of multiple compactions)
    let last_summary_idx = messages.iter().rposition(|m| {
        m.role == Role::System
            && m.visibility == Visibility::Internal
            && m.text().contains("<conversation-summary>")
    });

    if let Some(idx) = last_summary_idx {
        if idx > 0 {
            messages.drain(..idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::content::ContentBlock;
    use crate::contract::message::ToolCall;
    use serde_json::json;

    fn make_policy(max_tokens: usize, min_recent: usize) -> ContextWindowPolicy {
        ContextWindowPolicy {
            max_context_tokens: max_tokens,
            max_output_tokens: 0,
            min_recent_messages: min_recent,
            enable_prompt_cache: false,
            autocompact_threshold: None,
            compaction_mode: Default::default(),
            compaction_raw_suffix_messages: 2,
        }
    }

    #[test]
    fn truncation_preserves_all_when_under_budget() {
        let transform = ContextTransform::new(make_policy(100_000, 2));
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        let output = transform.transform(messages.clone(), &[]);
        assert_eq!(output.messages.len(), 3);
    }

    #[test]
    fn truncation_keeps_system_and_recent() {
        // Very tight budget: system + ~2 recent messages
        let transform = ContextTransform::new(make_policy(50, 2));
        let mut messages = vec![Message::system("sys")];
        // Add many user/assistant turns
        for i in 0..20 {
            messages.push(Message::user(format!("msg {i}")));
            messages.push(Message::assistant(format!("reply {i}")));
        }

        let output = transform.transform(messages, &[]);
        // Should have system + at least 2 recent messages
        assert!(output.messages.len() >= 3);
        assert_eq!(output.messages[0].role, Role::System);
    }

    #[test]
    fn truncation_repairs_dangling_tool_calls() {
        let transform = ContextTransform::new(make_policy(30, 1));
        let messages = vec![
            Message::system("sys"),
            Message::user("old msg 1"),
            Message::assistant_with_tool_calls(
                "calling",
                vec![ToolCall::new("c1", "search", json!({}))],
            ),
            Message::tool("c1", "result"),
            Message::user("old msg 2"),
            Message::assistant("old reply"),
            // many more to force truncation...
            Message::user("recent"),
            Message::assistant("recent reply"),
        ];

        let output = transform.transform(messages, &[]);
        // Should not have orphaned tool calls
        for (i, msg) in output.messages.iter().enumerate() {
            if msg.role == Role::Assistant {
                if let Some(ref calls) = msg.tool_calls {
                    for call in calls {
                        assert!(
                            output.messages[i + 1..]
                                .iter()
                                .any(|m| m.tool_call_id.as_deref() == Some(&call.id)),
                            "tool call {} should have a matching result",
                            call.id
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn find_compaction_boundary_respects_tool_pairs() {
        use std::sync::Arc;

        let messages: Vec<Arc<Message>> = vec![
            Arc::new(Message::user("start")),
            Arc::new(Message::assistant_with_tool_calls(
                "",
                vec![ToolCall::new("c1", "search", json!({}))],
            )),
            Arc::new(Message::tool("c1", "found")),
            Arc::new(Message::user("next")), // safe boundary here (idx 3)
            Arc::new(Message::assistant("reply")),
        ];

        let boundary = find_compaction_boundary(&messages, 0, messages.len());
        // Should be at idx 3 or 4 (after tool pair is complete)
        assert!(boundary.is_some());
        let b = boundary.unwrap();
        assert!(b >= 3);
    }

    #[test]
    fn render_transcript_formats_correctly() {
        use std::sync::Arc;

        let messages = vec![
            Arc::new(Message::user("hello")),
            Arc::new(Message::assistant("hi there")),
        ];
        let transcript = render_transcript(&messages);
        assert!(transcript.contains("[User]: hello"));
        assert!(transcript.contains("[Assistant]: hi there"));
    }

    #[test]
    fn render_transcript_excludes_internal_messages() {
        use std::sync::Arc;

        let messages = vec![
            Arc::new(Message::internal_system("you are helpful")),
            Arc::new(Message::user("hello")),
            Arc::new(Message::assistant("hi")),
        ];
        let transcript = render_transcript(&messages);
        assert!(!transcript.contains("you are helpful"));
        assert!(transcript.contains("[User]: hello"));
    }

    #[test]
    fn trim_to_compaction_boundary_drops_pre_summary() {
        use std::sync::Arc;

        let mut messages = vec![
            Arc::new(Message::user("old msg 1")),
            Arc::new(Message::assistant("old reply")),
            Arc::new(Message::internal_system(
                "<conversation-summary>\nSummary of old messages\n</conversation-summary>",
            )),
            Arc::new(Message::user("new msg")),
            Arc::new(Message::assistant("new reply")),
        ];

        trim_to_compaction_boundary(&mut messages);
        assert_eq!(messages.len(), 3);
        assert!(messages[0].text().contains("conversation-summary"));
        assert_eq!(messages[1].text(), "new msg");
    }

    #[test]
    fn trim_to_compaction_boundary_noop_without_summary() {
        use std::sync::Arc;

        let mut messages = vec![
            Arc::new(Message::user("hello")),
            Arc::new(Message::assistant("hi")),
        ];
        let len_before = messages.len();
        trim_to_compaction_boundary(&mut messages);
        assert_eq!(messages.len(), len_before);
    }

    #[test]
    fn extract_previous_summary_finds_summary() {
        use std::sync::Arc;

        let messages = vec![
            Arc::new(Message::internal_system(
                "<conversation-summary>\nPrevious summary text\n</conversation-summary>",
            )),
            Arc::new(Message::user("new msg")),
        ];
        let summary = extract_previous_summary(&messages);
        assert_eq!(summary.as_deref(), Some("Previous summary text"));
    }

    #[test]
    fn extract_previous_summary_none_without_summary() {
        use std::sync::Arc;

        let messages = vec![Arc::new(Message::user("hello"))];
        assert!(extract_previous_summary(&messages).is_none());
    }
}
