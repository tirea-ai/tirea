//! Canonical top-level state paths shared across runtime crates.

/// Durable skills state (`SkillState`).
pub const SKILLS_STATE_PATH: &str = "skills";

/// Durable suspended tool-call map (`SuspendedToolCallsState`).
pub const SUSPENDED_TOOL_CALLS_STATE_PATH: &str = "__suspended_tool_calls";

/// Durable resume queue (`ResumeToolCallsState`).
pub const RESUME_TOOL_CALLS_STATE_PATH: &str = "__resume_tool_calls";

/// Durable suspension resolution records (`ResolvedSuspensionsState`).
pub const RESOLVED_SUSPENSIONS_STATE_PATH: &str = "__resolved_suspensions";

/// Durable inference-error envelope (`InferenceErrorState`).
pub const INFERENCE_ERROR_STATE_PATH: &str = "__inference_error";

/// Durable permission state (`PermissionState`).
pub const PERMISSIONS_STATE_PATH: &str = "permissions";
