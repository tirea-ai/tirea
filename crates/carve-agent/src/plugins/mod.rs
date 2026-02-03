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
//! - [`ExecutionContextExt`] - Block execution or request user interaction
//! - [`PermissionContextExt`] - Manage tool permissions (allow/deny/ask)
//! - [`MessageContextExt`] - Manage conversation messages
//! - [`ReminderContextExt`] - Add reminders for LLM context
//! - [`ContextDataExt`] - Generic key-value storage
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
//!     // Execution control
//!     if should_block {
//!         ctx.block("Not allowed");
//!         return;
//!     }
//!
//!     // Messages
//!     ctx.append_message(Message::assistant("Done!"));
//!
//!     // Reminders
//!     ctx.add_reminder("Check the output");
//!
//!     // Generic context data
//!     ctx.set("last_action", json!("my_tool"));
//! }
//! ```

mod context_data;
mod execution;
mod message;
mod permission;
mod reminder;

// Re-export extension traits
pub use context_data::ContextDataExt;
pub use execution::ExecutionContextExt;
pub use message::MessageContextExt;
pub use permission::PermissionContextExt;
pub use reminder::ReminderContextExt;

// Re-export state types
pub use context_data::{ContextDataState, CONTEXT_DATA_STATE_PATH};
pub use execution::{ExecutionState, EXECUTION_STATE_PATH};
pub use message::{MessageState, MESSAGE_STATE_PATH};
pub use permission::{PermissionState, PERMISSION_STATE_PATH};
pub use reminder::{ReminderState, REMINDER_STATE_PATH};

// Re-export plugins
pub use permission::PermissionPlugin;
pub use reminder::ReminderPlugin;
