#![allow(missing_docs)]

//! Awaken framework primitives.

pub mod agent;
pub mod contract;
pub mod engine;
mod error;
mod model;
mod plugins;
pub mod registry;
mod runtime;
mod state;

// ── error ──
pub use error::{StateError, UnknownKeyPolicy};

// ── model ──
pub use model::{
    EffectSpec, FailedScheduledActions, JsonValue, PendingScheduledActions, Phase,
    ScheduledActionSpec, TypedEffect,
};

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
pub use state::{
    CommitEvent, CommitHook, KeyScope, MergeStrategy, MutationBatch, PersistedState, Snapshot,
    StateCommand, StateKey, StateKeyOptions, StateStore,
};
