//! Tool trait for agent actions.
//!
//! Tools execute actions and can modify state through `Thread`.

use super::ToolCallContext;
use crate::runtime::plugin::phase::SuspendTicket;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Tool execution status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Execution succeeded.
    Success,
    /// Execution succeeded with warnings.
    Warning,
    /// Execution is pending (waiting for suspension resolution).
    Pending,
    /// Execution failed.
    Error,
}

/// Result of tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool name.
    pub tool_name: String,
    /// Execution status.
    pub status: ToolStatus,
    /// Result data.
    pub data: Value,
    /// Optional message.
    pub message: Option<String>,
    /// Metadata.
    pub metadata: HashMap<String, Value>,
    /// Structured suspension payload for loop-level suspension handling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspension: Option<SuspendTicket>,
}

impl ToolResult {
    /// Create a success result.
    pub fn success(tool_name: impl Into<String>, data: impl Into<Value>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Success,
            data: data.into(),
            message: None,
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Create a success result with message.
    pub fn success_with_message(
        tool_name: impl Into<String>,
        data: impl Into<Value>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Success,
            data: data.into(),
            message: Some(message.into()),
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Create an error result.
    pub fn error(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Error,
            data: Value::Null,
            message: Some(message.into()),
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Create a structured error result with stable error code payload.
    pub fn error_with_code(
        tool_name: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let tool_name = tool_name.into();
        let code = code.into();
        let message = message.into();
        Self {
            tool_name,
            status: ToolStatus::Error,
            data: serde_json::json!({
                "error": {
                    "code": code,
                    "message": message,
                }
            }),
            message: Some(format!("[{code}] {message}")),
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Create a suspended result (waiting for external resume/decision).
    pub fn suspended(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Pending,
            data: Value::Null,
            message: Some(message.into()),
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Create a suspended result carrying an explicit suspension envelope.
    pub fn suspended_with(
        tool_name: impl Into<String>,
        message: impl Into<String>,
        ticket: SuspendTicket,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Pending,
            data: Value::Null,
            message: Some(message.into()),
            metadata: HashMap::new(),
            suspension: Some(ticket),
        }
    }

    /// Create a warning result.
    pub fn warning(
        tool_name: impl Into<String>,
        data: impl Into<Value>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Warning,
            data: data.into(),
            message: Some(message.into()),
            metadata: HashMap::new(),
            suspension: None,
        }
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Attach structured suspension payload for loop-level suspension handling.
    pub fn with_suspension(mut self, ticket: SuspendTicket) -> Self {
        self.suspension = Some(ticket);
        self
    }

    /// Check if execution succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self.status, ToolStatus::Success | ToolStatus::Warning)
    }

    /// Check if execution is pending.
    pub fn is_pending(&self) -> bool {
        matches!(self.status, ToolStatus::Pending)
    }

    /// Check if execution failed.
    pub fn is_error(&self) -> bool {
        matches!(self.status, ToolStatus::Error)
    }

    /// Structured suspension payload attached by `with_suspension`.
    pub fn suspension(&self) -> Option<SuspendTicket> {
        self.suspension.clone()
    }

    /// Convert to JSON value for serialization.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Tool execution errors.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Denied: {0}")]
    Denied(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Tool descriptor containing metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Unique tool ID.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON schema for parameters.
    pub parameters: Value,
    /// Tool category.
    pub category: Option<String>,
    /// Additional metadata.
    pub metadata: HashMap<String, Value>,
}

impl ToolDescriptor {
    /// Create a new tool descriptor.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
            category: None,
            metadata: HashMap::new(),
        }
    }

    /// Set parameters schema.
    pub fn with_parameters(mut self, schema: Value) -> Self {
        self.parameters = schema;
        self
    }

    /// Set category.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// Tool trait for implementing agent tools.
///
/// # Example
///
/// ```ignore
/// use tirea::contracts::runtime::tool_call::{Tool, ToolDescriptor, ToolResult};
/// use tirea::contracts::ToolCallContext;
/// use tirea_state::State;
///
/// #[derive(State)]
/// struct MyToolState {
///     pub count: i64,
/// }
///
/// struct CounterTool;
///
/// #[async_trait]
/// impl Tool for CounterTool {
///     fn descriptor(&self) -> ToolDescriptor {
///         ToolDescriptor::new("counter", "Counter", "Increment a counter")
///     }
///
///     async fn execute(
///         &self,
///         args: Value,
///         ctx: &ToolCallContext<'_>,
///     ) -> Result<ToolResult, ToolError> {
///         let state = ctx.call_state::<MyToolState>();
///         let current = state.count().unwrap_or(0);
///         state.set_count(current + 1);
///
///         // No need to call finish() - changes are auto-collected
///         Ok(ToolResult::success("counter", json!({"count": current + 1})))
///     }
/// }
/// ```
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool descriptor.
    fn descriptor(&self) -> ToolDescriptor;

    /// Validate tool arguments against the descriptor's JSON Schema before execution.
    ///
    /// The default implementation uses [`validate_against_schema`] with
    /// `descriptor().parameters`. Override to customise or skip validation.
    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        validate_against_schema(&self.descriptor().parameters, args)
    }

    /// Execute the tool.
    ///
    /// # Arguments
    ///
    /// - `args`: Tool arguments as JSON value
    /// - `ctx`: Execution context for state access (framework extracts patch after execution).
    ///   `ctx.idempotency_key()` is the current `tool_call_id`.
    ///   Tools should use it as the idempotency key for side effects.
    ///
    /// # Returns
    ///
    /// Tool result or error
    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError>;
}

/// Validate a JSON value against a JSON Schema.
///
/// Returns `Ok(())` if the value conforms to the schema, or
/// `Err(ToolError::InvalidArguments)` with a description of all violations.
pub fn validate_against_schema(schema: &Value, args: &Value) -> Result<(), ToolError> {
    let validator = jsonschema::Validator::new(schema)
        .map_err(|e| ToolError::Internal(format!("invalid tool schema: {e}")))?;
    if validator.is_valid(args) {
        return Ok(());
    }
    let errors: Vec<String> = validator.iter_errors(args).map(|e| e.to_string()).collect();
    Err(ToolError::InvalidArguments(errors.join("; ")))
}

// ---------------------------------------------------------------------------
// TypedTool – strongly-typed tool with automatic schema generation
// ---------------------------------------------------------------------------

/// Strongly-typed variant of [`Tool`] with automatic JSON Schema generation.
///
/// Implement this trait instead of [`Tool`] when your tool has a fixed
/// parameter shape. A blanket impl provides [`Tool`] automatically.
///
/// # Example
///
/// ```ignore
/// use serde::Deserialize;
/// use schemars::JsonSchema;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct GreetArgs {
///     name: String,
/// }
///
/// struct GreetTool;
///
/// #[async_trait]
/// impl TypedTool for GreetTool {
///     type Args = GreetArgs;
///     fn tool_id(&self) -> &str { "greet" }
///     fn name(&self) -> &str { "Greet" }
///     fn description(&self) -> &str { "Greet a user" }
///
///     async fn execute(&self, args: GreetArgs, _ctx: &ToolCallContext<'_>)
///         -> Result<ToolResult, ToolError>
///     {
///         Ok(ToolResult::success("greet", json!({"greeting": format!("Hello, {}!", args.name)})))
///     }
/// }
/// ```
#[async_trait]
pub trait TypedTool: Send + Sync {
    /// Argument type — must derive `Deserialize` and `JsonSchema`.
    type Args: for<'de> Deserialize<'de> + JsonSchema + Send;

    /// Unique tool id (snake_case).
    fn tool_id(&self) -> &str;

    /// Human-readable tool name.
    fn name(&self) -> &str;

    /// Tool description shown to the LLM.
    fn description(&self) -> &str;

    /// Optional business-logic validation after deserialization.
    ///
    /// Return `Err(message)` to reject with [`ToolError::InvalidArguments`].
    fn validate(&self, _args: &Self::Args) -> Result<(), String> {
        Ok(())
    }

    /// Execute with typed arguments.
    async fn execute(
        &self,
        args: Self::Args,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError>;
}

#[async_trait]
impl<T: TypedTool> Tool for T {
    fn descriptor(&self) -> ToolDescriptor {
        let schema = typed_tool_schema::<T::Args>();
        ToolDescriptor::new(self.tool_id(), self.name(), self.description()).with_parameters(schema)
    }

    /// Skips JSON Schema validation — `from_value` deserialization covers it.
    fn validate_args(&self, _args: &Value) -> Result<(), ToolError> {
        Ok(())
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let typed: T::Args =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        self.validate(&typed).map_err(ToolError::InvalidArguments)?;
        TypedTool::execute(self, typed, ctx).await
    }
}

/// Generate a JSON Schema `Value` from a type implementing `JsonSchema`.
fn typed_tool_schema<T: JsonSchema>() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(T))
        .unwrap_or_else(|_| serde_json::json!({"type": "object", "properties": {}}));
    // Strip the $schema key — LLM providers don't need it.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
    }
    v
}

#[cfg(test)]
mod contract_tests;
