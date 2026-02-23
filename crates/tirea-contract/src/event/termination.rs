use serde::{Deserialize, Serialize};

/// Generic stopped payload emitted when a plugin decides to terminate.
///
/// `code` is a stable, machine-readable reason id.
/// `detail` is optional human-readable context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoppedReason {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StoppedReason {
    #[must_use]
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }

    #[must_use]
    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }
}

/// Why a run terminated.
///
/// This is the top-level lifecycle exit reason for a run. A stop-condition hit is
/// represented by `Stopped(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TerminationReason {
    /// LLM returned a response with no tool calls.
    NaturalEnd,
    /// A plugin requested inference skip.
    PluginRequested,
    /// A configured stop condition fired.
    Stopped(StoppedReason),
    /// External run cancellation signal was received.
    Cancelled,
    /// Run paused waiting for external suspended tool-call resolution.
    Suspended,
    /// Run ended due to an error path.
    Error,
}

impl TerminationReason {
    #[must_use]
    pub fn stopped(code: impl Into<String>) -> Self {
        Self::Stopped(StoppedReason::new(code))
    }

    #[must_use]
    pub fn stopped_with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Stopped(StoppedReason::with_detail(code, detail))
    }
}
