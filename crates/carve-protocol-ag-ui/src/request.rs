use crate::protocol::MessageRole;
use carve_agent_contract::InteractionResponse;
use carve_thread_model::{gen_message_id, Message, Role, Visibility};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// AG-UI message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AGUIMessage {
    /// Message role (user, assistant, system, tool).
    pub role: MessageRole,
    /// Message content.
    pub content: String,
    /// Optional message ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Optional tool call ID (for tool messages).
    #[serde(rename = "toolCallId", skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl AGUIMessage {
    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            id: None,
            tool_call_id: None,
        }
    }

    /// Create a tool result message.
    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            id: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// AG-UI context entry from frontend readable values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AGUIContextEntry {
    /// Human-readable description of the context.
    pub description: String,
    /// The context value.
    pub value: Value,
}

/// Tool execution location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionLocation {
    /// Tool executes on the backend (server-side).
    Backend,
    /// Tool executes on the frontend (client-side).
    #[default]
    Frontend,
}

/// AG-UI tool definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AGUIToolDef {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON Schema for tool parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Where the tool executes (frontend or backend).
    #[serde(default, skip_serializing_if = "is_default_frontend")]
    pub execute: ToolExecutionLocation,
}

fn is_default_frontend(loc: &ToolExecutionLocation) -> bool {
    *loc == ToolExecutionLocation::Frontend
}

impl AGUIToolDef {
    /// Create a new backend tool definition.
    pub fn backend(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
            execute: ToolExecutionLocation::Backend,
        }
    }

    /// Create a new frontend tool definition.
    pub fn frontend(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: None,
            execute: ToolExecutionLocation::Frontend,
        }
    }

    /// Set the JSON Schema parameters.
    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Check if this is a frontend tool.
    pub fn is_frontend(&self) -> bool {
        self.execute == ToolExecutionLocation::Frontend
    }
}

/// Request to run an AG-UI agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunAgentRequest {
    /// Thread identifier.
    #[serde(rename = "threadId")]
    pub thread_id: String,
    /// Run identifier.
    #[serde(rename = "runId")]
    pub run_id: String,
    /// Conversation messages.
    pub messages: Vec<AGUIMessage>,
    /// Available tools.
    #[serde(default)]
    pub tools: Vec<AGUIToolDef>,
    /// Frontend readable context entries.
    #[serde(default)]
    pub context: Vec<AGUIContextEntry>,
    /// Initial state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
    /// Parent run ID (for sub-runs).
    #[serde(rename = "parentRunId", skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    /// Model to use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// System prompt.
    #[serde(rename = "systemPrompt", skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Additional configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

impl RunAgentRequest {
    /// Create a new request with minimal required fields.
    pub fn new(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            messages: Vec::new(),
            tools: Vec::new(),
            context: Vec::new(),
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        }
    }

    /// Add a message.
    pub fn with_message(mut self, message: AGUIMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Add messages.
    pub fn with_messages(mut self, messages: Vec<AGUIMessage>) -> Self {
        self.messages.extend(messages);
        self
    }

    /// Set initial state.
    pub fn with_state(mut self, state: Value) -> Self {
        self.state = Some(state);
        self
    }

    /// Set model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Validate the request.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.thread_id.is_empty() {
            return Err(RequestError::invalid_field("threadId cannot be empty"));
        }
        if self.run_id.is_empty() {
            return Err(RequestError::invalid_field("runId cannot be empty"));
        }
        Ok(())
    }

    /// Get frontend tools from the request.
    pub fn frontend_tools(&self) -> Vec<&AGUIToolDef> {
        self.tools.iter().filter(|t| t.is_frontend()).collect()
    }

    /// Check if any interaction responses exist in this request.
    pub fn has_any_interaction_responses(&self) -> bool {
        !self.interaction_responses().is_empty()
    }

    /// Extract all interaction responses from tool messages.
    pub fn interaction_responses(&self) -> Vec<InteractionResponse> {
        self.messages
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .filter_map(|m| {
                m.tool_call_id.as_ref().map(|id| {
                    let result = parse_interaction_result_value(&m.content);
                    InteractionResponse::new(id.clone(), result)
                })
            })
            .collect()
    }

    /// Get all approved interaction IDs.
    pub fn approved_interaction_ids(&self) -> Vec<String> {
        self.interaction_responses()
            .into_iter()
            .filter(|r| r.approved())
            .map(|r| r.interaction_id)
            .collect()
    }

    /// Get all denied interaction IDs.
    pub fn denied_interaction_ids(&self) -> Vec<String> {
        self.interaction_responses()
            .into_iter()
            .filter(|r| r.denied())
            .map(|r| r.interaction_id)
            .collect()
    }
}

fn parse_interaction_result_value(content: &str) -> Value {
    serde_json::from_str(content).unwrap_or_else(|_| Value::String(content.to_string()))
}

/// Convert AG-UI message to internal message.
pub fn core_message_from_ag_ui(msg: &AGUIMessage) -> Message {
    let role = match msg.role {
        MessageRole::System => Role::System,
        MessageRole::Developer => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Assistant => Role::Assistant,
        MessageRole::Tool => Role::Tool,
    };

    Message {
        id: Some(msg.id.clone().unwrap_or_else(gen_message_id)),
        role,
        content: msg.content.clone(),
        tool_calls: None,
        tool_call_id: msg.tool_call_id.clone(),
        visibility: Visibility::default(),
        metadata: None,
    }
}

/// Convert AG-UI messages to internal messages.
pub fn convert_agui_messages(messages: &[AGUIMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| m.role != MessageRole::Assistant)
        .map(core_message_from_ag_ui)
        .collect()
}

/// Error type for request processing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestError {
    /// Error code.
    pub code: String,
    /// Error message.
    pub message: String,
}

impl RequestError {
    /// Create an invalid field error.
    pub fn invalid_field(message: impl Into<String>) -> Self {
        Self {
            code: "INVALID_FIELD".into(),
            message: message.into(),
        }
    }

    /// Create a validation error.
    pub fn validation(message: impl Into<String>) -> Self {
        Self {
            code: "VALIDATION_ERROR".into(),
            message: message.into(),
        }
    }

    /// Create an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR".into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for RequestError {}

impl From<String> for RequestError {
    fn from(message: String) -> Self {
        Self::validation(message)
    }
}

/// Build a context string from AG-UI context entries to append to the system prompt.
pub fn build_context_addendum(request: &RunAgentRequest) -> Option<String> {
    if request.context.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for entry in &request.context {
        let value_str = match &entry.value {
            Value::String(s) => s.clone(),
            other => match serde_json::to_string(other) {
                Ok(value) => value,
                Err(err) => {
                    warn!(
                        error = %err,
                        description = %entry.description,
                        "failed to stringify AG-UI context value"
                    );
                    "<unserializable-context-value>".to_string()
                }
            },
        };
        parts.push(format!("[{}]: {}", entry.description, value_str));
    }
    Some(format!(
        "\n\nThe following context is available from the frontend:\n{}",
        parts.join("\n")
    ))
}
