//! Immutable state-driven agent framework built on carve-state.
//!
//! This crate provides a framework for building agents where all state changes
//! are tracked through patches, enabling:
//! - Full traceability of state changes
//! - State replay and time-travel debugging
//! - Component isolation through typed state views
//!
//! # Core Concepts
//!
//! - **Context**: Provides typed state access to components (from carve-state)
//! - **StateManager**: Manages immutable state with patch history (from carve-state)
//! - **Tool**: Executes actions and modifies state
//! - **ContextProvider**: Injects context messages based on state
//! - **SystemReminder**: Generates reminder messages based on state
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::{Context, Tool, ToolResult, ToolError, ToolDescriptor};
//! use carve_state::State;
//! use carve_state_derive::State;
//! use async_trait::async_trait;
//! use serde::{Serialize, Deserialize};
//! use serde_json::Value;
//!
//! #[derive(Serialize, Deserialize, State)]
//! pub struct MyToolState {
//!     pub count: i64,
//! }
//!
//! struct MyTool;
//!
//! #[async_trait]
//! impl Tool for MyTool {
//!     fn descriptor(&self) -> ToolDescriptor {
//!         ToolDescriptor::new("counter", "Counter", "Increment a counter")
//!     }
//!
//!     async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
//!         let state = ctx.call_state::<MyToolState>();
//!         let current = state.count().unwrap_or(0);
//!         state.set_count(current + 1);
//!
//!         // No need to call finish() - changes are auto-collected
//!         Ok(ToolResult::success("counter", serde_json::json!({"count": current + 1})))
//!     }
//! }
//! ```

pub mod traits;

// Re-export from carve-state for convenience
pub use carve_state::{Context, StateManager};

// Trait exports
pub use traits::provider::{ContextCategory, ContextProvider};
pub use traits::reminder::SystemReminder;
pub use traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};
