use tirea_state_derive::State;
use serde::{Deserialize, Serialize};

/// A web resource found during research.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Resource {
    pub id: String,
    pub url: String,
    pub title: String,
    pub description: String,
}

/// A log entry for real-time progress tracking (Pattern 4: Generative UI).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogEntry {
    pub message: String,
    pub level: String,
    pub step: String,
}

/// Root state for the research canvas example.
///
/// Corresponds to CopilotKit's research-canvas agent state.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct ResearchState {
    pub research_question: String,
    pub report: String,
    pub resources: Vec<Resource>,
    pub logs: Vec<LogEntry>,
}
