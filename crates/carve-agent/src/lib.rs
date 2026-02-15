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
//! use carve_agent::{Thread, Message, ThreadWriter};
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

pub mod activity;
pub mod agent_os;
pub mod convert;
pub mod execute;
pub mod interaction;
pub mod r#loop;
pub mod phase;
pub mod plugin;
pub mod plugins;
pub mod prelude;
pub mod skills;
pub mod state_types;
pub mod stop;
pub mod stream;
pub mod thread;
pub mod thread_store;
pub mod traits;
pub mod types;

#[cfg(feature = "mcp")]
pub mod mcp_registry;

// Re-export from carve-state for convenience
pub use carve_state::{Context, StateManager, TrackedPatch};

// Contracts exports
pub use contracts::conversation::{
    gen_message_id, Message, MessageMetadata, PendingDelta, Role, Thread, ThreadMetadata, ToolCall,
    Visibility,
};
pub use contracts::plugin::{
    AgentPlugin, ContextCategory, ContextProvider, Phase, StepContext, StepOutcome, SystemReminder,
    ToolContext,
};
pub use contracts::state::{
    AgentRunState, AgentRunStatus, AgentState, Interaction, InteractionResponse,
    ToolPermissionBehavior, AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX,
    AGENT_STATE_PATH,
};
pub use contracts::storage::{
    CheckpointReason, Committed, MessagePage, MessageQuery, MessageWithCursor, SortOrder,
    ThreadDelta, ThreadHead, ThreadListPage, ThreadListQuery, ThreadReader, ThreadStore,
    ThreadStoreError, ThreadSync, ThreadWriter, Version,
};
pub use contracts::tools::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};

// Runtime exports
pub use runtime::activity::ActivityHub;
pub use runtime::loop_runner::{
    execute_tools as loop_execute_tools, execute_tools_with_config, execute_tools_with_plugins,
    run_loop, run_loop_stream, run_round, run_step, tool_map, tool_map_from_arc, AgentConfig,
    AgentDefinition, AgentLoopError, ChannelStateCommitter, RoundResult, RunContext,
    StateCommitError, StateCommitter,
};
pub use runtime::streaming::{AgentEvent, StreamCollector, StreamOutput, StreamResult};

// Extensions exports
pub use extensions::skills::{
    CompositeSkillRegistry, CompositeSkillRegistryError, FsSkillRegistry, InMemorySkillRegistry,
    LoadSkillResourceTool, SkillActivateTool, SkillDiscoveryPlugin, SkillPlugin, SkillRegistry,
    SkillRegistryError, SkillRegistryWarning, SkillResource, SkillResourceKind, SkillRuntimePlugin,
    SkillScriptTool, SkillSubsystem, SkillSubsystemError,
};
pub use extensions::{
    observability::{
        AgentMetrics, GenAISpan, InMemorySink, LLMMetryPlugin, MetricsSink, ModelStats, ToolSpan,
        ToolStats,
    },
    permission::{PermissionContextExt, PermissionPlugin, PermissionState},
    reminder::{ReminderContextExt, ReminderPlugin, ReminderState},
};

// Engine exports
pub use engine::convert::{
    assistant_message, assistant_tool_calls, build_request, to_chat_message, to_genai_tool,
    tool_response, user_message,
};
pub use engine::stop_conditions::{
    check_stop_conditions, ConsecutiveErrors, ContentMatch, LoopDetection, MaxRounds,
    StopCheckContext, StopCondition, StopConditionSpec, StopOnTool, StopReason, Timeout,
    TokenBudget,
};
pub use engine::tool_execution::{
    collect_patches, execute_single_tool, execute_single_tool_with_runtime, execute_tools_parallel,
    execute_tools_sequential, ToolExecution,
};

// Orchestrator exports
pub use orchestrator::{
    AgentOs, AgentOsBuildError, AgentOsBuilder, AgentOsResolveError, AgentOsRunError,
    AgentOsWiringError, AgentRegistry, AgentRegistryError, AgentToolsConfig, BundleComposeError,
    BundleComposer, BundleRegistryAccumulator, BundleRegistryKind, CompositeAgentRegistry,
    CompositeModelRegistry, CompositePluginRegistry, CompositeProviderRegistry,
    CompositeToolRegistry, InMemoryAgentRegistry, InMemoryModelRegistry, InMemoryPluginRegistry,
    InMemoryProviderRegistry, InMemoryToolRegistry, ModelDefinition, ModelRegistry,
    ModelRegistryError, PluginRegistry, PluginRegistryError, ProviderRegistry,
    ProviderRegistryError, RegistryBundle, RegistrySet, RunExtensions, RunRequest, RunStream,
    SkillsConfig, SkillsMode, ToolPluginBundle, ToolRegistry, ToolRegistryError,
};
