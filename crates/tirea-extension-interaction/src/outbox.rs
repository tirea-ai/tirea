//! Internal suspension mailbox used by interaction extensions.
//!
//! Durable runtime rendezvous:
//! - `__resume_decisions`

pub use tirea_contract::runtime::control::{
    ResolvedSuspensionsState, ResumeDecisionsState, ResumeToolCallsState,
};
