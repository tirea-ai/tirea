#![allow(missing_docs)]

// Re-export contract crate
pub use awaken_contract::{
    // registry spec
    AgentSpec,
    // model
    EffectSpec,
    FailedScheduledActions,
    JsonValue,
    // state (contract-level)
    KeyScope,
    MergeStrategy,
    PendingScheduledActions,
    PersistedState,
    Phase,
    PluginConfigKey,
    ScheduledActionSpec,
    Snapshot,
    StateError,
    StateKey,
    StateKeyOptions,
    StateMap,
    TypedEffect,
    UnknownKeyPolicy,
};

pub use awaken_contract::contract;
pub use awaken_contract::model;
pub use awaken_contract::registry_spec;

// Re-export runtime crate
pub use awaken_runtime::{
    // runtime
    AgentResolver,
    AgentRuntime,
    AppRuntime,
    CancellationToken,
    // state (runtime-level)
    CommitEvent,
    CommitHook,
    DEFAULT_MAX_PHASE_ROUNDS,
    ExecutionEnv,
    MutationBatch,
    PhaseContext,
    PhaseHook,
    PhaseRuntime,
    // plugins
    Plugin,
    PluginDescriptor,
    PluginRegistrar,
    ResolvedAgent,
    RunHandle,
    RunRequest,
    StateCommand,
    StateStore,
    ToolPermission,
    ToolPermissionChecker,
    ToolPermissionResult,
    TypedEffectHandler,
    TypedScheduledActionHandler,
    aggregate_tool_permissions,
};

pub use awaken_runtime::agent;
pub use awaken_runtime::engine;
pub use awaken_runtime::plugins;
pub use awaken_runtime::registry;
pub use awaken_runtime::runtime;

/// Re-export combined state module. Use `awaken::state` for all state types.
pub mod state {
    pub use awaken_contract::state::*;
    pub use awaken_runtime::state::{
        CommitEvent, CommitHook, MutationBatch, StateCommand, StateStore,
    };
}
