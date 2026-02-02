//! Typed state + JSON patch library for deterministic immutable state management.
//!
//! `carve-state` provides typed access to JSON state with automatic patch collection,
//! enabling deterministic state transitions and full replay capability.
//!
//! # Core Concepts
//!
//! - **State**: Trait for types that can create typed state references
//! - **StateRef**: Generated typed accessor for reading and writing state
//! - **PatchSink**: Automatic operation collector (transparent to developers)
//! - **Context**: Provides typed state access with automatic patch collection
//! - **StateManager**: Manages immutable state with patch history and replay
//! - **Patch**: A serializable record of operations to apply to state
//!
//! # Deterministic State Transitions
//!
//! ```text
//! State' = apply_patch(State, Patch)
//! ```
//!
//! - Same `(State, Patch)` always produces the same `State'`
//! - `apply_patch` is a pure function that never mutates its input
//! - Full history enables replay to any point in time
//!
//! # Quick Start
//!
//! ```
//! use carve_state::{apply_patch, Patch, Op, path};
//! use serde_json::json;
//!
//! // Create initial state
//! let state = json!({"count": 0, "name": "counter"});
//!
//! // Build a patch
//! let patch = Patch::new()
//!     .with_op(Op::set(path!("count"), json!(10)))
//!     .with_op(Op::set(path!("updated"), json!(true)));
//!
//! // Apply patch (pure function)
//! let new_state = apply_patch(&state, &patch).unwrap();
//!
//! assert_eq!(new_state["count"], 10);
//! assert_eq!(new_state["updated"], true);
//! assert_eq!(state["count"], 0); // Original unchanged
//! ```
//!
//! # Using Typed State (with derive macro)
//!
//! For type-safe access with automatic patch collection:
//!
//! ```ignore
//! use carve_state::{Context, State};
//! use carve_state_derive::State;
//! use serde::{Serialize, Deserialize};
//! use serde_json::json;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize, State)]
//! struct Counter {
//!     value: i64,
//!     label: String,
//! }
//!
//! // In a tool implementation:
//! async fn execute(&self, ctx: &Context<'_>) -> Result<()> {
//!     let counter = ctx.state::<Counter>("counters.main");
//!
//!     // Read
//!     let current = counter.value()?;
//!
//!     // Write (automatically collected)
//!     counter.set_value(current + 1);
//!     counter.set_label("Updated");
//!
//!     Ok(())
//! }
//! // Framework calls ctx.take_patch() after execution
//! ```
//!
//! # Using JsonWriter
//!
//! For dynamic JSON manipulation without typed structs:
//!
//! ```
//! use carve_state::{JsonWriter, path};
//! use serde_json::json;
//!
//! let mut w = JsonWriter::new();
//! w.set(path!("user", "name"), json!("Alice"));
//! w.append(path!("user", "roles"), json!("admin"));
//! w.increment(path!("user", "login_count"), 1i64);
//!
//! let patch = w.build();
//! ```

mod apply;
mod conflict;
mod context;
mod error;
mod manager;
mod op;
mod patch;
mod path;
mod state;
mod writer;

// Core types
pub use apply::{apply_patch, apply_patches, get_at_path};
pub use conflict::{compute_touched, detect_conflicts, Conflict, ConflictKind, PatchExt};
pub use error::{value_type_name, CarveError, CarveResult};
pub use op::{Number, Op};
pub use patch::{Patch, TrackedPatch};
pub use path::{Path, Seg};
pub use writer::JsonWriter;

// State types
pub use context::{parse_path, Context};
pub use manager::{ApplyResult, StateError, StateManager};
pub use state::{PatchSink, State, StateExt};

// Re-export derive macro when feature is enabled
#[cfg(feature = "derive")]
pub use carve_state_derive::State;

// Re-export serde_json::Value for convenience
pub use serde_json::Value;
