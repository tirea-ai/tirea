//! Canonical top-level state paths shared across runtime crates.

/// Durable skills state (`SkillState`).
pub const SKILLS_STATE_PATH: &str = "skills";

/// Durable suspended tool-call map (`SuspendedToolCallsState`).
pub const SUSPENDED_TOOL_CALLS_STATE_PATH: &str = "__suspended_tool_calls";

/// Durable per-call runtime lifecycle state (`ToolCallLifecycleStatesState`).
pub const TOOL_CALL_STATES_STATE_PATH: &str = "__tool_call_states";

/// Durable inference-error envelope (`InferenceErrorState`).
pub const INFERENCE_ERROR_STATE_PATH: &str = "__inference_error";

/// Durable run lifecycle envelope (`RunLifecycleState`).
pub const RUN_LIFECYCLE_STATE_PATH: &str = "__run";
