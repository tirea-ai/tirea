//! Extension bundles: skills, interaction, policy, reminders, observability.

pub mod interaction;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod observability;
pub mod permission;
pub mod reminder;
pub mod skills;
