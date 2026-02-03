//! Execution control extension for Context.
//!
//! Provides methods for controlling tool execution flow:
//! - `block(reason)` - Block execution with a reason
//! - `pending(interaction)` - Request user interaction
//! - `is_blocked()` / `is_pending()` - Check execution state
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! async fn before_tool_execute(&self, ctx: &Context<'_>, tool_id: &str, _args: &Value) {
//!     if tool_id == "dangerous_tool" {
//!         ctx.block("This tool is not allowed");
//!     }
//! }
//! ```

use crate::state_types::Interaction;
use carve_state::Context;
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// State path for execution state.
pub const EXECUTION_STATE_PATH: &str = "execution";

/// Execution state stored in session state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct ExecutionState {
    /// Block reason if execution is blocked, None otherwise.
    pub blocked: Option<String>,
    /// Pending interaction if waiting for user, None otherwise.
    pub pending: Option<Interaction>,
}

/// Extension trait for execution control on Context.
pub trait ExecutionContextExt {
    /// Block execution with a reason.
    ///
    /// When blocked, tool execution will be skipped and the reason
    /// will be returned as an error.
    fn block(&self, reason: impl Into<String>);

    /// Request user interaction before continuing.
    ///
    /// When pending, execution will pause until the interaction is resolved.
    fn pending(&self, interaction: Interaction);

    /// Check if execution is blocked.
    fn is_blocked(&self) -> bool;

    /// Check if execution is pending user interaction.
    fn is_pending(&self) -> bool;

    /// Get the block reason if blocked.
    fn block_reason(&self) -> Option<String>;

    /// Get the pending interaction if any.
    fn pending_interaction(&self) -> Option<Interaction>;

    /// Clear the blocked state.
    fn clear_blocked(&self);

    /// Clear the pending state.
    fn clear_pending(&self);
}

impl ExecutionContextExt for Context<'_> {
    fn block(&self, reason: impl Into<String>) {
        // Use Context's built-in execution control
        self.set_blocked(reason);
    }

    fn pending(&self, interaction: Interaction) {
        // Store the interaction as JSON in Context's pending field
        let value = serde_json::to_value(&interaction).unwrap_or(Value::Null);
        self.set_pending(value);
    }

    fn is_blocked(&self) -> bool {
        // Use Context's built-in execution control
        Context::is_blocked(self)
    }

    fn is_pending(&self) -> bool {
        // Use Context's built-in execution control
        Context::is_pending(self)
    }

    fn block_reason(&self) -> Option<String> {
        // Use Context's built-in execution control
        Context::block_reason(self)
    }

    fn pending_interaction(&self) -> Option<Interaction> {
        // Retrieve interaction from Context's pending field
        self.pending_data()
            .and_then(|v| serde_json::from_value(v).ok())
    }

    fn clear_blocked(&self) {
        // Use Context's built-in execution control
        Context::clear_blocked(self);
    }

    fn clear_pending(&self) {
        // Use Context's built-in execution control
        Context::clear_pending(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_execution_state_default() {
        let state = ExecutionState::default();
        assert!(state.blocked.is_none());
        assert!(state.pending.is_none());
    }

    #[test]
    fn test_execution_state_serialization() {
        let state = ExecutionState {
            blocked: Some("test reason".to_string()),
            pending: None,
        };

        let json = serde_json::to_string(&state).unwrap();
        let parsed: ExecutionState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.blocked, Some("test reason".to_string()));
    }

    #[test]
    fn test_block() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(!ctx.is_blocked());
        ctx.block("Test block reason");
        // Block is immediately visible via Context's internal field
        assert!(ctx.is_blocked());
        assert_eq!(ctx.block_reason(), Some("Test block reason".to_string()));
    }

    #[test]
    fn test_pending() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(!ctx.is_pending());
        ctx.pending(Interaction::confirm("int_1", "Are you sure?"));
        // Pending is immediately visible via Context's internal field
        assert!(ctx.is_pending());
        let pending = ctx.pending_interaction().unwrap();
        assert_eq!(pending.id, "int_1");
    }

    #[test]
    fn test_clear_blocked() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        // First block
        ctx.block("existing");
        assert!(ctx.is_blocked());

        // Then clear
        ctx.clear_blocked();
        assert!(!ctx.is_blocked());
    }

    #[test]
    fn test_clear_pending() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        // First set pending
        ctx.pending(Interaction::confirm("int_1", "Test?"));
        assert!(ctx.is_pending());

        // Then clear
        ctx.clear_pending();
        assert!(!ctx.is_pending());
    }

    #[test]
    fn test_block_and_pending_independent() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.block("blocked");
        ctx.pending(Interaction::confirm("int_1", "pending"));

        // Both should be set independently
        assert!(ctx.is_blocked());
        assert!(ctx.is_pending());
        assert_eq!(ctx.block_reason(), Some("blocked".to_string()));

        // Clear one doesn't affect the other
        ctx.clear_blocked();
        assert!(!ctx.is_blocked());
        assert!(ctx.is_pending());
    }

    #[test]
    fn test_read_block_reason() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.block("detailed reason");
        assert_eq!(ctx.block_reason(), Some("detailed reason".to_string()));
    }

    #[test]
    fn test_read_pending_interaction() {
        let doc = json!({});
        let ctx = Context::new(&doc, "call_1", "test");

        let interaction = Interaction::confirm("int_1", "Are you sure?");
        ctx.pending(interaction);

        let pending = ctx.pending_interaction().unwrap();
        assert_eq!(pending.id, "int_1");
        assert_eq!(pending.message, "Are you sure?");
    }
}
