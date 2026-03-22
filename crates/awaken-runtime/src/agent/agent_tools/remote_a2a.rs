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
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct A2aSubmissionResponse {
    task_id: String,
}

/// Task status response from a remote A2A endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct A2aTaskResponse {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteTaskStatus {
    Running,
    Completed,
    Failed,
    Stopped,
}

/// Snapshot of a remote A2A task's state.
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
    fn agent_url(&self) -> String {
        let base = self.endpoint.base_url.trim_end_matches('/');
        format!("{}/agents/{}", base, self.endpoint.remote_agent_id.trim())
    }

    /// Build an HTTP client with optional authorization.
    fn build_request(
        &self,
        client: &reqwest::Client,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest::RequestBuilder {
        let builder = client.request(method, url);
        match &self.endpoint.bearer_token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    /// Submit a task to the remote A2A endpoint.
    async fn submit_task(
        &self,
        client: &reqwest::Client,
        prompt: &str,
    ) -> Result<A2aSubmissionResponse, ToolError> {
        let url = format!("{}/message:send", self.agent_url());
        let response = self
            .build_request(client, reqwest::Method::POST, &url)
            .json(&json!({ "input": prompt }))
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to submit A2A task: {e}")))?;

        let response = response
            .error_for_status()
            .map_err(|e| ToolError::ExecutionFailed(format!("A2A submission rejected: {e}")))?;

        response.json::<A2aSubmissionResponse>().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to decode A2A submission: {e}"))
        })
    }

    /// Fetch the current status of a remote task.
    async fn fetch_task_status(
        &self,
        client: &reqwest::Client,
        task_id: &str,
    ) -> Result<A2aTaskSnapshot, ToolError> {
        let url = format!("{}/tasks/{}", self.agent_url(), task_id.trim());
        let response = self
            .build_request(client, reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("failed to query A2A task: {e}")))?;

        let response = response
            .error_for_status()
            .map_err(|e| ToolError::ExecutionFailed(format!("A2A task query rejected: {e}")))?;

        let task_response = response.json::<A2aTaskResponse>().await.map_err(|e| {
            ToolError::ExecutionFailed(format!("failed to decode A2A task status: {e}"))
        })?;

        Ok(map_task_status(&task_response))
    }

    /// Poll until the task reaches a terminal state.
    async fn poll_to_completion(
        &self,
        client: &reqwest::Client,
        task_id: &str,
    ) -> Result<A2aTaskSnapshot, ToolError> {
        loop {
            let snapshot = self.fetch_task_status(client, task_id).await?;
            if snapshot.done {
                return Ok(snapshot);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(
                self.endpoint.poll_interval_ms,
            ))
            .await;
        }
    }
}

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

        let client = reqwest::Client::new();

        // Submit the task
        let submission = self.submit_task(&client, &prompt).await?;

        tracing::info!(
            task_id = %submission.task_id,
            remote_agent = %self.endpoint.remote_agent_id,
            "a2a_task_submitted"
        );

        // Poll to completion
        let snapshot = self
            .poll_to_completion(&client, &submission.task_id)
            .await?;

        match snapshot.status {
            RemoteTaskStatus::Completed => {
                let output = snapshot.output_text.unwrap_or_else(|| "(no output)".into());
                Ok(ToolResult::success(
                    &self.tool_id,
                    json!({
                        "remote_agent_id": self.endpoint.remote_agent_id,
                        "status": "completed",
                        "response": output,
                    }),
                ))
            }
            RemoteTaskStatus::Failed => {
                let error_msg = snapshot
                    .error
                    .unwrap_or_else(|| "remote agent run failed".into());
                Ok(ToolResult::error(&self.tool_id, &error_msg))
            }
            RemoteTaskStatus::Stopped => {
                let error_msg = snapshot
                    .error
                    .unwrap_or_else(|| "remote agent run was stopped".into());
                Ok(ToolResult::error(&self.tool_id, &error_msg))
            }
            RemoteTaskStatus::Running => {
                // Should not happen after poll_to_completion, but handle gracefully
                Ok(ToolResult::success(
                    &self.tool_id,
                    json!({
                        "remote_agent_id": self.endpoint.remote_agent_id,
                        "status": "running",
                        "message": "Task is still running after polling timeout.",
                    }),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_task_status_completed() {
        let response = A2aTaskResponse {
            status: "completed".into(),
            termination_code: None,
            termination_detail: None,
            message: None,
            history: vec![],
            artifacts: vec![json!({"text": "result"})],
        };
        let snapshot = map_task_status(&response);
        assert_eq!(snapshot.status, RemoteTaskStatus::Completed);
        assert!(snapshot.done);
        assert_eq!(snapshot.output_text.as_deref(), Some("result"));
    }

    #[test]
    fn map_task_status_cancelled() {
        let response = A2aTaskResponse {
            status: "done".into(),
            termination_code: Some("cancelled".into()),
            termination_detail: Some("user cancelled".into()),
            message: None,
            history: vec![],
            artifacts: vec![],
        };
        let snapshot = map_task_status(&response);
        assert_eq!(snapshot.status, RemoteTaskStatus::Stopped);
        assert!(snapshot.done);
        assert_eq!(snapshot.error.as_deref(), Some("user cancelled"));
    }

    #[test]
    fn map_task_status_failed() {
        let response = A2aTaskResponse {
            status: "failed".into(),
            termination_code: None,
            termination_detail: Some("out of memory".into()),
            message: None,
            history: vec![],
            artifacts: vec![],
        };
        let snapshot = map_task_status(&response);
        assert_eq!(snapshot.status, RemoteTaskStatus::Failed);
        assert!(snapshot.done);
    }

    #[test]
    fn map_task_status_running() {
        let response = A2aTaskResponse {
            status: "working".into(),
            termination_code: None,
            termination_detail: None,
            message: None,
            history: vec![],
            artifacts: vec![],
        };
        let snapshot = map_task_status(&response);
        assert_eq!(snapshot.status, RemoteTaskStatus::Running);
        assert!(!snapshot.done);
    }

    #[test]
    fn extract_text_from_artifacts() {
        let response = A2aTaskResponse {
            status: "completed".into(),
            termination_code: None,
            termination_detail: None,
            message: Some(json!({"role": "assistant", "content": "fallback"})),
            history: vec![],
            artifacts: vec![json!({
                "parts": [
                    {"text": "hello"},
                    {"text": "world"}
                ]
            })],
        };
        let text = extract_output_text(&response);
        assert_eq!(text.as_deref(), Some("hello\n\nworld"));
    }

    #[test]
    fn extract_text_from_message_when_no_artifacts() {
        let response = A2aTaskResponse {
            status: "completed".into(),
            termination_code: None,
            termination_detail: None,
            message: Some(json!({"role": "assistant", "text": "reply"})),
            history: vec![],
            artifacts: vec![],
        };
        let text = extract_output_text(&response);
        assert_eq!(text.as_deref(), Some("reply"));
    }

    #[test]
    fn extract_text_from_history_fallback() {
        let response = A2aTaskResponse {
            status: "completed".into(),
            termination_code: None,
            termination_detail: None,
            message: None,
            history: vec![
                json!({"role": "user", "text": "ignored"}),
                json!({"role": "assistant", "text": "last reply"}),
            ],
            artifacts: vec![],
        };
        let text = extract_output_text(&response);
        assert_eq!(text.as_deref(), Some("last reply"));
    }

    #[test]
    fn endpoint_builder() {
        let ep = A2aEndpoint::new("https://api.example.com", "agent-1")
            .with_bearer_token("tok_123")
            .with_poll_interval_ms(5000);

        assert_eq!(ep.base_url, "https://api.example.com");
        assert_eq!(ep.remote_agent_id, "agent-1");
        assert_eq!(ep.bearer_token.as_deref(), Some("tok_123"));
        assert_eq!(ep.poll_interval_ms, 5000);
    }

    #[test]
    fn endpoint_defaults() {
        let ep = A2aEndpoint::new("https://api.example.com", "worker");
        assert!(ep.bearer_token.is_none());
        assert!(ep.poll_interval_ms > 0);
    }

    #[test]
    fn descriptor_reflects_tool_id() {
        let ep = A2aEndpoint::new("https://api.example.com", "worker");
        let tool = RemoteA2aTool::new("remote_worker", "Remote worker agent", ep);
        let desc = tool.descriptor();
        assert_eq!(desc.id, "remote_worker");
        assert!(desc.description.contains("Remote worker"));
    }

    #[test]
    fn validates_prompt_required() {
        let ep = A2aEndpoint::new("https://api.example.com", "worker");
        let tool = RemoteA2aTool::new("rw", "desc", ep);

        assert!(tool.validate_args(&json!({})).is_err());
        assert!(tool.validate_args(&json!({"prompt": 42})).is_err());
        assert!(tool.validate_args(&json!({"prompt": "go"})).is_ok());
    }

    #[tokio::test]
    async fn rejects_empty_prompt() {
        let ep = A2aEndpoint::new("https://api.example.com", "worker");
        let tool = RemoteA2aTool::new("rw", "desc", ep);
        let ctx = ToolCallContext::test_default();

        let err = tool.execute(json!({"prompt": "   "}), &ctx).await;
        assert!(err.is_err());
    }

    #[test]
    fn map_task_status_done_with_error_termination() {
        let response = A2aTaskResponse {
            status: "done".into(),
            termination_code: Some("error".into()),
            termination_detail: Some("internal error".into()),
            message: None,
            history: vec![],
            artifacts: vec![],
        };
        let snapshot = map_task_status(&response);
        assert_eq!(snapshot.status, RemoteTaskStatus::Failed);
        assert!(snapshot.done);
        assert_eq!(snapshot.error.as_deref(), Some("internal error"));
    }

    #[test]
    fn extract_text_ignores_user_role_in_history() {
        // User messages should not be extracted
        let value = json!({"role": "user", "text": "user input"});
        assert!(extract_text_from_value(&value).is_none());
    }

    #[test]
    fn extract_text_from_data_field() {
        let value = json!({"data": "some data content"});
        let text = extract_text_from_value(&value);
        assert_eq!(text.as_deref(), Some("some data content"));
    }

    #[test]
    fn extract_text_from_nested_content() {
        let value = json!({"role": "assistant", "content": {"text": "nested"}});
        let text = extract_text_from_value(&value);
        assert_eq!(text.as_deref(), Some("nested"));
    }
}
