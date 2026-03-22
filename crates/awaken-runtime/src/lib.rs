#![allow(missing_docs)]

pub mod agent;
pub mod engine;
pub mod plugins;
pub mod registry;
pub mod runtime;
pub mod state;

// ── plugins ──
pub use plugins::{Plugin, PluginDescriptor, PluginRegistrar};

// ── runtime ──
pub use runtime::{
    AgentResolver, AgentRuntime, AppRuntime, CancellationToken, DEFAULT_MAX_PHASE_ROUNDS,
    ExecutionEnv, PhaseContext, PhaseHook, PhaseRuntime, ResolvedAgent, RunHandle, RunRequest,
    ToolPermission, ToolPermissionChecker, ToolPermissionResult, TypedEffectHandler,
    TypedScheduledActionHandler, aggregate_tool_permissions,
};

// ── state ──
pub use state::{CommitEvent, CommitHook, MutationBatch, StateCommand, StateStore};
