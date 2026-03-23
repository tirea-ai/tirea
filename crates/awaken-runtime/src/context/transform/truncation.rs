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
