//! Integration tests for carve-agent.
//!
//! These tests verify the Tool, ContextProvider, and SystemReminder traits
//! work correctly with the new State/Context API.

use async_trait::async_trait;
use carve_agent::{
    Context, ContextCategory, ContextProvider, StateManager, SystemReminder, Tool, ToolDescriptor,
    ToolError, ToolResult,
};
use carve_state_derive::State;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ============================================================================
// Test state types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State, Default)]
struct CounterState {
    value: i64,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State, Default)]
struct TodoState {
    items: Vec<String>,
    count: i64,
}

// ============================================================================
// Test tools
// ============================================================================

struct IncrementTool;

#[async_trait]
impl Tool for IncrementTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("increment", "Increment Counter", "Increments a counter by 1")
            .with_parameters(json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Counter path"}
                },
                "required": ["path"]
            }))
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("path is required".to_string()))?;

        let counter = ctx.state::<CounterState>(path);
        let current = counter.value().unwrap_or(0);
        counter.set_value(current + 1);

        Ok(ToolResult::success(
            "increment",
            json!({"new_value": current + 1}),
        ))
    }
}

struct AddTodoTool;

#[async_trait]
impl Tool for AddTodoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("add_todo", "Add Todo", "Adds a new todo item")
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let item = args["item"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("item is required".to_string()))?;

        let todos = ctx.state::<TodoState>("todos");
        let current_count = todos.count().unwrap_or(0);
        todos.items_push(item);
        todos.set_count(current_count + 1);

        Ok(ToolResult::success("add_todo", json!({"count": current_count + 1})))
    }
}

struct UpdateCallStateTool;

#[async_trait]
impl Tool for UpdateCallStateTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("update_call", "Update Call State", "Updates the call state")
    }

    async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let label = args["label"]
            .as_str()
            .unwrap_or("updated");

        // Use call_state() for per-call state
        let call_state = ctx.call_state::<CounterState>();
        let step = call_state.value().unwrap_or(0);
        call_state.set_value(step + 1);
        call_state.set_label(label);

        Ok(ToolResult::success(
            "update_call",
            json!({"step": step + 1}),
        ))
    }
}

// ============================================================================
// Test context providers
// ============================================================================

struct CounterContextProvider;

#[async_trait]
impl ContextProvider for CounterContextProvider {
    fn id(&self) -> &str {
        "counter_context"
    }

    fn category(&self) -> ContextCategory {
        ContextCategory::ToolExecution
    }

    fn priority(&self) -> u32 {
        100
    }

    async fn provide(&self, ctx: &Context<'_>) -> Vec<String> {
        let counter = ctx.state::<CounterState>("counter");

        // Provider can also modify state
        counter.set_label("context_provided");

        if let Ok(value) = counter.value() {
            if value > 10 {
                vec![format!("Counter is high: {}", value)]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }
}

// ============================================================================
// Test system reminders
// ============================================================================

struct TodoReminder;

#[async_trait]
impl SystemReminder for TodoReminder {
    fn id(&self) -> &str {
        "todo_reminder"
    }

    async fn remind(&self, ctx: &Context<'_>) -> Option<String> {
        let todos = ctx.state::<TodoState>("todos");

        let count = todos.count().unwrap_or(0);
        if count > 0 {
            Some(format!("You have {} pending todos", count))
        } else {
            None
        }
    }
}

// ============================================================================
// Tool execution tests
// ============================================================================

#[tokio::test]
async fn test_tool_basic_execution() {
    let manager = StateManager::new(json!({
        "counter": {"value": 0, "label": "test"}
    }));

    let tool = IncrementTool;
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "tool:increment");

    let result = tool
        .execute(json!({"path": "counter"}), &ctx)
        .await
        .unwrap();

    assert!(result.is_success());
    assert_eq!(result.data["new_value"], 1);

    // Apply patch
    let patch = ctx.take_patch();
    manager.apply(patch).await.unwrap();

    let new_state = manager.snapshot().await;
    assert_eq!(new_state["counter"]["value"], 1);
}

#[tokio::test]
async fn test_tool_multiple_executions() {
    let manager = StateManager::new(json!({
        "counter": {"value": 0, "label": "test"}
    }));

    let tool = IncrementTool;

    for i in 1..=5 {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, format!("call_{}", i), "tool:increment");

        let result = tool
            .execute(json!({"path": "counter"}), &ctx)
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["new_value"], i);

        manager.apply(ctx.take_patch()).await.unwrap();
    }

    let final_state = manager.snapshot().await;
    assert_eq!(final_state["counter"]["value"], 5);
}

#[tokio::test]
async fn test_tool_with_call_state() {
    let manager = StateManager::new(json!({
        "tool_calls": {
            "call_abc": {"value": 0, "label": "initial"}
        }
    }));

    let tool = UpdateCallStateTool;

    // First execution
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_abc", "tool:update_call");

        let result = tool
            .execute(json!({"label": "step1"}), &ctx)
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["step"], 1);

        manager.apply(ctx.take_patch()).await.unwrap();
    }

    // Second execution
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_abc", "tool:update_call");

        let result = tool
            .execute(json!({"label": "step2"}), &ctx)
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["step"], 2);

        manager.apply(ctx.take_patch()).await.unwrap();
    }

    let final_state = manager.snapshot().await;
    assert_eq!(final_state["tool_calls"]["call_abc"]["value"], 2);
    assert_eq!(final_state["tool_calls"]["call_abc"]["label"], "step2");
}

#[tokio::test]
async fn test_tool_error_handling() {
    let manager = StateManager::new(json!({}));

    let tool = IncrementTool;
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "tool:increment");

    // Missing required argument
    let result = tool.execute(json!({}), &ctx).await;
    assert!(result.is_err());

    match result {
        Err(ToolError::InvalidArguments(msg)) => {
            assert!(msg.contains("path"));
        }
        _ => panic!("Expected InvalidArguments error"),
    }
}

// ============================================================================
// Context provider tests
// ============================================================================

#[tokio::test]
async fn test_context_provider_basic() {
    let manager = StateManager::new(json!({
        "counter": {"value": 5, "label": "test"}
    }));

    let provider = CounterContextProvider;

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "provider:counter");

    let messages = provider.provide(&ctx).await;
    assert!(messages.is_empty()); // value <= 10

    // Provider modifies state
    let patch = ctx.take_patch();
    manager.apply(patch).await.unwrap();

    let new_state = manager.snapshot().await;
    assert_eq!(new_state["counter"]["label"], "context_provided");
}

#[tokio::test]
async fn test_context_provider_with_high_value() {
    let manager = StateManager::new(json!({
        "counter": {"value": 15, "label": "test"}
    }));

    let provider = CounterContextProvider;

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "provider:counter");

    let messages = provider.provide(&ctx).await;
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("high"));
    assert!(messages[0].contains("15"));
}

#[tokio::test]
async fn test_context_provider_metadata() {
    let provider = CounterContextProvider;

    assert_eq!(provider.id(), "counter_context");
    assert_eq!(provider.category(), ContextCategory::ToolExecution);
    assert_eq!(provider.priority(), 100);
}

// ============================================================================
// System reminder tests
// ============================================================================

#[tokio::test]
async fn test_system_reminder_with_todos() {
    let manager = StateManager::new(json!({
        "todos": {"items": ["Task 1", "Task 2"], "count": 2}
    }));

    let reminder = TodoReminder;

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "reminder:todo");

    let message = reminder.remind(&ctx).await;
    assert!(message.is_some());
    assert!(message.unwrap().contains("2"));
}

#[tokio::test]
async fn test_system_reminder_no_todos() {
    let manager = StateManager::new(json!({
        "todos": {"items": [], "count": 0}
    }));

    let reminder = TodoReminder;

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_001", "reminder:todo");

    let message = reminder.remind(&ctx).await;
    assert!(message.is_none());
}

#[tokio::test]
async fn test_system_reminder_metadata() {
    let reminder = TodoReminder;
    assert_eq!(reminder.id(), "todo_reminder");
}

// ============================================================================
// Tool descriptor tests
// ============================================================================

#[test]
fn test_tool_descriptor_basic() {
    let desc = ToolDescriptor::new("my_tool", "My Tool", "A test tool");

    assert_eq!(desc.id, "my_tool");
    assert_eq!(desc.name, "My Tool");
    assert_eq!(desc.description, "A test tool");
    assert!(!desc.requires_confirmation);
    assert!(desc.category.is_none());
}

#[test]
fn test_tool_descriptor_with_options() {
    let desc = ToolDescriptor::new("my_tool", "My Tool", "A test tool")
        .with_parameters(json!({"type": "object"}))
        .with_confirmation(true)
        .with_category("testing")
        .with_metadata("version", json!("1.0"));

    assert!(desc.requires_confirmation);
    assert_eq!(desc.category, Some("testing".to_string()));
    assert_eq!(desc.metadata.get("version"), Some(&json!("1.0")));
}

// ============================================================================
// Tool result tests
// ============================================================================

#[test]
fn test_tool_result_success() {
    let result = ToolResult::success("my_tool", json!({"data": 123}));

    assert!(result.is_success());
    assert!(!result.is_error());
    assert!(!result.is_pending());
    assert_eq!(result.tool_name, "my_tool");
    assert_eq!(result.data["data"], 123);
}

#[test]
fn test_tool_result_error() {
    let result = ToolResult::error("my_tool", "Something went wrong");

    assert!(result.is_error());
    assert!(!result.is_success());
    assert_eq!(result.message, Some("Something went wrong".to_string()));
}

#[test]
fn test_tool_result_pending() {
    let result = ToolResult::pending("my_tool", "Waiting for user input");

    assert!(result.is_pending());
    assert!(!result.is_success());
    assert!(!result.is_error());
}

#[test]
fn test_tool_result_warning() {
    let result = ToolResult::warning("my_tool", json!({}), "Partial success");

    assert!(result.is_success()); // Warning is still considered success
    assert!(!result.is_error());
}

#[test]
fn test_tool_result_with_metadata() {
    let result = ToolResult::success("my_tool", json!({}))
        .with_metadata("timing", json!({"ms": 100}))
        .with_metadata("version", json!("1.0"));

    assert_eq!(result.metadata.len(), 2);
    assert_eq!(result.metadata["timing"]["ms"], 100);
}

// ============================================================================
// Full workflow tests
// ============================================================================

#[tokio::test]
async fn test_full_tool_workflow() {
    // Simulate a complete tool execution workflow
    let manager = StateManager::new(json!({
        "todos": {"items": [], "count": 0},
        "tool_calls": {}
    }));

    let tool = AddTodoTool;

    // Execute tool multiple times
    for i in 1..=3 {
        let call_id = format!("call_{}", i);
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, &call_id, "tool:add_todo");

        let result = tool
            .execute(json!({"item": format!("Task {}", i)}), &ctx)
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["count"], i);

        // Apply changes
        manager.apply(ctx.take_patch()).await.unwrap();
    }

    // Verify final state
    let final_state = manager.snapshot().await;
    assert_eq!(
        final_state["todos"]["items"],
        json!(["Task 1", "Task 2", "Task 3"])
    );
    assert_eq!(final_state["todos"]["count"], 3);

    // Verify history
    assert_eq!(manager.history_len().await, 3);

    // Replay to middle state
    let mid_state = manager.replay_to(1).await.unwrap();
    assert_eq!(mid_state["todos"]["items"], json!(["Task 1", "Task 2"]));
    assert_eq!(mid_state["todos"]["count"], 2);
}

#[tokio::test]
async fn test_tool_provider_reminder_integration() {
    let manager = StateManager::new(json!({
        "counter": {"value": 0, "label": "initial"},
        "todos": {"items": [], "count": 0}
    }));

    let increment_tool = IncrementTool;
    let todo_tool = AddTodoTool;
    let provider = CounterContextProvider;
    let reminder = TodoReminder;

    // Execute increment tool
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_1", "tool:increment");
        let _ = increment_tool
            .execute(json!({"path": "counter"}), &ctx)
            .await
            .unwrap();
        manager.apply(ctx.take_patch()).await.unwrap();
    }

    // Execute add todo tool
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_2", "tool:add_todo");
        let _ = todo_tool
            .execute(json!({"item": "New task"}), &ctx)
            .await
            .unwrap();
        manager.apply(ctx.take_patch()).await.unwrap();
    }

    // Run context provider
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_3", "provider:counter");
        let messages = provider.provide(&ctx).await;
        assert!(messages.is_empty()); // value is only 1
        manager.apply(ctx.take_patch()).await.unwrap();
    }

    // Run system reminder
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_4", "reminder:todo");
        let message = reminder.remind(&ctx).await;
        assert!(message.is_some());
        assert!(message.unwrap().contains("1")); // 1 pending todo
    }

    // Verify final state
    let final_state = manager.snapshot().await;
    assert_eq!(final_state["counter"]["value"], 1);
    assert_eq!(final_state["counter"]["label"], "context_provided");
    assert_eq!(final_state["todos"]["count"], 1);
}
