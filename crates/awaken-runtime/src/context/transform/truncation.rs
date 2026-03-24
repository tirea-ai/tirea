//! Message truncation: find split points that respect token budgets and tool-call pairing.

use awaken_contract::contract::message::{Message, Role};
use awaken_contract::contract::transform::estimate_message_tokens;

/// Find the split point in history that fits the token budget.
///
/// Always keeps at least `min_recent` messages from the end.
/// Adjusts boundaries to avoid splitting tool call/result pairs.
pub fn find_split_point(history: &[Message], budget_tokens: usize, min_recent: usize) -> usize {
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
pub fn adjust_split_for_tool_pairs(history: &[Message], mut split: usize) -> usize {
    if split == 0 || split >= history.len() {
        return split;
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::message::ToolCall;
    use serde_json::json;

    #[test]
    fn find_split_empty_history() {
        let split = find_split_point(&[], 1000, 5);
        assert_eq!(split, 0);
    }

    #[test]
    fn find_split_all_within_budget() {
        let history = vec![Message::user("Hi"), Message::assistant("Hello!")];
        let split = find_split_point(&history, 100_000, 2);
        assert_eq!(split, 0, "everything fits, nothing to drop");
    }

    #[test]
    fn find_split_drops_oldest_when_tight() {
        let history: Vec<Message> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    Message::user(format!("msg {i}"))
                } else {
                    Message::assistant(format!("reply {i}"))
                }
            })
            .collect();
        let split = find_split_point(&history, 30, 2);
        assert!(split > 0, "some messages should be dropped");
        let kept = history.len() - split;
        assert!(kept >= 2, "must keep at least min_recent=2");
    }

    #[test]
    fn find_split_respects_min_recent_even_beyond_budget() {
        let history: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("message {i} with padding")))
            .collect();
        let split = find_split_point(&history, 1, 5);
        let kept = history.len() - split;
        assert!(kept >= 5, "must keep at least min_recent=5, kept={kept}");
    }

    #[test]
    fn find_split_min_recent_exceeds_history_len() {
        let history = vec![Message::user("a"), Message::assistant("b")];
        let split = find_split_point(&history, 1, 100);
        assert_eq!(split, 0, "min_recent > len means keep all");
    }

    #[test]
    fn adjust_split_moves_back_for_orphaned_tool_result() {
        let history = vec![
            Message::user("a"),
            Message::assistant_with_tool_calls("b", vec![ToolCall::new("c1", "t", json!({}))]),
            Message::tool("c1", "result"),
            Message::user("c"),
        ];
        let adjusted = adjust_split_for_tool_pairs(&history, 2);
        assert_eq!(adjusted, 1, "should include the assistant with tool calls");
    }

    #[test]
    fn adjust_split_drops_orphaned_results_after_dropped_assistant() {
        let history = vec![
            Message::user("a"),
            Message::assistant_with_tool_calls("b", vec![ToolCall::new("c1", "t", json!({}))]),
            Message::tool("c1", "result"),
            Message::user("c"),
            Message::assistant("answer"),
        ];
        let adjusted = adjust_split_for_tool_pairs(&history, 3);
        assert_eq!(adjusted, 3, "split at user boundary should be stable");
    }

    #[test]
    fn adjust_split_handles_multiple_consecutive_tool_results() {
        let history = vec![
            Message::user("start"),
            Message::assistant_with_tool_calls(
                "calling two",
                vec![
                    ToolCall::new("c1", "t1", json!({})),
                    ToolCall::new("c2", "t2", json!({})),
                ],
            ),
            Message::tool("c1", "r1"),
            Message::tool("c2", "r2"),
            Message::user("continue"),
        ];
        assert_eq!(adjust_split_for_tool_pairs(&history, 2), 1);
        assert_eq!(adjust_split_for_tool_pairs(&history, 3), 1);
    }

    #[test]
    fn adjust_split_at_zero_is_noop() {
        let history = vec![Message::user("a"), Message::assistant("b")];
        assert_eq!(adjust_split_for_tool_pairs(&history, 0), 0);
    }

    #[test]
    fn adjust_split_at_len_minus_one_non_tool() {
        // When split is at the last element and it's not a Tool, no adjustment needed
        let history = vec![Message::user("a"), Message::assistant("b")];
        assert_eq!(adjust_split_for_tool_pairs(&history, 1), 1);
    }

    #[test]
    fn adjust_split_at_end_returns_len() {
        let history = vec![Message::user("a"), Message::assistant("b")];
        // split == history.len() means keep nothing; guard returns early
        assert_eq!(adjust_split_for_tool_pairs(&history, 2), 2);
    }

    #[test]
    fn adjust_split_backward_then_no_forward() {
        let history = vec![
            Message::user("start"),
            Message::assistant_with_tool_calls(
                "calling",
                vec![ToolCall::new("c1", "t", json!({}))],
            ),
            Message::tool("c1", "result"),
            Message::user("next"),
        ];
        let adjusted = adjust_split_for_tool_pairs(&history, 2);
        assert_eq!(adjusted, 1);
    }

    #[test]
    fn find_split_tool_pair_not_broken() {
        let history = vec![
            Message::user("old1"),
            Message::assistant("old_reply"),
            Message::user("Do something"),
            Message::assistant_with_tool_calls(
                "Using tool",
                vec![ToolCall::new("c1", "search", json!({"q": "x"}))],
            ),
            Message::tool("c1", "found it"),
            Message::assistant("Here is the answer."),
            Message::user("Thanks"),
            Message::assistant("Welcome!"),
        ];
        let split = find_split_point(&history, 60, 2);
        if split < history.len() {
            assert_ne!(
                history[split].role,
                Role::Tool,
                "first kept message must not be an orphaned tool result"
            );
        }
    }
}
