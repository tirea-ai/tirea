//! Immutable state-driven agent framework built on carve-state.
//!
//! This crate provides a framework for building agents where all state changes
//! are tracked through patches, enabling:
//! - Full traceability of state changes
//! - State replay and time-travel debugging
//! - Component isolation through typed state views
//!
//! # Architecture
//!
//! The framework is designed around pure functions and minimal abstractions:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │  Application Layer                                   │
//! │  - Register tools, run agent loop, persist sessions  │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  Pure Functions                                      │
//! │  - build_request, StreamCollector, Session::with_*  │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  Tool Layer                                          │
//! │  - Tool trait, Context, automatic patch collection   │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Core Concepts
//!
//! - **Tool**: Trait for implementing agent tools (reads/writes state via Context)
//! - **Context**: Provides typed state access with automatic patch collection
//! - **Session**: Immutable conversation state with messages and patches
//! - **StreamCollector**: Collects streaming LLM responses
//! - **Storage**: Trait for session persistence
//!
//! # Example: Implementing a Tool
//!
//! ```ignore
//! use carve_agent::{Tool, ToolDescriptor, ToolResult, ToolError, Context};
//! use async_trait::async_trait;
//! use serde_json::{json, Value};
//!
//! struct CalculatorTool;
//!
//! #[async_trait]
//! impl Tool for CalculatorTool {
//!     fn descriptor(&self) -> ToolDescriptor {
//!         ToolDescriptor::new("calculator", "Calculator", "Evaluate expressions")
//!             .with_parameters(json!({
//!                 "type": "object",
//!                 "properties": { "expr": { "type": "string" } },
//!                 "required": ["expr"]
//!             }))
//!     }
//!
//!     async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
//!         let expr = args["expr"].as_str().unwrap_or("0");
//!         // ... evaluate expression ...
//!         Ok(ToolResult::success("calculator", json!({"result": 42})))
//!     }
//! }
//! ```
//!
//! # Example: Using Sessions
//!
//! ```ignore
//! use carve_agent::{Session, Message, Storage, FileStorage};
//!
//! // Create or load session
//! let session = Session::new("session-1")
//!     .with_message(Message::user("Hello"))
//!     .with_message(Message::assistant("Hi!"));
//!
//! // Save session
//! let storage = FileStorage::new("./sessions");
//! storage.save(&session).await?;
//! ```

pub mod convert;
pub mod execute;
pub mod r#loop;
pub mod session;
pub mod storage;
pub mod stream;
pub mod traits;
pub mod types;

// Re-export from carve-state for convenience
pub use carve_state::{Context, StateManager, TrackedPatch};

// Trait exports
pub use traits::provider::{ContextCategory, ContextProvider};
pub use traits::reminder::SystemReminder;
pub use traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};

// Type exports
pub use types::{Message, Role, ToolCall};

// Session exports
pub use session::{Session, SessionMetadata};

// Storage exports
pub use storage::{FileStorage, MemoryStorage, Storage, StorageError};

// Stream exports
pub use stream::{AgentEvent, StreamCollector, StreamOutput, StreamResult};

// Execute exports
pub use execute::{
    collect_patches, execute_single_tool, execute_tools_parallel, execute_tools_sequential,
    ToolExecution,
};

// Convert exports (pure functions)
pub use convert::{
    assistant_message, assistant_tool_calls, build_request, to_chat_message, to_genai_tool,
    tool_response, user_message,
};

// Loop exports
pub use r#loop::{
    execute_tools as loop_execute_tools, run_loop, run_loop_stream, run_step, run_turn,
    tool_map, tool_map_from_arc, AgentConfig, AgentLoopError, StepResult,
};
