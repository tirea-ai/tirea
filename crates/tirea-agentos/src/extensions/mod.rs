//! Extension bundles: skills, policy, reminders, observability.

#[cfg(feature = "mcp")]
pub mod mcp;
pub mod observability;
pub mod permission;
pub mod reminder;
pub mod skills;
