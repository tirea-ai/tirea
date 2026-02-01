//! Typed view + JSON patch library for deterministic immutable state management.
//!
//! `carve-state` provides a way to work with JSON documents using strongly-typed
//! Rust structs while maintaining deterministic state transitions through patches.
//!
//! # Core Concepts
//!
//! - **State**: The entire application state is stored as a JSON `Value`
//! - **View**: A typed struct that represents a portion of the state
//! - **Reader**: Provides read-only access to state through typed field accessors
//! - **Writer**: Accumulates changes as operations, producing a `Patch`
//! - **Patch**: A serializable record of operations to apply to state
//! - **apply_patch**: Pure function that applies a patch to produce new state
//!
//! # Deterministic State Transitions
//!
//! ```text
//! State' = apply_patch(State, Patch)
//! ```
//!
//! - Same `(State, Patch)` always produces the same `State'`
//! - `apply_patch` is a pure function that never mutates its input
//! - Both State and Patch are JSON-serializable
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
//! # Using JsonWriter
//!
//! For dynamic JSON manipulation, use `JsonWriter`:
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
//!
//! # Typed Views (with derive macro)
//!
//! For type-safe access, use the derive macro (requires `derive` feature):
//!
//! ```
//! use carve_state::{CarveViewModel, CarveViewModelExt, apply_patch};
//! use carve_state_derive::CarveViewModel;
//! use serde::{Serialize, Deserialize};
//! use serde_json::json;
//!
//! #[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
//! struct User {
//!     name: String,
//!     age: u32,
//!     roles: Vec<String>,
//! }
//!
//! // Read with typed accessors
//! let doc = json!({"name": "Alice", "age": 30, "roles": ["admin"]});
//! let reader = User::read(&doc);
//! assert_eq!(reader.name().unwrap(), "Alice");
//! assert_eq!(reader.age().unwrap(), 30);
//!
//! // Write with typed setters
//! let mut writer = User::write();
//! writer.name("Bob");
//! writer.age(25);
//! writer.roles_push("moderator");
//! let patch = writer.build();
//!
//! // Apply patch to get new state
//! let new_doc = apply_patch(&doc, &patch).unwrap();
//! assert_eq!(new_doc["name"], "Bob");
//! ```

mod apply;
mod conflict;
mod error;
mod op;
mod patch;
mod path;
mod view;
mod writer;

// Re-export main types
pub use apply::{apply_patch, get_at_path};
pub use conflict::{compute_touched, detect_conflicts, Conflict, ConflictKind, PatchExt};
pub use error::{value_type_name, CarveError, CarveResult};
pub use op::{Number, Op};
pub use patch::{Patch, TrackedPatch};
pub use path::{Path, Seg};
pub use view::{CarveViewModel, CarveViewModelExt};
pub use writer::{JsonWriter, WriterOps};

// Re-export derive macro when feature is enabled
#[cfg(feature = "derive")]
pub use carve_state_derive::CarveViewModel;

// Re-export serde_json::Value for convenience
pub use serde_json::Value;
