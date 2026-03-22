//! Dynamic system prompt injection via durable prompt segments.
//!
//! Prompt segments are persistent context messages stored in the state store.
//! Plugins and tools can upsert, remove, or mark segments as ephemeral
//! (consume-after-emit). The `PromptSegmentsPlugin` reads them at each
//! inference turn and injects them into the system prompt.

use serde::{Deserialize, Serialize};

use crate::plugins::{Plugin, PluginDescriptor, PluginRegistrar};
use crate::state::{MutationBatch, StateKey, StateKeyOptions};
use awaken_contract::StateError;
use awaken_contract::registry_spec::AgentSpec;

/// Plugin ID for the prompt segments system.
pub const PROMPT_SEGMENTS_PLUGIN_ID: &str = "prompt_segments";

// ---------------------------------------------------------------------------
// ContextMessage
// ---------------------------------------------------------------------------

/// Injection point for a prompt segment.
///
/// Controls where in the prompt assembly the segment content is inserted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionPoint {
    /// Inject immediately after the base system prompt.
    AfterSystemPrompt,
    /// Inject in the session-context band before conversation history (default).
    #[default]
    BeforeHistory,
    /// Inject at the end of the assembled prompt, after conversation history.
    AfterHistory,
}

/// A persistent context message injected into inference turns.
///
/// Supports injection point targeting and priority ordering for deterministic
/// prompt assembly across multiple plugins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMessage {
    /// Unique key for upsert semantics.
    pub key: String,
    /// Message content injected into the prompt.
    pub content: String,
    /// If true, the message is removed after being emitted once.
    #[serde(default)]
    pub consume_after_emit: bool,
    /// Where in the prompt to inject this segment.
    #[serde(default)]
    pub injection_point: InjectionPoint,
    /// Priority for ordering within the same injection point (lower = earlier).
    /// Segments with the same priority are ordered by insertion time.
    #[serde(default = "default_priority")]
    pub priority: i32,
    /// Whether this segment updates dynamically each step.
    /// Dynamic segments are always re-evaluated; static segments persist unchanged.
    #[serde(default)]
    pub dynamic: bool,
}

const fn default_priority() -> i32 {
    100
}

impl ContextMessage {
    /// Create a session-scoped context message (injected before history).
    pub fn session(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            content: content.into(),
            consume_after_emit: false,
            injection_point: InjectionPoint::BeforeHistory,
            priority: default_priority(),
            dynamic: false,
        }
    }

    /// Create a context message injected after the system prompt.
    pub fn after_system(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            content: content.into(),
            consume_after_emit: false,
            injection_point: InjectionPoint::AfterSystemPrompt,
            priority: default_priority(),
            dynamic: false,
        }
    }

    /// Create a context message appended after the conversation history.
    pub fn after_history(key: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            content: content.into(),
            consume_after_emit: false,
            injection_point: InjectionPoint::AfterHistory,
            priority: default_priority(),
            dynamic: false,
        }
    }

    /// Set whether this message should be consumed after one emission.
    pub fn with_consume_after_emit(mut self, consume: bool) -> Self {
        self.consume_after_emit = consume;
        self
    }

    /// Set the injection point.
    pub fn with_injection_point(mut self, point: InjectionPoint) -> Self {
        self.injection_point = point;
        self
    }

    /// Set the priority (lower = earlier within the same injection point).
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Mark this segment as dynamic (re-evaluated per step).
    pub fn with_dynamic(mut self, dynamic: bool) -> Self {
        self.dynamic = dynamic;
        self
    }
}

// ---------------------------------------------------------------------------
// PromptSegmentState + Actions
// ---------------------------------------------------------------------------

/// Durable store for persistent context messages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptSegmentState {
    #[serde(default)]
    pub items: Vec<ContextMessage>,
}

impl PromptSegmentState {
    /// Return items sorted by injection point then priority for deterministic ordering.
    pub fn sorted_items(&self) -> Vec<&ContextMessage> {
        let mut sorted: Vec<&ContextMessage> = self.items.iter().collect();
        sorted.sort_by(|a, b| {
            a.injection_point
                .cmp(&b.injection_point)
                .then(a.priority.cmp(&b.priority))
        });
        sorted
    }
}

/// Reducer actions for `PromptSegmentState`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PromptSegmentAction {
    /// Insert or replace a context message by key.
    Upsert { message: ContextMessage },
    /// Remove one context message by key.
    Remove { key: String },
    /// Remove every context message whose key starts with the given prefix.
    RemoveByKeyPrefix { prefix: String },
    /// Remove every context message flagged `consume_after_emit`.
    ConsumeAfterEmit,
    /// Clear all stored context messages.
    Clear,
}

impl PromptSegmentState {
    pub fn reduce(&mut self, action: PromptSegmentAction) {
        match action {
            PromptSegmentAction::Upsert { message } => {
                if let Some(existing) = self.items.iter_mut().find(|item| item.key == message.key) {
                    *existing = message;
                } else {
                    self.items.push(message);
                }
            }
            PromptSegmentAction::Remove { key } => {
                self.items.retain(|item| item.key != key);
            }
            PromptSegmentAction::RemoveByKeyPrefix { prefix } => {
                self.items.retain(|item| !item.key.starts_with(&prefix));
            }
            PromptSegmentAction::ConsumeAfterEmit => {
                self.items.retain(|item| !item.consume_after_emit);
            }
            PromptSegmentAction::Clear => self.items.clear(),
        }
    }
}

// ---------------------------------------------------------------------------
// StateKey
// ---------------------------------------------------------------------------

/// State key for prompt segments.
pub struct PromptSegmentKey;

impl StateKey for PromptSegmentKey {
    const KEY: &'static str = "prompt_segments";
    type Value = PromptSegmentState;
    type Update = PromptSegmentAction;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        value.reduce(update);
    }
}

// ---------------------------------------------------------------------------
// Action constructors
// ---------------------------------------------------------------------------

/// Create a state update that upserts a context message.
pub fn upsert_context_message_action(message: ContextMessage) -> PromptSegmentAction {
    PromptSegmentAction::Upsert { message }
}

/// Create a state update that removes a context message by key.
pub fn remove_context_message_action(key: impl Into<String>) -> PromptSegmentAction {
    PromptSegmentAction::Remove { key: key.into() }
}

/// Create a state update that removes all messages with a given key prefix.
pub fn remove_context_messages_by_prefix_action(prefix: impl Into<String>) -> PromptSegmentAction {
    PromptSegmentAction::RemoveByKeyPrefix {
        prefix: prefix.into(),
    }
}

/// Create a state update that consumes all ephemeral messages.
pub fn consume_after_emit_context_messages_action() -> PromptSegmentAction {
    PromptSegmentAction::ConsumeAfterEmit
}

// ---------------------------------------------------------------------------
// PromptSegmentsPlugin
// ---------------------------------------------------------------------------

/// Plugin that bridges durable prompt-segment state into per-inference context.
#[derive(Debug, Clone, Default)]
pub struct PromptSegmentsPlugin;

impl Plugin for PromptSegmentsPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: PROMPT_SEGMENTS_PLUGIN_ID,
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_key::<PromptSegmentKey>(StateKeyOptions::default())?;
        Ok(())
    }

    fn on_activate(
        &self,
        _agent_spec: &AgentSpec,
        _patch: &mut MutationBatch,
    ) -> Result<(), StateError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateStore;

    fn msg(key: &str, content: &str, consume: bool) -> ContextMessage {
        ContextMessage::session(key, content).with_consume_after_emit(consume)
    }

    #[test]
    fn upsert_replaces_existing_key() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "old", true)],
        };
        state.reduce(PromptSegmentAction::Upsert {
            message: msg("a", "new", false),
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].content, "new");
        assert!(!state.items[0].consume_after_emit);
    }

    #[test]
    fn upsert_adds_new_key() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", false)],
        };
        state.reduce(PromptSegmentAction::Upsert {
            message: msg("b", "two", false),
        });
        assert_eq!(state.items.len(), 2);
    }

    #[test]
    fn remove_by_key() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", false), msg("b", "two", false)],
        };
        state.reduce(PromptSegmentAction::Remove { key: "a".into() });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "b");
    }

    #[test]
    fn remove_by_key_prefix_drops_matching() {
        let mut state = PromptSegmentState {
            items: vec![msg("reminder:a", "one", true), msg("skill:b", "two", false)],
        };
        state.reduce(PromptSegmentAction::RemoveByKeyPrefix {
            prefix: "reminder:".into(),
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "skill:b");
    }

    #[test]
    fn consume_after_emit_only_removes_ephemeral() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", true), msg("b", "two", false)],
        };
        state.reduce(PromptSegmentAction::ConsumeAfterEmit);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "b");
    }

    #[test]
    fn clear_removes_all() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", false), msg("b", "two", false)],
        };
        state.reduce(PromptSegmentAction::Clear);
        assert!(state.items.is_empty());
    }

    #[test]
    fn plugin_registers_key() {
        let store = StateStore::new();
        store.install_plugin(PromptSegmentsPlugin).unwrap();
        let registry = store.registry.lock().unwrap();
        assert!(registry.keys_by_name.contains_key("prompt_segments"));
    }

    #[test]
    fn state_via_store() {
        let store = StateStore::new();
        store.install_plugin(PromptSegmentsPlugin).unwrap();

        // Upsert
        let mut patch = store.begin_mutation();
        patch.update::<PromptSegmentKey>(upsert_context_message_action(msg(
            "greeting", "Hello!", false,
        )));
        store.commit(patch).unwrap();

        let state = store.read::<PromptSegmentKey>().unwrap();
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "greeting");

        // Upsert again (replace)
        let mut patch = store.begin_mutation();
        patch.update::<PromptSegmentKey>(upsert_context_message_action(msg(
            "greeting", "Hi!", true,
        )));
        store.commit(patch).unwrap();

        let state = store.read::<PromptSegmentKey>().unwrap();
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].content, "Hi!");
        assert!(state.items[0].consume_after_emit);

        // Consume after emit
        let mut patch = store.begin_mutation();
        patch.update::<PromptSegmentKey>(consume_after_emit_context_messages_action());
        store.commit(patch).unwrap();

        let state = store.read::<PromptSegmentKey>().unwrap();
        assert!(state.items.is_empty());
    }

    #[test]
    fn action_constructors() {
        let a = upsert_context_message_action(msg("k", "v", false));
        assert!(matches!(a, PromptSegmentAction::Upsert { .. }));

        let a = remove_context_message_action("k");
        assert!(matches!(a, PromptSegmentAction::Remove { key } if key == "k"));

        let a = remove_context_messages_by_prefix_action("p:");
        assert!(matches!(a, PromptSegmentAction::RemoveByKeyPrefix { prefix } if prefix == "p:"));

        let a = consume_after_emit_context_messages_action();
        assert!(matches!(a, PromptSegmentAction::ConsumeAfterEmit));
    }

    #[test]
    fn context_message_builder() {
        let m = ContextMessage::session("test", "hello").with_consume_after_emit(true);
        assert_eq!(m.key, "test");
        assert_eq!(m.content, "hello");
        assert!(m.consume_after_emit);
    }

    // -----------------------------------------------------------------------
    // Migrated from uncarve: additional prompt segment tests
    // -----------------------------------------------------------------------

    #[test]
    fn remove_nonexistent_key_is_noop() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", false)],
        };
        state.reduce(PromptSegmentAction::Remove {
            key: "nonexistent".into(),
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "a");
    }

    #[test]
    fn remove_by_prefix_no_match() {
        let mut state = PromptSegmentState {
            items: vec![msg("skill:a", "one", false), msg("skill:b", "two", false)],
        };
        state.reduce(PromptSegmentAction::RemoveByKeyPrefix {
            prefix: "reminder:".into(),
        });
        assert_eq!(state.items.len(), 2);
    }

    #[test]
    fn remove_by_prefix_removes_all_matching() {
        let mut state = PromptSegmentState {
            items: vec![
                msg("mcp:tool1", "t1", false),
                msg("mcp:tool2", "t2", false),
                msg("skill:a", "s1", false),
            ],
        };
        state.reduce(PromptSegmentAction::RemoveByKeyPrefix {
            prefix: "mcp:".into(),
        });
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "skill:a");
    }

    #[test]
    fn consume_after_emit_with_no_ephemeral() {
        let mut state = PromptSegmentState {
            items: vec![msg("a", "one", false), msg("b", "two", false)],
        };
        state.reduce(PromptSegmentAction::ConsumeAfterEmit);
        assert_eq!(state.items.len(), 2);
    }

    #[test]
    fn consume_after_emit_removes_all_ephemeral() {
        let mut state = PromptSegmentState {
            items: vec![
                msg("a", "one", true),
                msg("b", "two", true),
                msg("c", "three", false),
            ],
        };
        state.reduce(PromptSegmentAction::ConsumeAfterEmit);
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].key, "c");
    }

    #[test]
    fn clear_on_empty_state() {
        let mut state = PromptSegmentState::default();
        state.reduce(PromptSegmentAction::Clear);
        assert!(state.items.is_empty());
    }

    #[test]
    fn upsert_preserves_ordering_of_other_items() {
        let mut state = PromptSegmentState {
            items: vec![
                msg("a", "one", false),
                msg("b", "two", false),
                msg("c", "three", false),
            ],
        };
        state.reduce(PromptSegmentAction::Upsert {
            message: msg("b", "updated", false),
        });
        assert_eq!(state.items.len(), 3);
        assert_eq!(state.items[0].key, "a");
        assert_eq!(state.items[1].key, "b");
        assert_eq!(state.items[1].content, "updated");
        assert_eq!(state.items[2].key, "c");
    }

    #[test]
    fn multiple_operations_in_sequence() {
        let mut state = PromptSegmentState::default();

        // Add three items
        for i in 0..3 {
            state.reduce(PromptSegmentAction::Upsert {
                message: msg(&format!("k{i}"), &format!("v{i}"), i % 2 == 0),
            });
        }
        assert_eq!(state.items.len(), 3);

        // Remove one
        state.reduce(PromptSegmentAction::Remove { key: "k1".into() });
        assert_eq!(state.items.len(), 2);

        // Consume ephemeral (k0 and k2 are ephemeral)
        state.reduce(PromptSegmentAction::ConsumeAfterEmit);
        assert_eq!(state.items.len(), 0);
    }

    #[test]
    fn state_serde_roundtrip() {
        let state = PromptSegmentState {
            items: vec![msg("a", "hello", false), msg("b", "world", true)],
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: PromptSegmentState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.items.len(), 2);
        assert_eq!(parsed.items[0].key, "a");
        assert!(parsed.items[1].consume_after_emit);
    }

    #[test]
    fn action_serde_roundtrip() {
        let actions = vec![
            PromptSegmentAction::Upsert {
                message: msg("k", "v", true),
            },
            PromptSegmentAction::Remove { key: "k".into() },
            PromptSegmentAction::RemoveByKeyPrefix {
                prefix: "p:".into(),
            },
            PromptSegmentAction::ConsumeAfterEmit,
            PromptSegmentAction::Clear,
        ];
        for action in actions {
            let json = serde_json::to_string(&action).unwrap();
            let parsed: PromptSegmentAction = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, action);
        }
    }

    #[test]
    fn context_message_default_not_ephemeral() {
        let m = ContextMessage::session("test", "content");
        assert!(!m.consume_after_emit);
    }

    #[test]
    fn context_message_equality() {
        let m1 = ContextMessage::session("test", "content");
        let m2 = ContextMessage::session("test", "content");
        assert_eq!(m1, m2);

        let m3 = ContextMessage::session("test", "different");
        assert_ne!(m1, m3);
    }

    // -----------------------------------------------------------------------
    // Injection point and priority tests
    // -----------------------------------------------------------------------

    #[test]
    fn session_defaults_to_before_history() {
        let m = ContextMessage::session("test", "content");
        assert_eq!(m.injection_point, InjectionPoint::BeforeHistory);
        assert_eq!(m.priority, 100);
        assert!(!m.dynamic);
    }

    #[test]
    fn after_system_injection_point() {
        let m = ContextMessage::after_system("sys", "system context");
        assert_eq!(m.injection_point, InjectionPoint::AfterSystemPrompt);
        assert_eq!(m.key, "sys");
        assert_eq!(m.content, "system context");
    }

    #[test]
    fn after_history_injection_point() {
        let m = ContextMessage::after_history("tail", "tail instructions");
        assert_eq!(m.injection_point, InjectionPoint::AfterHistory);
        assert_eq!(m.key, "tail");
    }

    #[test]
    fn with_injection_point_builder() {
        let m = ContextMessage::session("k", "v")
            .with_injection_point(InjectionPoint::AfterSystemPrompt);
        assert_eq!(m.injection_point, InjectionPoint::AfterSystemPrompt);
    }

    #[test]
    fn with_priority_builder() {
        let m = ContextMessage::session("k", "v").with_priority(10);
        assert_eq!(m.priority, 10);
    }

    #[test]
    fn with_dynamic_builder() {
        let m = ContextMessage::session("k", "v").with_dynamic(true);
        assert!(m.dynamic);
    }

    #[test]
    fn sorted_items_by_injection_point_then_priority() {
        let state = PromptSegmentState {
            items: vec![
                ContextMessage::after_history("tail", "last").with_priority(50),
                ContextMessage::session("mid", "middle").with_priority(10),
                ContextMessage::after_system("sys", "first").with_priority(5),
                ContextMessage::session("mid2", "middle2").with_priority(5),
            ],
        };

        let sorted = state.sorted_items();
        assert_eq!(sorted.len(), 4);
        // AfterSystemPrompt comes first
        assert_eq!(sorted[0].key, "sys");
        // BeforeHistory items next, sorted by priority
        assert_eq!(sorted[1].key, "mid2");
        assert_eq!(sorted[2].key, "mid");
        // AfterHistory last
        assert_eq!(sorted[3].key, "tail");
    }

    #[test]
    fn sorted_items_same_point_same_priority_preserves_insertion_order() {
        let state = PromptSegmentState {
            items: vec![
                ContextMessage::session("a", "first").with_priority(100),
                ContextMessage::session("b", "second").with_priority(100),
                ContextMessage::session("c", "third").with_priority(100),
            ],
        };

        let sorted = state.sorted_items();
        assert_eq!(sorted[0].key, "a");
        assert_eq!(sorted[1].key, "b");
        assert_eq!(sorted[2].key, "c");
    }

    #[test]
    fn injection_point_serde_roundtrip() {
        let m = ContextMessage::after_system("k", "v")
            .with_priority(42)
            .with_dynamic(true)
            .with_consume_after_emit(true);
        let json = serde_json::to_string(&m).unwrap();
        let parsed: ContextMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.injection_point, InjectionPoint::AfterSystemPrompt);
        assert_eq!(parsed.priority, 42);
        assert!(parsed.dynamic);
        assert!(parsed.consume_after_emit);
    }

    #[test]
    fn injection_point_ordering() {
        assert!(InjectionPoint::AfterSystemPrompt < InjectionPoint::BeforeHistory);
        assert!(InjectionPoint::BeforeHistory < InjectionPoint::AfterHistory);
    }

    #[test]
    fn sorted_items_empty() {
        let state = PromptSegmentState::default();
        assert!(state.sorted_items().is_empty());
    }

    #[test]
    fn full_builder_chain() {
        let m = ContextMessage::session("reminder", "Remember this")
            .with_injection_point(InjectionPoint::AfterHistory)
            .with_priority(1)
            .with_dynamic(true)
            .with_consume_after_emit(true);

        assert_eq!(m.key, "reminder");
        assert_eq!(m.content, "Remember this");
        assert_eq!(m.injection_point, InjectionPoint::AfterHistory);
        assert_eq!(m.priority, 1);
        assert!(m.dynamic);
        assert!(m.consume_after_emit);
    }
}
