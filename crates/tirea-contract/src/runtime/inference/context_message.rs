use serde::{Deserialize, Serialize};

/// Injection target for a [`ContextMessage`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextMessageTarget {
    /// Inject as a system message (good for prompt caching of static content).
    #[default]
    System,
    /// Inject as session context before conversation history.
    Session,
}

/// A structured, managed context entry with throttle metadata.
///
/// Plugins emit `BeforeInferenceAction::AddContextMessage(ContextMessage)` to inject
/// context with lifecycle control. Each context
/// entry carries metadata that allows the loop runner to:
///
/// - **Deduplicate** by `key` (same key replaces previous content)
/// - **Throttle** by `cooldown_turns` (skip injection if recently injected
///   and content hasn't changed)
/// - **Target** by `target` (system message vs session context)
///
/// The loop runner maintains a per-run tracker. An entry is injected when:
/// 1. It has never been injected before, OR
/// 2. Its `cooldown_turns` have elapsed since last injection, OR
/// 3. Its content has changed (regardless of cooldown)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMessage {
    /// Unique identifier for deduplication and throttle tracking.
    pub key: String,
    /// The text content to inject.
    pub content: String,
    /// Number of turns to skip after injection. `0` means inject every turn.
    ///
    /// For example, `cooldown_turns: 5` means: inject on turn 0, skip turns
    /// 1–5, inject again on turn 6 (unless content changed earlier).
    #[serde(default)]
    pub cooldown_turns: u32,
    /// Where to inject the content in the message sequence.
    #[serde(default)]
    pub target: ContextMessageTarget,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let entry = ContextMessage {
            key: "skills".into(),
            content: "<skills>...</skills>".into(),
            cooldown_turns: 10,
            target: ContextMessageTarget::System,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let restored: ContextMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.key, "skills");
        assert_eq!(restored.cooldown_turns, 10);
        assert_eq!(restored.target, ContextMessageTarget::System);
    }

    #[test]
    fn defaults_are_system_and_zero_cooldown() {
        let json = r#"{"key":"test","content":"hello"}"#;
        let entry: ContextMessage = serde_json::from_str(json).unwrap();
        assert_eq!(entry.cooldown_turns, 0);
        assert_eq!(entry.target, ContextMessageTarget::System);
    }

    #[test]
    fn session_target_serde() {
        let entry = ContextMessage {
            key: "git_status".into(),
            content: "branch: main".into(),
            cooldown_turns: 3,
            target: ContextMessageTarget::Session,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"session\""));
        let restored: ContextMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.target, ContextMessageTarget::Session);
    }
}
