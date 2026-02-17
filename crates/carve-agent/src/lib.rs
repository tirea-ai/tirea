//! Backward-compatible facade crate for carve agent modules.
//!
//! - `carve-agent-loop`: core loop/runtime/engine
//! - `carve-agentos`: registry + orchestration (AgentOS)
//! - extension re-exports and prelude remain here for compatibility

pub use carve_agent_loop::contracts;
pub use carve_agent_loop::{engine, runtime};
pub use carve_agentos::{extensions, orchestrator};
pub mod prelude;
