//! Internal suspension mailboxes used by interaction extensions.
//!
//! These are durable runtime states managed by the loop and interaction plugins:
//! - `__resume_tool_calls`
//! - `__resolved_suspensions`

pub use tirea_contract::runtime::control::{ResolvedSuspensionsState, ResumeToolCallsState};
