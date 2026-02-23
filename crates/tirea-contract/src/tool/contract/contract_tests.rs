
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
fn test_tool_error_denied() {
    let err = ToolError::Denied("no access".to_string());
    assert_eq!(err.to_string(), "Denied: no access");
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
    let result =
        ToolResult::success_with_message("my_tool", json!({"done": true}), "Operation complete");
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
fn test_tool_result_error_with_code() {
    let result = ToolResult::error_with_code("my_tool", "invalid_arguments", "missing input");
    assert_eq!(result.tool_name, "my_tool");
    assert_eq!(result.status, ToolStatus::Error);
    assert_eq!(
        result.data,
        json!({
            "error": {
                "code": "invalid_arguments",
                "message": "missing input"
            }
        })
    );
    assert_eq!(
        result.message,
        Some("[invalid_arguments] missing input".to_string())
    );
    assert!(result.is_error());
}

#[test]
fn test_tool_result_pending() {
    let result = ToolResult::suspended("my_tool", "Waiting for confirmation");
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
fn test_tool_descriptor_with_category() {
    let desc = ToolDescriptor::new("read_file", "Read File", "Read").with_category("filesystem");
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
        .with_category("test")
        .with_metadata("key", "value");

    assert_eq!(desc.id, "tool");
    assert_eq!(desc.category, Some("test".to_string()));
    assert_eq!(desc.metadata.get("key"), Some(&json!("value")));
}

#[test]
fn test_tool_descriptor_serialization() {
    let desc = ToolDescriptor::new("my_tool", "My Tool", "Does things").with_category("utilities");

    let json = serde_json::to_string(&desc).unwrap();
    let parsed: ToolDescriptor = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.id, "my_tool");
    assert_eq!(parsed.name, "My Tool");
    assert_eq!(parsed.category, Some("utilities".to_string()));
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

// =============================================================================
// validate_against_schema tests
// =============================================================================

#[test]
fn test_validate_against_schema_valid() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "required": ["name"]
    });
    assert!(validate_against_schema(&schema, &json!({"name": "Alice"})).is_ok());
}

#[test]
fn test_validate_against_schema_missing_required() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "required": ["name"]
    });
    let err = validate_against_schema(&schema, &json!({})).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[test]
fn test_validate_against_schema_wrong_type() {
    let schema = json!({
        "type": "object",
        "properties": {
            "count": { "type": "integer" }
        },
        "required": ["count"]
    });
    let err = validate_against_schema(&schema, &json!({"count": "not_a_number"})).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[test]
fn test_validate_against_schema_empty_schema_accepts_object() {
    let schema = json!({"type": "object", "properties": {}});
    assert!(validate_against_schema(&schema, &json!({"anything": true})).is_ok());
}

#[test]
fn test_validate_against_schema_multiple_errors_joined() {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age":  { "type": "integer" }
        },
        "required": ["name", "age"]
    });
    let err = validate_against_schema(&schema, &json!({})).unwrap_err();
    let msg = err.to_string();
    // Both missing-field errors should be present, joined by "; "
    assert!(
        msg.contains("; "),
        "expected multiple errors joined by '; ', got: {msg}"
    );
    assert!(msg.contains("name"), "expected 'name' in error: {msg}");
    assert!(msg.contains("age"), "expected 'age' in error: {msg}");
}

#[test]
fn test_validate_against_schema_null_args_rejected() {
    let schema = json!({"type": "object", "properties": {}});
    let err = validate_against_schema(&schema, &json!(null)).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[test]
fn test_validate_against_schema_invalid_schema_returns_internal() {
    // "type" must be a string — passing an integer makes the schema itself invalid.
    let bad_schema = json!({"type": 123});
    let err = validate_against_schema(&bad_schema, &json!({})).unwrap_err();
    assert!(
        matches!(err, ToolError::Internal(_)),
        "expected Internal error for invalid schema, got: {err}"
    );
}

#[test]
fn test_validate_against_schema_nested_object() {
    let schema = json!({
        "type": "object",
        "properties": {
            "address": {
                "type": "object",
                "properties": {
                    "city": { "type": "string" }
                },
                "required": ["city"]
            }
        },
        "required": ["address"]
    });
    // Valid nested
    assert!(validate_against_schema(&schema, &json!({"address": {"city": "Berlin"}})).is_ok());
    // Missing nested required field
    let err = validate_against_schema(&schema, &json!({"address": {}})).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
    // Wrong nested type
    let err = validate_against_schema(&schema, &json!({"address": {"city": 42}})).unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

// =============================================================================
// TypedTool tests
// =============================================================================

#[derive(Deserialize, JsonSchema)]
struct GreetArgs {
    name: String,
}

struct GreetTool;

#[async_trait]
impl TypedTool for GreetTool {
    type Args = GreetArgs;
    fn tool_id(&self) -> &str {
        "greet"
    }
    fn name(&self) -> &str {
        "Greet"
    }
    fn description(&self) -> &str {
        "Greet a user"
    }

    async fn execute(
        &self,
        args: GreetArgs,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(
            "greet",
            json!({"greeting": format!("Hello, {}!", args.name)}),
        ))
    }
}

#[test]
fn test_typed_tool_descriptor_schema() {
    let tool = GreetTool;
    let desc = Tool::descriptor(&tool);
    assert_eq!(desc.id, "greet");
    assert_eq!(desc.name, "Greet");
    assert_eq!(desc.description, "Greet a user");

    let props = desc.parameters.get("properties").unwrap();
    assert!(props.get("name").is_some());
    let required = desc.parameters.get("required").unwrap().as_array().unwrap();
    assert!(required.iter().any(|v| v == "name"));
    // No $schema key
    assert!(desc.parameters.get("$schema").is_none());
}

#[tokio::test]
async fn test_typed_tool_execute_success() {
    let tool = GreetTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    let result = Tool::execute(&tool, json!({"name": "World"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_success());
    assert_eq!(result.data["greeting"], "Hello, World!");
}

#[tokio::test]
async fn test_typed_tool_execute_deser_failure() {
    let tool = GreetTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    let err = Tool::execute(&tool, json!({"name": 123}), &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}

#[derive(Deserialize, JsonSchema)]
struct PositiveArgs {
    value: i64,
}

struct PositiveTool;

#[async_trait]
impl TypedTool for PositiveTool {
    type Args = PositiveArgs;
    fn tool_id(&self) -> &str {
        "positive"
    }
    fn name(&self) -> &str {
        "Positive"
    }
    fn description(&self) -> &str {
        "Requires positive value"
    }

    fn validate(&self, args: &PositiveArgs) -> Result<(), String> {
        if args.value <= 0 {
            return Err("value must be positive".into());
        }
        Ok(())
    }

    async fn execute(
        &self,
        args: PositiveArgs,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(
            "positive",
            json!({"value": args.value}),
        ))
    }
}

#[tokio::test]
async fn test_typed_tool_validate_rejection() {
    let tool = PositiveTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    let err = Tool::execute(&tool, json!({"value": -1}), &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
    assert!(err.to_string().contains("positive"));
}

#[test]
fn test_typed_tool_as_arc_dyn_tool() {
    let tool: std::sync::Arc<dyn Tool> = std::sync::Arc::new(GreetTool);
    let desc = tool.descriptor();
    assert_eq!(desc.id, "greet");
}

#[test]
fn test_typed_tool_skips_schema_validation() {
    let tool = GreetTool;
    // validate_args should always return Ok for TypedTool
    assert!(tool.validate_args(&json!({})).is_ok());
    assert!(tool.validate_args(&json!({"wrong": 123})).is_ok());
    assert!(tool.validate_args(&json!(null)).is_ok());
}

// -- TypedTool edge cases --------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct OptionalArgs {
    required_field: String,
    optional_field: Option<i64>,
}

struct OptionalTool;

#[async_trait]
impl TypedTool for OptionalTool {
    type Args = OptionalArgs;
    fn tool_id(&self) -> &str {
        "optional"
    }
    fn name(&self) -> &str {
        "Optional"
    }
    fn description(&self) -> &str {
        "Tool with optional field"
    }

    async fn execute(
        &self,
        args: OptionalArgs,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::success(
            "optional",
            json!({
                "required": args.required_field,
                "optional": args.optional_field,
            }),
        ))
    }
}

#[tokio::test]
async fn test_typed_tool_optional_field_absent() {
    let tool = OptionalTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    let result = Tool::execute(&tool, json!({"required_field": "hi"}), &ctx)
        .await
        .unwrap();
    assert!(result.is_success());
    assert_eq!(result.data["optional"], json!(null));
}

#[tokio::test]
async fn test_typed_tool_extra_fields_ignored() {
    let tool = GreetTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    // serde ignores unknown fields by default
    let result = Tool::execute(&tool, json!({"name": "World", "extra": 999}), &ctx)
        .await
        .unwrap();
    assert!(result.is_success());
    assert_eq!(result.data["greeting"], "Hello, World!");
}

#[tokio::test]
async fn test_typed_tool_empty_json_all_required() {
    let tool = GreetTool;
    let fixture = crate::testing::TestFixture::new();
    let ctx = fixture.ctx_with("call_1", "test");
    let err = Tool::execute(&tool, json!({}), &ctx).await.unwrap_err();
    assert!(matches!(err, ToolError::InvalidArguments(_)));
}
