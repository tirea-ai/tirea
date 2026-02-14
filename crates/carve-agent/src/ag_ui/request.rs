use crate::ag_ui::protocol::MessageRole;
pub use crate::interaction::InteractionPlugin;
#[cfg(test)]
use crate::interaction::{FrontendToolPlugin, InteractionResponsePlugin};
use crate::interaction::{FrontendToolRegistry, FrontendToolSpec};
use crate::r#loop::AgentConfig;
use crate::state_types::InteractionResponse;
use crate::thread::Thread;
use crate::traits::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

fn frontend_tool_specs_from_request(request: &RunAgentRequest) -> Vec<FrontendToolSpec> {
    request
        .frontend_tools()
        .iter()
        .map(|tool| FrontendToolSpec {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        })
        .collect()
}

pub(super) fn merge_frontend_tools(
    tools: &mut HashMap<String, std::sync::Arc<dyn crate::traits::tool::Tool>>,
    request: &RunAgentRequest,
) {
    let frontend_specs = frontend_tool_specs_from_request(request);
    FrontendToolRegistry::new(frontend_specs).install_stubs(tools);
}

impl InteractionPlugin {
    /// Build an interaction plugin from AG-UI request fields.
    pub fn from_request(request: &RunAgentRequest) -> Self {
        Self::new(
            request
                .frontend_tools()
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
            request.approved_interaction_ids(),
            request.denied_interaction_ids(),
        )
    }
}

#[cfg(test)]
impl FrontendToolPlugin {
    pub(super) fn from_request(request: &RunAgentRequest) -> Self {
        Self::new(
            request
                .frontend_tools()
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
        )
    }
}

#[cfg(test)]
impl InteractionResponsePlugin {
    pub(super) fn from_request(request: &RunAgentRequest) -> Self {
        Self::new(
            request.approved_interaction_ids(),
            request.denied_interaction_ids(),
        )
    }
}

// ============================================================================

// AG-UI Request Types
// ============================================================================

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
    /// The context value (serialized as JSON string).
    pub value: Value,
}

/// Tool execution location.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionLocation {
    /// Tool executes on the backend (server-side).
    Backend,
    /// Tool executes on the frontend (client-side).
    ///
    /// This is the default because tools sent in the AG-UI request body are
    /// frontend-registered (e.g. via CopilotKit `useCopilotAction`) and should
    /// be executed by the client, not the server.
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
    /// Frontend tools are executed by the client and results sent back.
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

    /// Get the last user message content.
    pub fn last_user_message(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.as_str())
    }

    /// Get frontend tools from the request.
    pub fn frontend_tools(&self) -> Vec<&AGUIToolDef> {
        self.tools.iter().filter(|t| t.is_frontend()).collect()
    }

    /// Get backend tools from the request.
    pub fn backend_tools(&self) -> Vec<&AGUIToolDef> {
        self.tools.iter().filter(|t| !t.is_frontend()).collect()
    }

    /// Check if a tool is a frontend tool by name.
    pub fn is_frontend_tool(&self, name: &str) -> bool {
        self.tools
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.is_frontend())
            .unwrap_or(false)
    }

    /// Add a tool definition.
    pub fn with_tool(mut self, tool: AGUIToolDef) -> Self {
        self.tools.push(tool);
        self
    }

    // ========================================================================
    // Interaction Response Methods
    // ========================================================================

    /// Extract all interaction responses from tool messages.
    ///
    /// Returns a list of interaction responses parsed from tool role messages.
    /// Each tool message with a `tool_call_id` is treated as a response to
    /// a pending interaction with that ID.
    pub fn interaction_responses(&self) -> Vec<InteractionResponse> {
        self.messages
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .filter_map(|m| {
                m.tool_call_id.as_ref().map(|id| {
                    // Try to parse content as JSON, fallback to string value
                    let result = serde_json::from_str(&m.content)
                        .unwrap_or_else(|_| Value::String(m.content.clone()));
                    InteractionResponse::new(id.clone(), result)
                })
            })
            .collect()
    }

    /// Get the response for a specific interaction ID.
    pub fn get_interaction_response(&self, interaction_id: &str) -> Option<InteractionResponse> {
        self.interaction_responses()
            .into_iter()
            .find(|r| r.interaction_id == interaction_id)
    }

    /// Check if a specific interaction was approved.
    ///
    /// An interaction is considered approved if:
    /// - The result is boolean `true`
    /// - The result is string "true", "yes", "approved", "allow", "confirm" (case-insensitive)
    /// - The result is an object with `approved: true` or `allowed: true`
    pub fn is_interaction_approved(&self, interaction_id: &str) -> bool {
        self.get_interaction_response(interaction_id)
            .map(|r| InteractionResponse::is_approved(&r.result))
            .unwrap_or(false)
    }

    /// Check if a specific interaction was denied.
    ///
    /// An interaction is considered denied if:
    /// - The result is boolean `false`
    /// - The result is string "false", "no", "denied", "deny", "reject", "cancel" (case-insensitive)
    /// - The result is an object with `approved: false` or `denied: true`
    pub fn is_interaction_denied(&self, interaction_id: &str) -> bool {
        self.get_interaction_response(interaction_id)
            .map(|r| InteractionResponse::is_denied(&r.result))
            .unwrap_or(false)
    }

    /// Check if the request contains a response for a pending interaction.
    pub fn has_interaction_response(&self, interaction_id: &str) -> bool {
        self.get_interaction_response(interaction_id).is_some()
    }

    /// Check if any interaction responses exist in this request.
    pub fn has_any_interaction_responses(&self) -> bool {
        !self.interaction_responses().is_empty()
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

pub(crate) fn core_message_from_ag_ui(msg: &AGUIMessage) -> crate::types::Message {
    use crate::types::{gen_message_id, Message, Role};

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
        visibility: crate::types::Visibility::default(),
        metadata: None,
    }
}

pub(super) fn should_seed_session_from_request(thread: &Thread, request: &RunAgentRequest) -> bool {
    let session_state_is_empty_object = thread.state.as_object().is_some_and(|m| m.is_empty());

    let request_has_state = request.state.as_ref().is_some_and(|s| !s.is_null());
    let request_has_messages = !request.messages.is_empty();

    thread.messages.is_empty()
        && thread.patches.is_empty()
        && session_state_is_empty_object
        && (request_has_state || request_has_messages)
}

pub(super) fn seed_session_from_request(thread: Thread, request: &RunAgentRequest) -> Thread {
    // For AG-UI, thread_id is the session identity; if caller didn't provide
    // a session history/state, we seed it from the request payload.
    let state = request
        .state
        .clone()
        .unwrap_or_else(|| thread.state.clone());
    let messages = request
        .messages
        .iter()
        .map(core_message_from_ag_ui)
        .collect::<Vec<_>>();

    Thread::with_initial_state(request.thread_id.clone(), state).with_messages(messages)
}

pub(super) fn session_has_message_id(thread: &Thread, id: &str) -> bool {
    thread
        .messages
        .iter()
        .any(|m| m.id.as_deref().is_some_and(|mid| mid == id))
}

pub(super) fn session_has_tool_call_id(thread: &Thread, tool_call_id: &str) -> bool {
    thread.messages.iter().any(|m| {
        m.tool_call_id
            .as_deref()
            .is_some_and(|tid| tid == tool_call_id)
    })
}

/// Apply request messages/state into an existing session.
///
/// - If the session is empty and the request carries messages/state, we seed the session.
/// - Otherwise we import messages with the following rules:
///   - tool role messages with `toolCallId`
///   - any message with a stable `id`
///   - id-less non-tool messages are appended as-is (same-run duplicates are valid user input)
pub(crate) fn apply_agui_request_to_thread(thread: Thread, request: &RunAgentRequest) -> Thread {
    if should_seed_session_from_request(&thread, request) {
        return seed_session_from_request(thread, request);
    }

    let mut new_msgs: Vec<crate::types::Message> = Vec::new();

    for msg in &request.messages {
        // Skip assistant messages — our backend generates these during the agent
        // loop with its own message IDs.  AG-UI clients echo them back with
        // client-assigned IDs (e.g. from TEXT_MESSAGE_START.messageId) that
        // differ from the session's internal IDs, causing duplicates that
        // corrupt the conversation history sent to the LLM.
        if msg.role == MessageRole::Assistant {
            continue;
        }

        // For tool messages, dedup by tool_call_id FIRST.  CopilotKit echoes
        // back tool results with client-assigned IDs (e.g. "result_call_00_...")
        // that differ from the backend's internal IDs.  If we only checked by
        // message id, duplicates would slip through and appear after the final
        // assistant text, causing "tool must follow tool_calls" LLM errors.
        if msg.role == MessageRole::Tool {
            if let Some(tool_call_id) = msg.tool_call_id.as_deref() {
                if !session_has_tool_call_id(&thread, tool_call_id) {
                    new_msgs.push(core_message_from_ag_ui(msg));
                }
            }
            continue;
        }

        if let Some(id) = msg.id.as_deref() {
            if !session_has_message_id(&thread, id) {
                new_msgs.push(core_message_from_ag_ui(msg));
            }
            continue;
        }

        // Some clients don't provide stable IDs for user/system turns.
        // Do not deduplicate these by content: repeated messages in the same run
        // are valid user input and must be preserved.
        new_msgs.push(core_message_from_ag_ui(msg));
    }

    if new_msgs.is_empty() {
        thread
    } else {
        thread.with_messages(new_msgs)
    }
}

/// Runtime key used to mark that a specific AG-UI request has already been
/// applied to a thread, preventing duplicate ingress on re-entry.
pub(crate) const AGUI_REQUEST_APPLIED_RUNTIME_KEY: &str = "__agui_request_applied";

pub(super) fn ensure_agui_request_applied(mut thread: Thread, request: &RunAgentRequest) -> Thread {
    let already_applied = thread
        .runtime
        .value(AGUI_REQUEST_APPLIED_RUNTIME_KEY)
        .and_then(|v| v.as_str())
        .is_some_and(|run_id| run_id == request.run_id);
    if !already_applied {
        thread = apply_agui_request_to_thread(thread, request);
    }
    let _ = thread
        .runtime
        .set(AGUI_REQUEST_APPLIED_RUNTIME_KEY, request.run_id.clone());
    thread
}

impl InteractionResponse {
    /// Check if a result value indicates approval.
    pub fn is_approved(result: &Value) -> bool {
        match result {
            Value::Bool(b) => *b,
            Value::String(s) => {
                let lower = s.to_lowercase();
                matches!(
                    lower.as_str(),
                    "true" | "yes" | "approved" | "allow" | "confirm" | "ok" | "accept"
                )
            }
            Value::Object(obj) => {
                obj.get("approved")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || obj
                        .get("allowed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Check if a result value indicates denial.
    pub fn is_denied(result: &Value) -> bool {
        match result {
            Value::Bool(b) => !*b,
            Value::String(s) => {
                let lower = s.to_lowercase();
                matches!(
                    lower.as_str(),
                    "false" | "no" | "denied" | "deny" | "reject" | "cancel" | "abort"
                )
            }
            Value::Object(obj) => {
                obj.get("approved")
                    .and_then(|v| v.as_bool())
                    .map(|v| !v)
                    .unwrap_or(false)
                    || obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Check if this response indicates approval.
    pub fn approved(&self) -> bool {
        Self::is_approved(&self.result)
    }

    /// Check if this response indicates denial.
    pub fn denied(&self) -> bool {
        Self::is_denied(&self.result)
    }
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
pub(super) fn build_context_addendum(request: &RunAgentRequest) -> Option<String> {
    if request.context.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for entry in &request.context {
        let value_str = match &entry.value {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        parts.push(format!("[{}]: {}", entry.description, value_str));
    }
    Some(format!(
        "\n\nThe following context is available from the frontend:\n{}",
        parts.join("\n")
    ))
}

struct RequestWiring {
    install_interaction: bool,
}

impl RequestWiring {
    fn from_request(request: &RunAgentRequest) -> Self {
        Self {
            install_interaction: request.tools.iter().any(|t| t.is_frontend())
                || request.has_any_interaction_responses(),
        }
    }

    fn apply(
        self,
        mut config: AgentConfig,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
        request: &RunAgentRequest,
    ) -> AgentConfig {
        if request.frontend_tools().is_empty() {
            // no-op
        } else {
            merge_frontend_tools(tools, request);
        }

        if self.install_interaction && !config.plugins.iter().any(|p| p.id() == "interaction") {
            config = config.with_plugin(Arc::new(InteractionPlugin::default()));
        }
        config
    }
}

fn apply_request_overrides(mut config: AgentConfig, request: &RunAgentRequest) -> AgentConfig {
    if let Some(model) = request.model.as_ref() {
        config.model = model.clone();
    }
    if let Some(prompt) = request.system_prompt.as_ref() {
        config.system_prompt = prompt.clone();
    }

    if let Some(addendum) = build_context_addendum(request) {
        config.system_prompt.push_str(&addendum);
    }

    config
}

pub(super) fn set_run_identity(thread: &mut Thread, run_id: &str, parent_run_id: Option<&str>) {
    let _ = thread.runtime.set("run_id", run_id.to_string());
    if let Some(parent) = parent_run_id {
        let _ = thread.runtime.set("parent_run_id", parent.to_string());
    }
}

fn set_interaction_request_context(thread: &mut Thread, request: &RunAgentRequest) {
    let frontend_tools: Vec<String> = request
        .frontend_tools()
        .into_iter()
        .map(|tool| tool.name.clone())
        .collect();
    if !frontend_tools.is_empty() {
        let _ = thread.runtime.set(
            crate::interaction::RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY,
            frontend_tools,
        );
    }

    let responses = request.interaction_responses();
    if !responses.is_empty() {
        let _ = thread.runtime.set(
            crate::interaction::RUNTIME_INTERACTION_RESPONSES_KEY,
            responses,
        );
    }
}

pub(super) fn prepare_request_runtime(
    config: AgentConfig,
    thread: Thread,
    mut tools: HashMap<String, Arc<dyn Tool>>,
    request: &RunAgentRequest,
) -> (AgentConfig, Thread, HashMap<String, Arc<dyn Tool>>) {
    let mut thread = ensure_agui_request_applied(thread, request);
    set_interaction_request_context(&mut thread, request);
    let config = apply_request_overrides(config, request);
    let config = RequestWiring::from_request(request).apply(config, &mut tools, request);
    (config, thread, tools)
}

// ============================================================================
