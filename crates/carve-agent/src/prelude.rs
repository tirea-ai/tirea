//! Prelude for convenient imports.
//!
//! This module re-exports the most commonly used types and traits for
//! tool and plugin development. Using the prelude reduces boilerplate
//! and ensures all extension traits are available.
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! struct MyTool;
//!
//! #[async_trait]
//! impl Tool for MyTool {
//!     fn descriptor(&self) -> ToolDescriptor {
//!         ToolDescriptor::new("my_tool", "My Tool", "Does something useful")
//!     }
//!
//!     async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
//!         // Extension traits are auto-imported
//!         ctx.allow_tool("follow_up");           // PermissionContextExt
//!         ctx.add_reminder("Remember to check"); // ReminderContextExt
//!
//!         Ok(ToolResult::success("my_tool", json!({"status": "done"})))
//!     }
//! }
//! ```

// Re-export async_trait for convenience
pub use async_trait::async_trait;

// Re-export serde_json for tool implementations
pub use serde_json::{json, Value};

// Core types from carve-state
pub use carve_state::Context;

// Tool trait and types
pub use crate::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};

// Message types
pub use crate::types::{Message, Role, ToolCall};

// Plugin trait
pub use crate::plugin::AgentPlugin;

// Phase types for plugins
pub use crate::phase::{Phase, StepContext, StepOutcome, ToolContext};

// State types (for plugin developers)
pub use crate::state_types::{
    AgentRunState, AgentRunStatus, AgentState, Interaction, InteractionResponse,
    ToolPermissionBehavior, AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX,
    AGENT_STATE_PATH,
};

// Extension traits - these add methods to Context
pub use crate::plugins::{PermissionContextExt, ReminderContextExt};

// State types (for advanced usage)
pub use crate::plugins::{PermissionState, ReminderState};

// Built-in plugins
pub use crate::plugins::{PermissionPlugin, ReminderPlugin};
