//! Unified umbrella crate for the carve agent framework.
//!
//! Use feature flags to control which modules are included:
//!
//! | Feature    | What it enables                                  |
//! |------------|--------------------------------------------------|
//! | `core`     | Agent loop + AgentOS orchestrator (**default**)   |
//! | `ag-ui`    | AG-UI protocol adapters                          |
//! | `ai-sdk-v6`| Vercel AI SDK v6 protocol adapters               |
//! | `mcp`      | MCP tool registry integration                    |
//! | `postgres` | PostgreSQL thread store backend                  |
//! | `nats`     | NATS JetStream thread store backend              |
//! | `full`     | All of the above                                 |
//!
//! # Quick start
//!
//! ```toml
//! [dependencies]
//! carve-agent = { version = "0.1", features = ["ag-ui"] }
//! ```
//!
//! ```ignore
//! use carve_agent::prelude::*;
//! ```

// ── Always available: contracts + state ─────────────────────────────────

/// Core agent contracts: traits, data models, events, and tool/plugin SPI.
pub use carve_agent_contract as contracts;

/// Typed JSON state engine (patches, ops, paths, state manager).
pub use carve_state as state;

// ── Core (default): loop + orchestrator ─────────────────────────────────

/// Low-level agent execution engine (inference, tool execution, stop policies).
#[cfg(feature = "core")]
pub use carve_agent_loop::engine;

/// Agent loop runtime (loop runner, streaming, activity tracking).
#[cfg(feature = "core")]
pub use carve_agent_loop::runtime;

/// AgentOS orchestrator extensions (permission, reminder, interaction, etc.).
#[cfg(feature = "core")]
pub use carve_agentos::extensions;

/// AgentOS orchestrator (agent registry, builder, run management).
#[cfg(feature = "core")]
pub use carve_agentos::orchestrator;

// ── Protocols ───────────────────────────────────────────────────────────

/// AG-UI protocol types, adapters, and runtime wiring.
#[cfg(feature = "ag-ui")]
pub use carve_protocol_ag_ui as ag_ui;

/// Vercel AI SDK v6 protocol types and adapters.
#[cfg(feature = "ai-sdk-v6")]
pub use carve_protocol_ai_sdk_v6 as ai_sdk_v6;

// ── Extensions ────────────────────────────────────────────────────────

/// Skill subsystem (discovery, activation, resource loading, scripts).
#[cfg(feature = "skills")]
pub use carve_agent_extension_skills as skills;

// ── Storage backends ────────────────────────────────────────────────────

/// Thread store adapters (file, memory, postgres, nats).
#[cfg(any(feature = "postgres", feature = "nats"))]
pub use carve_thread_store_adapters as store;

// ── Prelude ─────────────────────────────────────────────────────────────

pub mod prelude;
