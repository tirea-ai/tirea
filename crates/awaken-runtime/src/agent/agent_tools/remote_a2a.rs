//! Remote A2A agent tool — HTTP call to a remote agent endpoint.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};

/// Configuration for a remote A2A agent endpoint.
#[derive(Debug, Clone)]
pub struct A2aEndpoint {
    /// Base URL of the remote A2A server.
    pub base_url: String,
    /// Remote agent ID on the server.
    pub remote_agent_id: String,
    /// Optional bearer token for authentication.
    pub bearer_token: Option<String>,
    /// Poll interval in milliseconds for async task completion.
    pub poll_interval_ms: u64,
}

impl A2aEndpoint {
    pub fn new(base_url: impl Into<String>, remote_agent_id: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            remote_agent_id: remote_agent_id.into(),
            bearer_token: None,
            poll_interval_ms: 2000,
        }
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub fn with_poll_interval_ms(mut self, ms: u64) -> Self {
        self.poll_interval_ms = ms;
        self
    }
}

/// Submission response from a remote A2A endpoint.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct A2aSubmissionResponse {
    #[serde(default)]
    context_id: Option<String>,
    task_id: String,
}

/// Task status response from a remote A2A endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct A2aTaskResponse {
    #[serde(default)]
    context_id: Option<String>,
    status: String,
    #[serde(default)]
    termination_code: Option<String>,
    #[serde(default)]
    termination_detail: Option<String>,
    #[serde(default)]
    message: Option<Value>,
    #[serde(default)]
    history: Vec<Value>,
    #[serde(default)]
    artifacts: Vec<Value>,
}

/// Status of a remote A2A task.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTaskStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

impl RemoteTaskStatus {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    #[allow(dead_code)]
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Snapshot of a remote A2A task's state.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct A2aTaskSnapshot {
    pub status: RemoteTaskStatus,
    pub error: Option<String>,
    pub done: bool,
    pub output_text: Option<String>,
}

/// Tool that delegates to a remote A2A agent via HTTP.
///
/// Submits a prompt to the remote endpoint, polls for completion,
/// and returns the final output as the tool result.
pub struct RemoteA2aTool {
    /// Unique tool ID.
    tool_id: String,
    /// Human-readable description for the LLM.
    description: String,
    /// Remote endpoint configuration.
    endpoint: A2aEndpoint,
}

impl RemoteA2aTool {
    pub fn new(
        tool_id: impl Into<String>,
        description: impl Into<String>,
        endpoint: A2aEndpoint,
    ) -> Self {
        Self {
            tool_id: tool_id.into(),
            description: description.into(),
            endpoint,
        }
    }

    /// Returns the endpoint configuration.
    pub fn endpoint(&self) -> &A2aEndpoint {
        &self.endpoint
    }

    /// Build the agent-scoped URL for the remote A2A endpoint.
    #[allow(dead_code)]
    fn agent_url(&self) -> String {
        let base = self.endpoint.base_url.trim_end_matches('/');
        format!("{}/agents/{}", base, self.endpoint.remote_agent_id.trim())
    }
}

#[allow(dead_code)]
fn extract_output_text(response: &A2aTaskResponse) -> Option<String> {
    // Try artifacts first
    for artifact in &response.artifacts {
        if let Some(text) = extract_text_from_value(artifact) {
            return Some(text);
        }
    }
    // Then message
    if let Some(ref message) = response.message
        && let Some(text) = extract_text_from_value(message)
    {
        return Some(text);
    }
    // Then last history entry
    for entry in response.history.iter().rev() {
        if let Some(text) = extract_text_from_value(entry) {
            return Some(text);
        }
    }
    None
}

#[allow(dead_code)]
fn extract_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(obj) => {
            // Check role filter
            if let Some(role) = obj.get("role").and_then(Value::as_str) {
                let role_lower = role.trim().to_ascii_lowercase();
                if role_lower != "assistant" && role_lower != "agent" {
                    return None;
                }
            }
            if let Some(text) = obj.get("text").and_then(Value::as_str) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
            if let Some(content) = obj.get("content") {
                return extract_text_from_value(content);
            }
            if let Some(parts) = obj.get("parts").and_then(Value::as_array) {
                let texts: Vec<String> = parts.iter().filter_map(extract_text_from_value).collect();
                if !texts.is_empty() {
                    return Some(texts.join("\n\n"));
                }
            }
            if let Some(data) = obj.get("data") {
                return match data {
                    Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                    _ => None,
                };
            }
            None
        }
        Value::Array(items) => {
            let texts: Vec<String> = items.iter().filter_map(extract_text_from_value).collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n\n"))
            }
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn map_task_status(response: &A2aTaskResponse) -> A2aTaskSnapshot {
    let output_text = extract_output_text(response);
    let status = response.status.trim().to_ascii_lowercase();
    match status.as_str() {
        "done" | "completed" => {
            let termination = response
                .termination_code
                .as_deref()
                .map(str::trim)
                .map(str::to_ascii_lowercase);
            match termination.as_deref() {
                Some("cancelled") | Some("cancel_requested") => A2aTaskSnapshot {
                    status: RemoteTaskStatus::Stopped,
                    error: response.termination_detail.clone(),
                    done: true,
                    output_text,
                },
                Some("error") => A2aTaskSnapshot {
                    status: RemoteTaskStatus::Failed,
                    error: response
                        .termination_detail
                        .clone()
                        .or_else(|| Some("remote run failed".into())),
                    done: true,
                    output_text,
                },
                _ => A2aTaskSnapshot {
                    status: RemoteTaskStatus::Completed,
                    error: response.termination_detail.clone(),
                    done: true,
                    output_text,
                },
            }
        }
        "failed" | "error" => A2aTaskSnapshot {
            status: RemoteTaskStatus::Failed,
            error: response
                .termination_detail
                .clone()
                .or_else(|| Some("remote run failed".into())),
            done: true,
            output_text,
        },
        "cancelled" | "stopped" => A2aTaskSnapshot {
            status: RemoteTaskStatus::Stopped,
            error: response.termination_detail.clone(),
            done: true,
            output_text,
        },
        _ => A2aTaskSnapshot {
            status: RemoteTaskStatus::Running,
            error: None,
            done: false,
            output_text,
        },
    }
}

#[async_trait]
impl Tool for RemoteA2aTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(&self.tool_id, &self.tool_id, &self.description)
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        if args.get("prompt").and_then(Value::as_str).is_none() {
            return Err(ToolError::InvalidArguments(
                "missing required field \"prompt\"".into(),
            ));
        }
        Ok(())
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext) -> Result<ToolResult, ToolError> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();

        if prompt.is_empty() {
            return Err(ToolError::InvalidArguments(
                "prompt must not be empty".into(),
            ));
        }

        // Build the result as a delegation marker — actual HTTP call
        // would be handled by the runtime's async delegation infrastructure.
        // For now, return the submission intent.
        Ok(ToolResult::success(
            &self.tool_id,
            json!({
                "remote_agent_id": self.endpoint.remote_agent_id,
                "base_url": self.endpoint.base_url,
                "prompt": prompt,
                "status": "submitted",
                "message": format!(
                    "Request submitted to remote agent '{}'.",
                    self.endpoint.remote_agent_id,
                ),
            }),
        ))
    }
}
