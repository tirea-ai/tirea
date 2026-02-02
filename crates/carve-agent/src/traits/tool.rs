//! Tool trait for agent actions.
//!
//! Tools execute actions and can modify state through the Context.

use carve_state::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

/// Tool execution errors.
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Tool execution status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// Execution succeeded.
    Success,
    /// Execution succeeded with warnings.
    Warning,
    /// Execution is pending (waiting for user interaction).
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
        }
    }

    /// Create a pending result (waiting for interaction).
    pub fn pending(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            status: ToolStatus::Pending,
            data: Value::Null,
            message: Some(message.into()),
            metadata: HashMap::new(),
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
        }
    }

    /// Add metadata.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.metadata.insert(key.into(), value.into());
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
    /// Whether tool requires confirmation.
    pub requires_confirmation: bool,
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
            requires_confirmation: false,
            category: None,
            metadata: HashMap::new(),
        }
    }

    /// Set parameters schema.
    pub fn with_parameters(mut self, schema: Value) -> Self {
        self.parameters = schema;
        self
    }

    /// Set requires confirmation.
    pub fn with_confirmation(mut self, requires: bool) -> Self {
        self.requires_confirmation = requires;
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
/// use carve_agent::{Tool, ToolDescriptor, ToolResult, Context};
/// use carve_state_derive::State;
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
///         ctx: &Context<'_>,
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

    /// Execute the tool.
    ///
    /// # Arguments
    ///
    /// - `args`: Tool arguments as JSON value
    /// - `ctx`: Context for state access (reference - framework extracts patch after execution)
    ///
    /// # Returns
    ///
    /// Tool result or error
    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError>;
}
