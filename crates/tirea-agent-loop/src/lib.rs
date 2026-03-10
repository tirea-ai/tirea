//! LLM inference engine, tool dispatch, and streaming execution loop.
//!
//! - [`engine`]: context window management, token estimation, and tool execution helpers.
//! - [`runtime`]: the core agent loop, streaming, and run lifecycle coordination.
#![allow(missing_docs)]

pub use tirea_contract as contracts;
pub mod engine;
pub mod runtime;
