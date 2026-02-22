//! Canonical top-level state paths shared across runtime crates.

/// Durable skills state (`SkillState`).
pub const SKILLS_STATE_PATH: &str = "skills";

/// Durable interaction outbox used by interaction extensions and loop runner.
pub const INTERACTION_OUTBOX_STATE_PATH: &str = "interaction_outbox";

/// Durable permission state (`PermissionState`).
pub const PERMISSIONS_STATE_PATH: &str = "permissions";
