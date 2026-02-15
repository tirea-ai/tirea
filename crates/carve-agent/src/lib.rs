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
//! The crate is organized into five explicit layers:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────┐
//! │ orchestrator                                          │
//! │ - AgentOS, registries, bundle composition, wiring     │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │ runtime                                               │
//! │ - loop runners, streaming, state commit adapters      │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │ engine                                                │
//! │ - pure conversion, tool execution, stop conditions    │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │ extensions                                            │
//! │ - skills / interaction / permission / reminder / obs  │
//! └─────────────────────────────────────────────────────┘
//!                          │
//!                          ▼
//! ┌─────────────────────────────────────────────────────┐
//! │ contracts                                             │
//! │ - tool/thread/state/plugin contracts                  │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! # Core Concepts
//!
//! - **Tool**: Trait for implementing agent tools (reads/writes state via Context)
//! - **Context**: Provides typed state access with automatic patch collection
//! - **Thread**: Immutable conversation state with messages and patches
//! - **StreamCollector**: Collects streaming LLM responses
//! - **ThreadWriter/ThreadReader**: Traits for thread persistence
//!
//! # Example: Implementing a Tool
//!
//! ```ignore
//! use carve_agent::contracts::traits::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
//! use carve_agent::prelude::Context;
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
//! use carve_agent::contracts::storage::ThreadWriter;
//! use carve_agent::thread::Thread;
//! use carve_agent::types::Message;
//! use carve_thread_store_adapters::FileStore;
//!
//! // Create or load session
//! let thread = Thread::new("session-1")
//!     .with_message(Message::user("Hello"))
//!     .with_message(Message::assistant("Hi!"));
//!
//! // Save session
//! let storage = FileStore::new("./sessions");
//! storage.save(&thread).await?;
//! ```

pub mod contracts;
pub mod engine;
pub mod extensions;
pub mod orchestrator;
pub mod runtime;

pub mod prelude;
pub mod thread;
pub mod thread_store;
pub mod types;
