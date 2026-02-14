//! Agent plugins and Context extensions.
//!
//! This module provides:
//! - Extension traits for Context that add domain-specific functionality
//! - State types for each extension (using carve-state-derive)
//! - Built-in plugins that implement common behaviors
//!
//! # Extension Traits
//!
//! Each extension trait adds methods to `Context`:
//!
//! - [`PermissionContextExt`] - Manage tool permissions (allow/deny/ask)
//! - [`ReminderContextExt`] - Add reminders for LLM context
//!
//! # Built-in Plugins
//!
//! - [`PermissionPlugin`] - Enforces tool permissions before execution
//! - [`ReminderPlugin`] - Manages reminder lifecycle
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::prelude::*;
//!
//! // All extension traits are auto-imported via prelude
//! async fn my_tool(ctx: &Context<'_>) {
//!     // Permission management
//!     ctx.allow_tool("follow_up");
//!
//!     // Reminders
//!     ctx.add_reminder("Check the output");
//! }
//! ```

pub mod llmmetry;
mod permission;
mod reminder;

// Re-export extension traits
pub use permission::PermissionContextExt;
pub use reminder::ReminderContextExt;

// Re-export state types
pub use permission::{PermissionState, PERMISSION_STATE_PATH};
pub use reminder::{ReminderState, REMINDER_STATE_PATH};

// Re-export plugins
pub use llmmetry::{
    AgentMetrics, GenAISpan, InMemorySink, LLMMetryPlugin, MetricsSink, ModelStats, ToolSpan,
    ToolStats,
};
pub use permission::PermissionPlugin;
pub use reminder::ReminderPlugin;
