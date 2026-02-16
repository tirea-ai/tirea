//! Tool trait for agent actions.
//!
//! Tools execute actions and can modify state through `AgentState`.

pub use crate::{ToolResult, ToolStatus};
use async_trait::async_trait;
use crate::AgentState;
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
/// use carve_agent::contracts::traits::tool::{Tool, ToolDescriptor, ToolResult};
/// use carve_agent::prelude::AgentState;
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
///         ctx: &AgentState<'_>,
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
    /// - `ctx`: AgentState for state access (framework extracts patch after execution)
    ///
    /// # Returns
    ///
    /// Tool result or error
    async fn execute(&self, args: Value, ctx: &AgentState<'_>) -> Result<ToolResult, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =============================================================================
    // ToolError tests
    // =============================================================================

    #[test]
    fn test_tool_error_invalid_arguments() {
        let err = ToolError::InvalidArguments("missing field".to_string());
        assert_eq!(err.to_string(), "Invalid arguments: missing field");
    }

    #[test]
    fn test_tool_error_execution_failed() {
        let err = ToolError::ExecutionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Execution failed: timeout");
    }

    #[test]
    fn test_tool_error_permission_denied() {
        let err = ToolError::PermissionDenied("no access".to_string());
        assert_eq!(err.to_string(), "Permission denied: no access");
    }

    #[test]
    fn test_tool_error_not_found() {
        let err = ToolError::NotFound("file.txt".to_string());
        assert_eq!(err.to_string(), "Not found: file.txt");
    }

    #[test]
    fn test_tool_error_internal() {
        let err = ToolError::Internal("unexpected".to_string());
        assert_eq!(err.to_string(), "Internal error: unexpected");
    }

    // =============================================================================
    // ToolStatus tests
    // =============================================================================

    #[test]
    fn test_tool_status_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolStatus::Success).unwrap(),
            "\"success\""
        );
        assert_eq!(
            serde_json::to_string(&ToolStatus::Warning).unwrap(),
            "\"warning\""
        );
        assert_eq!(
            serde_json::to_string(&ToolStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&ToolStatus::Error).unwrap(),
            "\"error\""
        );
    }

    #[test]
    fn test_tool_status_deserialization() {
        assert_eq!(
            serde_json::from_str::<ToolStatus>("\"success\"").unwrap(),
            ToolStatus::Success
        );
        assert_eq!(
            serde_json::from_str::<ToolStatus>("\"warning\"").unwrap(),
            ToolStatus::Warning
        );
        assert_eq!(
            serde_json::from_str::<ToolStatus>("\"pending\"").unwrap(),
            ToolStatus::Pending
        );
        assert_eq!(
            serde_json::from_str::<ToolStatus>("\"error\"").unwrap(),
            ToolStatus::Error
        );
    }

    #[test]
    fn test_tool_status_equality() {
        assert_eq!(ToolStatus::Success, ToolStatus::Success);
        assert_ne!(ToolStatus::Success, ToolStatus::Error);
    }

    #[test]
    fn test_tool_status_clone() {
        let status = ToolStatus::Warning;
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    #[test]
    fn test_tool_status_debug() {
        assert_eq!(format!("{:?}", ToolStatus::Success), "Success");
        assert_eq!(format!("{:?}", ToolStatus::Error), "Error");
    }

    // =============================================================================
    // ToolResult tests
    // =============================================================================

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("my_tool", json!({"value": 42}));
        assert_eq!(result.tool_name, "my_tool");
        assert_eq!(result.status, ToolStatus::Success);
        assert_eq!(result.data, json!({"value": 42}));
        assert!(result.message.is_none());
        assert!(result.metadata.is_empty());
        assert!(result.is_success());
        assert!(!result.is_error());
        assert!(!result.is_pending());
    }

    #[test]
    fn test_tool_result_success_with_message() {
        let result = ToolResult::success_with_message(
            "my_tool",
            json!({"done": true}),
            "Operation complete",
        );
        assert_eq!(result.tool_name, "my_tool");
        assert_eq!(result.status, ToolStatus::Success);
        assert_eq!(result.data, json!({"done": true}));
        assert_eq!(result.message, Some("Operation complete".to_string()));
        assert!(result.is_success());
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("my_tool", "Something went wrong");
        assert_eq!(result.tool_name, "my_tool");
        assert_eq!(result.status, ToolStatus::Error);
        assert_eq!(result.data, Value::Null);
        assert_eq!(result.message, Some("Something went wrong".to_string()));
        assert!(!result.is_success());
        assert!(result.is_error());
        assert!(!result.is_pending());
    }

    #[test]
    fn test_tool_result_pending() {
        let result = ToolResult::pending("my_tool", "Waiting for confirmation");
        assert_eq!(result.tool_name, "my_tool");
        assert_eq!(result.status, ToolStatus::Pending);
        assert_eq!(result.data, Value::Null);
        assert_eq!(result.message, Some("Waiting for confirmation".to_string()));
        assert!(!result.is_success());
        assert!(!result.is_error());
        assert!(result.is_pending());
    }

    #[test]
    fn test_tool_result_warning() {
        let result = ToolResult::warning("my_tool", json!({"partial": true}), "Some items skipped");
        assert_eq!(result.tool_name, "my_tool");
        assert_eq!(result.status, ToolStatus::Warning);
        assert_eq!(result.data, json!({"partial": true}));
        assert_eq!(result.message, Some("Some items skipped".to_string()));
        // Warning is considered success
        assert!(result.is_success());
        assert!(!result.is_error());
    }

    #[test]
    fn test_tool_result_with_metadata() {
        let result = ToolResult::success("my_tool", json!({}))
            .with_metadata("duration_ms", 150)
            .with_metadata("retry_count", 2);
        assert_eq!(result.metadata.get("duration_ms"), Some(&json!(150)));
        assert_eq!(result.metadata.get("retry_count"), Some(&json!(2)));
    }

    #[test]
    fn test_tool_result_serialization() {
        let result =
            ToolResult::success("my_tool", json!({"key": "value"})).with_metadata("extra", "data");

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool_name, "my_tool");
        assert_eq!(parsed.status, ToolStatus::Success);
        assert_eq!(parsed.data, json!({"key": "value"}));
    }

    #[test]
    fn test_tool_result_clone() {
        let result = ToolResult::success("my_tool", json!({"x": 1}));
        let cloned = result.clone();
        assert_eq!(result.tool_name, cloned.tool_name);
        assert_eq!(result.status, cloned.status);
    }

    #[test]
    fn test_tool_result_debug() {
        let result = ToolResult::success("test", json!(null));
        let debug = format!("{:?}", result);
        assert!(debug.contains("ToolResult"));
        assert!(debug.contains("test"));
    }

    // =============================================================================
    // ToolDescriptor tests
    // =============================================================================

    #[test]
    fn test_tool_descriptor_new() {
        let desc = ToolDescriptor::new("read_file", "Read File", "Reads a file from disk");
        assert_eq!(desc.id, "read_file");
        assert_eq!(desc.name, "Read File");
        assert_eq!(desc.description, "Reads a file from disk");
        assert!(!desc.requires_confirmation);
        assert!(desc.category.is_none());
        assert!(desc.metadata.is_empty());
        // Default parameters
        assert_eq!(desc.parameters, json!({"type": "object", "properties": {}}));
    }

    #[test]
    fn test_tool_descriptor_with_parameters() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let desc =
            ToolDescriptor::new("read_file", "Read File", "Read").with_parameters(schema.clone());
        assert_eq!(desc.parameters, schema);
    }

    #[test]
    fn test_tool_descriptor_with_confirmation() {
        let desc = ToolDescriptor::new("delete", "Delete", "Delete file").with_confirmation(true);
        assert!(desc.requires_confirmation);

        let desc2 = ToolDescriptor::new("read", "Read", "Read file").with_confirmation(false);
        assert!(!desc2.requires_confirmation);
    }

    #[test]
    fn test_tool_descriptor_with_category() {
        let desc =
            ToolDescriptor::new("read_file", "Read File", "Read").with_category("filesystem");
        assert_eq!(desc.category, Some("filesystem".to_string()));
    }

    #[test]
    fn test_tool_descriptor_with_metadata() {
        let desc = ToolDescriptor::new("my_tool", "My Tool", "Description")
            .with_metadata("version", "1.0")
            .with_metadata("author", "test");
        assert_eq!(desc.metadata.get("version"), Some(&json!("1.0")));
        assert_eq!(desc.metadata.get("author"), Some(&json!("test")));
    }

    #[test]
    fn test_tool_descriptor_builder_chain() {
        let desc = ToolDescriptor::new("tool", "Tool", "Desc")
            .with_parameters(json!({"type": "object"}))
            .with_confirmation(true)
            .with_category("test")
            .with_metadata("key", "value");

        assert_eq!(desc.id, "tool");
        assert!(desc.requires_confirmation);
        assert_eq!(desc.category, Some("test".to_string()));
        assert_eq!(desc.metadata.get("key"), Some(&json!("value")));
    }

    #[test]
    fn test_tool_descriptor_serialization() {
        let desc = ToolDescriptor::new("my_tool", "My Tool", "Does things")
            .with_category("utilities")
            .with_confirmation(true);

        let json = serde_json::to_string(&desc).unwrap();
        let parsed: ToolDescriptor = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "my_tool");
        assert_eq!(parsed.name, "My Tool");
        assert_eq!(parsed.category, Some("utilities".to_string()));
        assert!(parsed.requires_confirmation);
    }

    #[test]
    fn test_tool_descriptor_clone() {
        let desc = ToolDescriptor::new("tool", "Tool", "Desc").with_category("cat");
        let cloned = desc.clone();
        assert_eq!(desc.id, cloned.id);
        assert_eq!(desc.category, cloned.category);
    }

    #[test]
    fn test_tool_descriptor_debug() {
        let desc = ToolDescriptor::new("tool", "Tool", "Desc");
        let debug = format!("{:?}", desc);
        assert!(debug.contains("ToolDescriptor"));
        assert!(debug.contains("tool"));
    }
}
