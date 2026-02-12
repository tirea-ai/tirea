//! Backward-compatible re-exports from the `thread` module.
//!
//! The canonical types are `Thread` and `ThreadMetadata` in the `thread` module.
//! This module provides `Session` and `SessionMetadata` aliases for compatibility.

pub use crate::thread::{Session, SessionMetadata, Thread, ThreadMetadata};
