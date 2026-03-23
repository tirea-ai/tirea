//! Artifact and tool-result compaction: shrink oversized content blocks to previews.

use awaken_contract::contract::content::ContentBlock;
use awaken_contract::contract::message::{Message, Role};

/// Token threshold above which a tool result is compacted to a preview.
pub const ARTIFACT_COMPACT_THRESHOLD_TOKENS: usize = 2048;

/// Maximum characters retained in a compacted artifact preview.
pub const ARTIFACT_PREVIEW_MAX_CHARS: usize = 1600;

/// Maximum lines retained in a compacted artifact preview.
pub const ARTIFACT_PREVIEW_MAX_LINES: usize = 24;

/// Compact a single artifact string if it exceeds the token threshold.
///
/// Returns the original content unchanged when estimated tokens are within budget.
/// Otherwise truncates to [`ARTIFACT_PREVIEW_MAX_CHARS`] / [`ARTIFACT_PREVIEW_MAX_LINES`]
/// (whichever is shorter) and appends a compaction indicator.
pub fn compact_artifact(content: &str) -> String {
    let estimated_tokens = content.len() / 4;
    if estimated_tokens < ARTIFACT_COMPACT_THRESHOLD_TOKENS {
        return content.to_string();
    }

    // Truncate by line limit first, then by char limit
    let mut char_count = 0usize;
    let mut line_count = 0usize;
    let mut end_byte = 0usize;

    for (idx, ch) in content.char_indices() {
        if char_count >= ARTIFACT_PREVIEW_MAX_CHARS || line_count >= ARTIFACT_PREVIEW_MAX_LINES {
            break;
        }
        if ch == '\n' {
            line_count += 1;
        }
        char_count += 1;
        end_byte = idx + ch.len_utf8();
    }

    let preview = &content[..end_byte];
    format!(
        "{preview}\n\n[Content compacted: original ~{estimated_tokens} tokens, showing first {char_count} chars]"
    )
}

/// Compact tool result messages that exceed the artifact token threshold.
///
/// Iterates over all `Role::Tool` messages and replaces oversized text content
/// blocks with a truncated preview plus compaction indicator.
pub fn compact_tool_results(messages: &mut [Message]) {
    for msg in messages.iter_mut() {
        if msg.role != Role::Tool {
            continue;
        }
        let mut modified = false;
        let new_content: Vec<ContentBlock> = msg
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => {
                    let compacted = compact_artifact(text);
                    if compacted.len() != text.len() {
                        modified = true;
                    }
                    ContentBlock::Text { text: compacted }
                }
                other => other.clone(),
            })
            .collect();
        if modified {
            msg.content = new_content;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::message::ToolCall;
    use serde_json::json;

    #[test]
    fn small_tool_result_not_compacted() {
        let small_content = "x".repeat(100);
        let mut messages = vec![
            Message::user("go"),
            Message::assistant_with_tool_calls("", vec![ToolCall::new("c1", "search", json!({}))]),
            Message::tool("c1", &small_content),
        ];
        compact_tool_results(&mut messages);
        assert_eq!(messages[2].text(), small_content);
    }

    #[test]
    fn large_tool_result_compacted_to_preview() {
        // 2048 tokens * 4 chars/token = 8192 chars needed to exceed threshold
        let large_content = "a".repeat(10_000);
        let mut messages = vec![
            Message::user("go"),
            Message::assistant_with_tool_calls(
                "",
                vec![ToolCall::new("c1", "list_files", json!({}))],
            ),
            Message::tool("c1", &large_content),
        ];
        compact_tool_results(&mut messages);

        let result = messages[2].text();
        assert!(
            result.len() < large_content.len(),
            "content should be shorter after compaction"
        );
        assert!(
            result.contains("[Content compacted:"),
            "should contain compaction indicator"
        );
        assert!(result.contains("tokens"), "indicator should mention tokens");
        assert!(result.contains("chars"), "indicator should mention chars");
    }

    #[test]
    fn compact_preserves_non_tool_messages() {
        let large_text = "b".repeat(10_000);
        let mut messages = vec![
            Message::system(&large_text),
            Message::user(&large_text),
            Message::assistant(&large_text),
        ];
        let texts_before: Vec<String> = messages.iter().map(|m| m.text()).collect();
        compact_tool_results(&mut messages);
        let texts_after: Vec<String> = messages.iter().map(|m| m.text()).collect();
        assert_eq!(
            texts_before, texts_after,
            "non-tool messages should be unchanged"
        );
    }

    #[test]
    fn compact_artifact_below_threshold_unchanged() {
        let content = "short content";
        let result = compact_artifact(content);
        assert_eq!(result, content);
    }

    #[test]
    fn compact_artifact_above_threshold_truncates() {
        let content = "x".repeat(10_000);
        let result = compact_artifact(&content);
        assert!(result.len() < content.len());
        assert!(result.contains("[Content compacted:"));
    }

    #[test]
    fn compact_artifact_respects_line_limit() {
        // Create content with many lines that exceeds threshold
        let content: String = (0..100)
            .map(|i| format!("line {}: {}", i, "x".repeat(200)))
            .collect::<Vec<_>>()
            .join("\n");
        let result = compact_artifact(&content);
        // Count lines in the preview part (before the compaction indicator)
        let lines_before_indicator = result
            .split("[Content compacted:")
            .next()
            .unwrap_or("")
            .lines()
            .count();
        assert!(
            lines_before_indicator <= ARTIFACT_PREVIEW_MAX_LINES + 1,
            "should respect line limit, got {} lines",
            lines_before_indicator
        );
    }

    #[test]
    fn compact_tool_results_multiple_tool_messages() {
        let small = "x".repeat(100);
        let large = "y".repeat(10_000);
        let mut messages = vec![
            Message::user("go"),
            Message::assistant_with_tool_calls(
                "",
                vec![
                    ToolCall::new("c1", "small", json!({})),
                    ToolCall::new("c2", "large", json!({})),
                ],
            ),
            Message::tool("c1", &small),
            Message::tool("c2", &large),
        ];
        compact_tool_results(&mut messages);

        // Small tool result unchanged
        assert_eq!(messages[2].text(), small);
        // Large tool result compacted
        assert!(messages[3].text().len() < large.len());
        assert!(messages[3].text().contains("[Content compacted:"));
    }

    #[test]
    fn compact_artifact_boundary_just_under_threshold() {
        // Exactly at threshold: 2048 * 4 = 8192 chars
        let content = "a".repeat(8191);
        let result = compact_artifact(&content);
        // 8191 / 4 = 2047, which is < 2048 threshold
        assert_eq!(result, content, "just under threshold should not compact");
    }

    #[test]
    fn compact_artifact_boundary_at_threshold() {
        // At threshold: 2048 * 4 = 8192 chars
        let content = "a".repeat(8192);
        let result = compact_artifact(&content);
        // 8192 / 4 = 2048, which is NOT < 2048
        assert!(result.len() < content.len(), "at threshold should compact");
    }
}
