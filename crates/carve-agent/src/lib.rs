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

pub mod activity;
pub mod ag_ui;
pub mod agent;
pub mod agent_os;
pub mod convert;
pub mod execute;
pub mod r#loop;
pub mod phase;
pub mod plugin;
pub mod plugins;
pub mod prelude;
pub mod session;
pub mod skills;
pub mod state_types;
pub mod storage;
pub mod stream;
pub mod traits;
pub mod types;
pub mod ui_stream;

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

// Activity exports
pub use activity::ActivityHub;

// Skills exports
pub use skills::{
    LoadSkillReferenceTool, SkillActivateTool, SkillDiscoveryPlugin, SkillPlugin, SkillRegistry,
    SkillRuntimePlugin, SkillScriptTool, SkillSubsystem, SkillSubsystemError,
};

// UI Stream exports (AI SDK v6 compatible)
pub use ui_stream::{
    AiSdkAdapter, StreamState, ToolState, UIMessage, UIMessagePart, UIRole, UIStreamEvent,
};

// AG-UI exports (CopilotKit compatible)
pub use ag_ui::{
    run_agent_stream, run_agent_stream_sse, run_agent_stream_sse_with_parent,
    run_agent_stream_with_parent, AGUIContext, AGUIEvent, AGUIMessage, AGUIToolDef, AgUiAdapter,
    RequestError, RunAgentRequest, ToolExecutionLocation,
};

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

// Agent exports
pub use agent::{filter_tools, Agent, SubAgentHandle, SubAgentResult, SubAgentTool};
pub use agent_os::{
    AgentOs, AgentOsBuildError, AgentOsBuilder, AgentOsResolveError, AgentOsRunError,
    AgentOsWiringError, SkillsConfig, SkillsMode,
};

// Loop exports
pub use r#loop::{
    execute_tools as loop_execute_tools, execute_tools_with_config, execute_tools_with_plugins,
    run_loop, run_loop_stream, run_round, run_step, tool_map, tool_map_from_arc, AgentConfig,
    AgentDefinition, AgentLoopError, RoundResult,
};

// Plugin exports
pub use plugin::AgentPlugin;

// Phase exports
pub use phase::{Phase, StepContext, StepOutcome, ToolContext};

// State types exports
pub use state_types::{Interaction, InteractionResponse, ToolPermissionBehavior};

// Plugins and extension traits
pub use plugins::{
    // Built-in plugins
    AgentMetrics,
    GenAISpan,
    InMemorySink,
    LLMMetryPlugin,
    MetricsSink,
    // Extension traits
    PermissionContextExt,
    PermissionPlugin,
    // State types
    PermissionState,
    ReminderContextExt,
    ReminderPlugin,
    ReminderState,
    ToolSpan,
};
