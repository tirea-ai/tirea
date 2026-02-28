//! Reminder policy extension.
//!
//! External callers only depend on [`ReminderAction`]. Internal reminder
//! state/reducer details are handled by [`ReminderPlugin`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tirea_contract::runtime::plugin::agent::{AgentBehavior, ReadOnlyContext};
use tirea_contract::runtime::plugin::phase::action::Action;
use tirea_contract::runtime::plugin::phase::core::actions::{AddSessionContext, EmitStatePatch};
use tirea_contract::runtime::plugin::phase::state_spec::{AnyStateAction, StateSpec};
use tirea_state::State;

mod system_reminder;
pub use system_reminder::SystemReminder;

/// Reminder state stored in session state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
#[tirea(path = "reminders")]
struct ReminderState {
    /// List of reminder texts.
    #[serde(default)]
    pub items: Vec<String>,
}

/// Action type for `ReminderState` reducer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReminderAction {
    /// Add one reminder item.
    Add { text: String },
    /// Remove one reminder item.
    Remove { text: String },
    /// Clear all reminder items.
    Clear,
}

/// Stable plugin id for reminder actions.
pub const REMINDER_PLUGIN_ID: &str = "reminder";

impl StateSpec for ReminderState {
    type Action = ReminderAction;

    fn reduce(&mut self, action: ReminderAction) {
        match action {
            ReminderAction::Add { text } => self.items.push(text),
            ReminderAction::Remove { text } => self.items.retain(|item| item != &text),
            ReminderAction::Clear => self.items.clear(),
        }
    }
}

/// Plugin that manages system reminders.
///
/// This plugin:
/// - Initializes the reminder state
/// - Can clear reminders after they're used in `before_llm_request`
///
/// Note: The actual injection of reminders into LLM context is done by
/// the agent loop, which reads `ctx.reminders()` and formats them
/// appropriately.
pub struct ReminderPlugin {
    /// Whether to clear reminders after each LLM request.
    pub clear_after_llm_request: bool,
}

impl Default for ReminderPlugin {
    fn default() -> Self {
        Self {
            clear_after_llm_request: true,
        }
    }
}

impl ReminderPlugin {
    /// Create a new reminder plugin.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set whether to clear reminders after each LLM request.
    pub fn with_clear_after_llm_request(mut self, clear: bool) -> Self {
        self.clear_after_llm_request = clear;
        self
    }
}

#[async_trait]
impl AgentBehavior for ReminderPlugin {
    fn id(&self) -> &str {
        REMINDER_PLUGIN_ID
    }

    async fn before_inference(&self, ctx: &ReadOnlyContext<'_>) -> Vec<Box<dyn Action>> {
        let reminders = ctx
            .snapshot_of::<ReminderState>()
            .ok()
            .map(|s| s.items)
            .unwrap_or_default();
        if reminders.is_empty() {
            return vec![];
        }

        let mut actions: Vec<Box<dyn Action>> = Vec::new();
        for text in &reminders {
            actions.push(Box::new(AddSessionContext(format!("Reminder: {}", text))));
        }

        if self.clear_after_llm_request {
            actions.push(Box::new(EmitStatePatch(
                AnyStateAction::new::<ReminderState>(ReminderAction::Clear),
            )));
        }

        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tirea_contract::runtime::plugin::phase::Phase;
    use tirea_contract::RunConfig;
    use tirea_state::DocCell;

    fn count_session_contexts(actions: &[Box<dyn Action>]) -> usize {
        actions
            .iter()
            .filter(|a| a.label() == "add_session_context")
            .count()
    }

    fn has_emit_state_patch(actions: &[Box<dyn Action>]) -> bool {
        actions.iter().any(|a| a.label() == "emit_state_patch")
    }

    #[test]
    fn test_reminder_state_default() {
        let state = ReminderState::default();
        assert!(state.items.is_empty());
    }

    #[test]
    fn test_reminder_state_serialization() {
        let mut state = ReminderState::default();
        state.items.push("Reminder 1".to_string());
        state.items.push("Reminder 2".to_string());

        let json = serde_json::to_string(&state).unwrap();
        let parsed: ReminderState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.items.len(), 2);
    }

    #[test]
    fn test_reminder_plugin_id() {
        let plugin = ReminderPlugin::new();
        assert_eq!(AgentBehavior::id(&plugin), REMINDER_PLUGIN_ID);
    }

    #[test]
    fn test_reminder_plugin_builder() {
        let plugin = ReminderPlugin::new().with_clear_after_llm_request(false);
        assert!(!plugin.clear_after_llm_request);
    }

    #[tokio::test]
    async fn test_reminder_plugin_before_inference() {
        let plugin = ReminderPlugin::new();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "reminders": { "items": ["Test reminder"] } }));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&plugin, &ctx).await;
        assert!(count_session_contexts(&actions) > 0);
    }

    #[tokio::test]
    async fn test_reminder_plugin_generates_clear_action() {
        let plugin = ReminderPlugin::new(); // clear_after_llm_request = true
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "reminders": { "items": ["Reminder A", "Reminder B"] } }));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&plugin, &ctx).await;
        assert_eq!(count_session_contexts(&actions), 2);
        assert!(
            has_emit_state_patch(&actions),
            "should include EmitStatePatch for clear"
        );
    }

    #[tokio::test]
    async fn test_reminder_plugin_no_clear_when_disabled() {
        let plugin = ReminderPlugin::new().with_clear_after_llm_request(false);
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "reminders": { "items": ["Reminder"] } }));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&plugin, &ctx).await;
        assert!(count_session_contexts(&actions) > 0);
        assert!(
            !has_emit_state_patch(&actions),
            "should not include EmitStatePatch when clear disabled"
        );
    }

    #[tokio::test]
    async fn test_reminder_plugin_empty_reminders() {
        let plugin = ReminderPlugin::new();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({ "reminders": { "items": [] } }));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&plugin, &ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn test_reminder_plugin_no_state() {
        let plugin = ReminderPlugin::new();
        let config = RunConfig::new();
        let doc = DocCell::new(json!({}));
        let ctx = ReadOnlyContext::new(Phase::BeforeInference, "t1", &[], &config, &doc);
        let actions = AgentBehavior::before_inference(&plugin, &ctx).await;
        assert!(actions.is_empty());
    }
}
