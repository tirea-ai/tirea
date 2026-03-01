pub mod scope_registry;
pub mod spec;

pub use scope_registry::StateScopeRegistry;
pub use spec::{reduce_state_actions, AnyStateAction, StateScope, StateSpec};
