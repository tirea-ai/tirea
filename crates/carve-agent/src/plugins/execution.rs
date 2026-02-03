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
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.set_blocked(Some(reason.into()));
    }

    fn pending(&self, interaction: Interaction) {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.set_pending(Some(interaction));
    }

    fn is_blocked(&self) -> bool {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.blocked().ok().flatten().is_some()
    }

    fn is_pending(&self) -> bool {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.pending().ok().flatten().is_some()
    }

    fn block_reason(&self) -> Option<String> {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.blocked().ok().flatten()
    }

    fn pending_interaction(&self) -> Option<Interaction> {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.pending().ok().flatten()
    }

    fn clear_blocked(&self) {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.set_blocked(None);
    }

    fn clear_pending(&self) {
        let state = self.state::<ExecutionState>(EXECUTION_STATE_PATH);
        state.set_pending(None);
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
        let doc = json!({ "execution": { "blocked": null, "pending": null } });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(!ctx.is_blocked());
        ctx.block("Test block reason");
        // Note: We can't verify is_blocked() returns true because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_pending() {
        let doc = json!({ "execution": { "blocked": null, "pending": null } });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(!ctx.is_pending());
        ctx.pending(Interaction::confirm("int_1", "Are you sure?"));
        // Note: We can't verify is_pending() returns true because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_clear_blocked() {
        let doc = json!({ "execution": { "blocked": "existing", "pending": null } });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.is_blocked());
        ctx.clear_blocked();
        // Note: We can't verify is_blocked() returns false because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_clear_pending() {
        let interaction = Interaction::confirm("int_1", "Test?");
        let doc = json!({
            "execution": {
                "blocked": null,
                "pending": serde_json::to_value(&interaction).unwrap()
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.is_pending());
        ctx.clear_pending();
        // Note: We can't verify is_pending() returns false because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_block_and_pending_independent() {
        let doc = json!({ "execution": { "blocked": null, "pending": null } });
        let ctx = Context::new(&doc, "call_1", "test");

        ctx.block("blocked");
        ctx.pending(Interaction::confirm("int_1", "pending"));

        // Note: We can't verify is_blocked()/is_pending() return true because
        // Context collects ops that need to be applied to see the result.
        assert!(ctx.has_changes());
    }

    #[test]
    fn test_read_existing_blocked() {
        let doc = json!({ "execution": { "blocked": "existing reason", "pending": null } });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.is_blocked());
        assert_eq!(ctx.block_reason(), Some("existing reason".to_string()));
    }

    #[test]
    fn test_read_existing_pending() {
        let interaction = Interaction::confirm("int_1", "Are you sure?");
        let doc = json!({
            "execution": {
                "blocked": null,
                "pending": serde_json::to_value(&interaction).unwrap()
            }
        });
        let ctx = Context::new(&doc, "call_1", "test");

        assert!(ctx.is_pending());
        let pending = ctx.pending_interaction().unwrap();
        assert_eq!(pending.id, "int_1");
    }
}
