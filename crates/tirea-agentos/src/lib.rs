//! Plugin orchestration, sub-agent management, and lifecycle composition for AgentOS.
//!
//! - [`composition`]: agent definitions, builder, registries, and wiring.
//! - [`runtime`]: run preparation, execution, stop policies, and background tasks.
//! - [`extensions`]: feature-gated bridges to permission, reminder, skills, observability, and MCP.
#![allow(missing_docs)]

pub use tirea_contract as contracts;

// Internal aliases for tirea-agent-loop modules (used throughout runtime/).
pub(crate) mod loop_engine {
    pub use tirea_agent_loop::engine::*;
}
pub(crate) mod loop_runtime {
    pub use tirea_agent_loop::runtime::*;
}

pub mod composition;
pub mod extensions;
pub mod runtime;

// ── Top-level re-exports for common entry points ────────────────────────

pub use composition::{AgentDefinition, AgentOsBuilder, RegistrySet, ToolBehaviorBundle};
pub use runtime::{AgentOs, AgentOsRunError, PreparedRun, RunStream};
