//! Backward-compatible facade crate for carve agent modules.
//!
//! - `carve-agent-loop`: core loop/runtime/engine
//! - `carve-agentos`: registry + orchestration (AgentOS)
//! - extension re-exports and prelude remain here for compatibility

pub use carve_agent_contract as contracts;
pub use carve_agent_loop::{engine, runtime};
pub use carve_agentos::orchestrator;
pub mod extensions;
pub mod prelude;
