//! SystemReminder trait for generating reminder messages.
//!
//! System reminders generate messages based on state to remind
//! the agent about important information.

use crate::AgentState;
use async_trait::async_trait;

/// System reminder for generating reminder messages.
///
/// # Example
///
/// ```ignore
/// use carve_agent::contracts::extension::traits::reminder::SystemReminder;
/// use carve_agent::prelude::AgentState;
/// use carve_state_derive::State;
///
/// #[derive(State)]
/// struct TodoState {
///     pub pending_count: i64,
/// }
///
/// struct TodoReminder;
///
/// #[async_trait]
/// impl SystemReminder for TodoReminder {
///     fn id(&self) -> &str {
///         "todo_reminder"
///     }
///
///     async fn remind(&self, ctx: &AgentState) -> Option<String> {
///         let state = ctx.state::<TodoState>("components.todos");
///
///         let pending = state.pending_count().unwrap_or(0);
///         if pending > 0 {
///             Some(format!("You have {} pending todos.", pending))
///         } else {
///             None
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait SystemReminder: Send + Sync {
    /// Unique identifier for this reminder.
    fn id(&self) -> &str;

    /// Generate a reminder message.
    ///
    /// # Arguments
    ///
    /// - `ctx`: Context for state access (reference - framework extracts patch after execution)
    ///
    /// # Returns
    ///
    /// Optional reminder message. None means no reminder.
    async fn remind(&self, ctx: &AgentState) -> Option<String>;
}
