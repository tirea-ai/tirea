//! Reminder management extension for Context.
//!
//! Provides methods for managing system reminders that can be injected
//! into LLM context:
//! - `add_reminder(text)` - Add a reminder
//! - `reminders()` - Get all reminders
//!
//! The `ReminderPlugin` can inject reminders into the LLM request.
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! async fn after_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, result: &ToolResult) {
//!     if tool_id == "file_read" {
//!         ctx.add_reminder("Remember to close the file when done");
//!     }
//! }
//! ```

use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// State path for reminders.
pub const REMINDER_STATE_PATH: &str = "reminders";

/// Reminder state stored in session state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct ReminderState {
    /// List of reminder texts.
    #[serde(default)]
    pub items: Vec<String>,
}

/// Extension trait for reminder management on Context.
pub trait ReminderContextExt {
    /// Add a reminder.
    fn add_reminder(&self, text: impl Into<String>);

    /// Get all reminders.
    fn reminders(&self) -> Vec<String>;

    /// Get the number of reminders.
    fn reminder_count(&self) -> usize;

    /// Clear all reminders.
    fn clear_reminders(&self);

    /// Remove a specific reminder by text.
    fn remove_reminder(&self, text: &str);
}

impl ReminderContextExt for Context<'_> {
    fn add_reminder(&self, text: impl Into<String>) {
        let state = self.state::<ReminderState>(REMINDER_STATE_PATH);
        state.items_push(text.into());
    }

    fn reminders(&self) -> Vec<String> {
        let state = self.state::<ReminderState>(REMINDER_STATE_PATH);
        state.items().ok().unwrap_or_default()
    }

    fn reminder_count(&self) -> usize {
        self.reminders().len()
    }

    fn clear_reminders(&self) {
        let state = self.state::<ReminderState>(REMINDER_STATE_PATH);
        state.set_items(Vec::new());
    }

    fn remove_reminder(&self, text: &str) {
        let reminders = self.reminders();
        let filtered: Vec<String> = reminders.into_iter().filter(|r| r != text).collect();
        let state = self.state::<ReminderState>(REMINDER_STATE_PATH);
        state.set_items(filtered);
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
impl AgentPlugin for ReminderPlugin {
    fn id(&self) -> &str {
        "reminder"
    }

    fn initial_state(&self) -> Option<(&'static str, Value)> {
        Some((REMINDER_STATE_PATH, json!({ "items": [] })))
    }

    async fn before_llm_request(&self, _ctx: &Context<'_>) {
        // Reminders are read by the agent loop for injection.
        // We can optionally clear them after the request is made.
        // The actual clearing happens in a post-request hook if needed.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn test_add_reminder() {
        let doc = json!({
            "reminders": { "items": [] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.add_reminder("Test reminder");
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_reminders_empty() {
        let doc = json!({
            "reminders": { "items": [] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.reminders().is_empty());
        assert_eq!(ctx.reminder_count(), 0);
    }

    #[test]
    fn test_reminders_with_existing() {
        let doc = json!({
            "reminders": { "items": ["Reminder 1", "Reminder 2"] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        let reminders = ctx.reminders();
        assert_eq!(reminders.len(), 2);
        assert_eq!(ctx.reminder_count(), 2);
    }

    #[test]
    fn test_clear_reminders() {
        let doc = json!({
            "reminders": { "items": ["Reminder 1", "Reminder 2"] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert_eq!(ctx.reminder_count(), 2);
        ctx.clear_reminders();
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_remove_reminder() {
        let doc = json!({
            "reminders": { "items": ["Keep", "Remove", "Keep2"] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.remove_reminder("Remove");
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_reminder_plugin_id() {
        let plugin = ReminderPlugin::new();
        assert_eq!(plugin.id(), "reminder");
    }

    #[test]
    fn test_reminder_plugin_initial_state() {
        let plugin = ReminderPlugin::new();
        let state = plugin.initial_state();

        assert!(state.is_some());
        let (path, value) = state.unwrap();
        assert_eq!(path, "reminders");
        assert_eq!(value["items"], json!([]));
    }

    #[test]
    fn test_reminder_plugin_builder() {
        let plugin = ReminderPlugin::new().with_clear_after_llm_request(false);
        assert!(!plugin.clear_after_llm_request);
    }

    #[tokio::test]
    async fn test_reminder_plugin_before_llm_request() {
        let plugin = ReminderPlugin::new();
        let doc = json!({
            "reminders": { "items": ["Test reminder"] }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        // Call before_llm_request - should not panic
        plugin.before_llm_request(&ctx).await;

        // The method is a no-op but should be callable
        assert!(!ctx.has_changes());
    }
}
