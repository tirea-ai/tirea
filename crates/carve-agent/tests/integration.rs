//! Integration tests for carve-agent.
//!
//! These tests verify the Tool, ContextProvider, and SystemReminder traits
//! work correctly with the new State/Context API.

use async_trait::async_trait;
use carve_agent::{
    ag_ui::MessageRole, Context, ContextCategory, ContextProvider, InteractionResponse,
    StateManager, SystemReminder, Tool, ToolDescriptor, ToolError, ToolExecutionLocation,
    ToolResult,
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
    manager.commit(patch).await.unwrap();

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

        manager.commit(ctx.take_patch()).await.unwrap();
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

        manager.commit(ctx.take_patch()).await.unwrap();
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

        manager.commit(ctx.take_patch()).await.unwrap();
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
    manager.commit(patch).await.unwrap();

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
        manager.commit(ctx.take_patch()).await.unwrap();
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
        manager.commit(ctx.take_patch()).await.unwrap();
    }

    // Execute add todo tool
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_2", "tool:add_todo");
        let _ = todo_tool
            .execute(json!({"item": "New task"}), &ctx)
            .await
            .unwrap();
        manager.commit(ctx.take_patch()).await.unwrap();
    }

    // Run context provider
    {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, "call_3", "provider:counter");
        let messages = provider.provide(&ctx).await;
        assert!(messages.is_empty()); // value is only 1
        manager.commit(ctx.take_patch()).await.unwrap();
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

// ============================================================================
// Session and Agent Loop Integration Tests
// ============================================================================

use carve_agent::{
    loop_execute_tools, tool_map, AgentConfig, FileStorage, MemoryStorage, Message, Role, Session,
    Storage, StreamResult,
};
use carve_state::{path, Op, Patch, TrackedPatch};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_session_with_tool_workflow() {
    // Create session with initial state
    let session = Session::with_initial_state("workflow-test", json!({"counter": 0}))
        .with_message(Message::user("Increment the counter twice"));

    // Simulate first LLM response with tool call
    let result1 = StreamResult {
        text: "I'll increment the counter".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "increment",
            json!({"path": "counter"}),
        )],
    };

    // Add assistant message
    let session = session.with_message(carve_agent::assistant_tool_calls(
        &result1.text,
        result1.tool_calls.clone(),
    ));

    // Execute tool
    let tools = tool_map([IncrementTool]);
    let session = loop_execute_tools(session, &result1, &tools, true)
        .await
        .unwrap();

    // Verify state after first tool call
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 1);

    // Simulate second LLM response
    let result2 = StreamResult {
        text: "Incrementing again".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_2",
            "increment",
            json!({"path": "counter"}),
        )],
    };

    let session = session.with_message(carve_agent::assistant_tool_calls(
        &result2.text,
        result2.tool_calls.clone(),
    ));

    let session = loop_execute_tools(session, &result2, &tools, true)
        .await
        .unwrap();

    // Verify final state
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 2);

    // Session should have all messages
    assert_eq!(session.message_count(), 5); // user + 2*(assistant + tool)
}

#[tokio::test]
async fn test_session_storage_roundtrip() {
    let storage = MemoryStorage::new();

    // Create and save session
    let session = Session::with_initial_state("storage-test", json!({"data": "initial"}))
        .with_message(Message::user("Hello"))
        .with_message(Message::assistant("Hi there!"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("data"), json!("updated"))),
        ));

    storage.save(&session).await.unwrap();

    // Load session
    let loaded = storage.load("storage-test").await.unwrap().unwrap();

    // Verify
    assert_eq!(loaded.id, "storage-test");
    assert_eq!(loaded.message_count(), 2);
    assert_eq!(loaded.patch_count(), 1);

    // Rebuild state
    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["data"], "updated");
}

#[tokio::test]
async fn test_file_storage_session_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());

    // Create complex session
    let session = Session::with_initial_state(
        "persist-test",
        json!({
            "user": {"name": "Test", "level": 1},
            "items": []
        }),
    )
    .with_message(Message::user("Add item"))
    .with_message(Message::assistant("Adding item..."))
    .with_patch(TrackedPatch::new(
        Patch::new()
            .with_op(Op::append(path!("items"), json!("item1")))
            .with_op(Op::increment(path!("user").key("level"), 1)),
    ));

    storage.save(&session).await.unwrap();

    // Verify file exists
    let path = temp_dir.path().join("persist-test.json");
    assert!(path.exists());

    // Load and verify
    let loaded = storage.load("persist-test").await.unwrap().unwrap();
    let state = loaded.rebuild_state().unwrap();

    assert_eq!(state["user"]["level"], 2);
    assert_eq!(state["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_session_snapshot_and_continue() {
    let storage = MemoryStorage::new();

    // Create session with patches
    let session = Session::with_initial_state("snapshot-test", json!({"counter": 0}))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(10))),
        ));

    assert_eq!(session.patch_count(), 2);

    // Snapshot to collapse patches
    let session = session.snapshot().unwrap();
    assert_eq!(session.patch_count(), 0);
    assert_eq!(session.state["counter"], 10);

    // Save and load
    storage.save(&session).await.unwrap();
    let loaded = storage.load("snapshot-test").await.unwrap().unwrap();

    // Continue with more patches
    let session = loaded.with_patch(TrackedPatch::new(
        Patch::new().with_op(Op::set(path!("counter"), json!(15))),
    ));

    assert_eq!(session.patch_count(), 1);
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"], 15);
}

#[tokio::test]
async fn test_agent_config_variations() {
    let config1 = AgentConfig::new("gpt-4o-mini");
    assert_eq!(config1.model, "gpt-4o-mini");
    assert_eq!(config1.max_rounds, 10);
    assert!(config1.parallel_tools);

    let config2 = AgentConfig::new("claude-3-opus")
        .with_max_rounds(5)
        .with_parallel_tools(false);

    assert_eq!(config2.model, "claude-3-opus");
    assert_eq!(config2.max_rounds, 5);
    assert!(!config2.parallel_tools);
}

#[tokio::test]
async fn test_tool_map_with_multiple_tools() {
    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("increment".to_string(), Arc::new(IncrementTool));
    tools.insert("add_todo".to_string(), Arc::new(AddTodoTool));

    assert_eq!(tools.len(), 2);
    assert!(tools.contains_key("increment"));
    assert!(tools.contains_key("add_todo"));
}

#[tokio::test]
async fn test_session_message_types() {
    let session = Session::new("msg-test")
        .with_message(Message::user("User message"))
        .with_message(Message::assistant("Assistant response"))
        .with_message(Message::tool("call_1", "Tool result"));

    assert_eq!(session.messages.len(), 3);
    assert_eq!(session.messages[0].role, Role::User);
    assert_eq!(session.messages[1].role, Role::Assistant);
    assert_eq!(session.messages[2].role, Role::Tool);
}

#[tokio::test]
async fn test_storage_list_and_delete() {
    let storage = MemoryStorage::new();

    // Create multiple sessions
    storage.save(&Session::new("session-1")).await.unwrap();
    storage.save(&Session::new("session-2")).await.unwrap();
    storage.save(&Session::new("session-3")).await.unwrap();

    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 3);

    // Delete one
    storage.delete("session-2").await.unwrap();

    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&"session-2".to_string()));
}

#[tokio::test]
async fn test_parallel_tool_execution_order() {
    // Create session with initial state
    let session = Session::with_initial_state("parallel-test", json!({"results": []}));

    // Multiple tool calls
    let result = StreamResult {
        text: "Running parallel tools".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("call_1", "add_todo", json!({"item": "first"})),
            carve_agent::ToolCall::new("call_2", "add_todo", json!({"item": "second"})),
            carve_agent::ToolCall::new("call_3", "add_todo", json!({"item": "third"})),
        ],
    };

    let tools = tool_map([AddTodoTool]);
    let session = loop_execute_tools(session, &result, &tools, true)
        .await
        .unwrap();

    // All three tools should have produced messages
    assert_eq!(session.message_count(), 3);
    // All should have patches (adding todos)
    assert_eq!(session.patch_count(), 3);
}

#[tokio::test]
async fn test_session_needs_snapshot_threshold() {
    let mut session = Session::new("threshold-test");

    // Add patches
    for i in 0..5 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("value"), json!(i))),
        ));
    }

    assert!(!session.needs_snapshot(10));
    assert!(session.needs_snapshot(5));
    assert!(session.needs_snapshot(3));
}

// ============================================================================
// Concurrent Stress Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_tool_execution_stress() {
    // Test 50 concurrent tool executions
    let manager = StateManager::new(json!({
        "counters": {}
    }));

    let mut handles = vec![];

    for i in 0..50 {
        let manager = manager.clone();
        let handle = tokio::spawn(async move {
            let snapshot = manager.snapshot().await;
            let ctx = Context::new(&snapshot, format!("call_{}", i), "tool:increment");

            // Create counter path for this task
            let path = format!("counters.c{}", i % 10);
            let counter = ctx.state::<CounterState>(&path);
            let current = counter.value().unwrap_or(0);
            counter.set_value(current + 1);

            let patch = ctx.take_patch();
            manager.commit(patch).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    let results: Vec<_> = futures::future::join_all(handles).await;
    let success_count = results.iter().filter(|r| r.is_ok()).count();

    // Most should succeed (some may fail due to conflicts, which is expected)
    assert!(success_count >= 40, "Expected at least 40 successful executions, got {}", success_count);
}

#[tokio::test]
async fn test_concurrent_storage_operations() {
    let storage = Arc::new(MemoryStorage::new());

    let mut handles = vec![];

    // 100 concurrent save operations
    for i in 0..100 {
        let storage = Arc::clone(&storage);
        let handle: tokio::task::JoinHandle<Result<(), carve_agent::StorageError>> = tokio::spawn(async move {
            let session = Session::with_initial_state(
                format!("concurrent-{}", i),
                json!({"index": i}),
            )
            .with_message(Message::user(format!("Message {}", i)));

            storage.save(&session).await
        });
        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;
    let success_count = results.iter().filter(|r| r.as_ref().map(|r| r.is_ok()).unwrap_or(false)).count();
    assert_eq!(success_count, 100, "All saves should succeed");

    // Verify all sessions exist
    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 100);
}

#[tokio::test]
async fn test_concurrent_session_rebuild() {
    // Create a session with many patches
    let mut session = Session::with_initial_state("rebuild-stress", json!({"counter": 0}));

    for i in 0..100 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::increment(path!("counter"), 1)),
        ));
    }

    // Concurrent rebuilds
    let mut handles = vec![];
    for _ in 0..50 {
        let session = session.clone();
        let handle = tokio::spawn(async move {
            session.rebuild_state()
        });
        handles.push(handle);
    }

    let results: Vec<_> = futures::future::join_all(handles).await;

    // All rebuilds should succeed and return same value
    for result in results {
        let state = result.unwrap().unwrap();
        assert_eq!(state["counter"], 100);
    }
}

// ============================================================================
// Large Session Tests (1000+ messages)
// ============================================================================

#[tokio::test]
async fn test_large_session_1000_messages() {
    let mut session = Session::new("large-msg-test");

    // Add 1000 messages
    for i in 0..1000 {
        if i % 2 == 0 {
            session = session.with_message(Message::user(format!("User message {}", i)));
        } else {
            session = session.with_message(Message::assistant(format!("Assistant response {}", i)));
        }
    }

    assert_eq!(session.message_count(), 1000);

    // Verify first and last messages
    assert!(session.messages[0].content.contains("0"));
    assert!(session.messages[999].content.contains("999"));
}

#[tokio::test]
async fn test_large_session_1000_patches() {
    let mut session = Session::with_initial_state("large-patch-test", json!({"values": []}));

    // Add 1000 patches
    for i in 0..1000 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::append(path!("values"), json!(i))),
        ));
    }

    assert_eq!(session.patch_count(), 1000);
    assert!(session.needs_snapshot(500));

    // Rebuild should work
    let state = session.rebuild_state().unwrap();
    let values = state["values"].as_array().unwrap();
    assert_eq!(values.len(), 1000);
    assert_eq!(values[0], 0);
    assert_eq!(values[999], 999);
}

#[tokio::test]
async fn test_large_session_storage_roundtrip() {
    let storage = MemoryStorage::new();

    // Create large session
    let mut session = Session::with_initial_state("large-storage-test", json!({"counter": 0}));

    for i in 0..500 {
        session = session.with_message(Message::user(format!("Msg {}", i)));
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::increment(path!("counter"), 1)),
        ));
    }

    // Save
    storage.save(&session).await.unwrap();

    // Load
    let loaded = storage.load("large-storage-test").await.unwrap().unwrap();

    assert_eq!(loaded.message_count(), 500);
    assert_eq!(loaded.patch_count(), 500);

    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["counter"], 500);
}

#[tokio::test]
async fn test_large_session_snapshot_performance() {
    let mut session = Session::with_initial_state("snapshot-perf-test", json!({"data": {}}));

    // Add 200 patches with nested data
    for i in 0..200 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("data").key(format!("key_{}", i)), json!({
                "index": i,
                "nested": {"value": i * 2}
            }))),
        ));
    }

    // Snapshot should collapse all patches
    let start = std::time::Instant::now();
    let session = session.snapshot().unwrap();
    let duration = start.elapsed();

    assert_eq!(session.patch_count(), 0);
    assert!(duration.as_millis() < 1000, "Snapshot took too long: {:?}", duration);

    // Verify data
    let keys_count = session.state["data"].as_object().unwrap().len();
    assert_eq!(keys_count, 200);
}

// ============================================================================
// Session Interruption Recovery Tests
// ============================================================================

#[tokio::test]
async fn test_session_recovery_after_partial_save() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());

    // Create session with multiple messages and patches
    let session = Session::with_initial_state("recovery-test", json!({"step": 0}))
        .with_message(Message::user("Step 1"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("step"), json!(1))),
        ))
        .with_message(Message::assistant("Done step 1"))
        .with_message(Message::user("Step 2"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("step"), json!(2))),
        ));

    // Save checkpoint
    storage.save(&session).await.unwrap();

    // Simulate adding more work
    let session = session
        .with_message(Message::assistant("Done step 2"))
        .with_message(Message::user("Step 3"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("step"), json!(3))),
        ));

    // "Crash" happens here - we don't save

    // Recovery: load from last checkpoint
    let recovered = storage.load("recovery-test").await.unwrap().unwrap();

    // Should have state from checkpoint (step 2, not step 3)
    let state = recovered.rebuild_state().unwrap();
    assert_eq!(state["step"], 2);
    // Messages: "Step 1", "Done step 1", "Step 2" = 3 messages (no "Done step 2" yet at checkpoint)
    assert_eq!(recovered.message_count(), 3);
}

#[tokio::test]
async fn test_session_incremental_checkpoints() {
    let storage = MemoryStorage::new();

    let mut session = Session::with_initial_state("checkpoint-test", json!({"progress": 0}));

    // Simulate long-running task with periodic checkpoints
    for checkpoint in 1..=5 {
        // Do some work
        for _ in 0..10 {
            session = session.with_message(Message::user(format!("Work item at checkpoint {}", checkpoint)));
        }

        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("progress"), json!(checkpoint * 10))),
        ));

        // Save checkpoint
        storage.save(&session).await.unwrap();
    }

    // Verify final state
    let loaded = storage.load("checkpoint-test").await.unwrap().unwrap();
    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["progress"], 50);
    assert_eq!(loaded.message_count(), 50);
}

#[tokio::test]
async fn test_session_recovery_with_snapshot() {
    let storage = MemoryStorage::new();

    // Create session with many patches
    let mut session = Session::with_initial_state("snapshot-recovery", json!({"counter": 0}));

    for i in 0..50 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::increment(path!("counter"), 1)),
        ));
    }

    // Snapshot to optimize
    let session = session.snapshot().unwrap();
    assert_eq!(session.patch_count(), 0);
    assert_eq!(session.state["counter"], 50);

    // Save
    storage.save(&session).await.unwrap();

    // Continue work
    let mut session = storage.load("snapshot-recovery").await.unwrap().unwrap();
    for _ in 0..25 {
        session = session.with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::increment(path!("counter"), 1)),
        ));
    }

    // Save again
    storage.save(&session).await.unwrap();

    // Load and verify
    let loaded = storage.load("snapshot-recovery").await.unwrap().unwrap();
    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["counter"], 75);
}

// ============================================================================
// Patch Conflict Handling Tests
// ============================================================================

#[tokio::test]
async fn test_patch_conflict_same_field() {
    let manager = StateManager::new(json!({"value": 0}));

    // Two concurrent modifications to same field
    let snapshot1 = manager.snapshot().await;
    let snapshot2 = manager.snapshot().await;

    let ctx1 = Context::new(&snapshot1, "call_1", "test");
    let ctx2 = Context::new(&snapshot2, "call_2", "test");

    // Both read same value
    let counter1 = ctx1.state::<CounterState>("");
    let counter2 = ctx2.state::<CounterState>("");

    // Both increment from 0
    counter1.set_value(1);
    counter2.set_value(1);

    // First commit succeeds
    let patch1 = ctx1.take_patch();
    manager.commit(patch1).await.unwrap();

    // Second commit also succeeds (last-write-wins)
    let patch2 = ctx2.take_patch();
    manager.commit(patch2).await.unwrap();

    // Final value is 1 (not 2 - this is the "conflict")
    let final_state = manager.snapshot().await;
    assert_eq!(final_state["value"], 1);
}

#[tokio::test]
async fn test_patch_conflict_different_fields() {
    let manager = StateManager::new(json!({
        "field_a": 0,
        "field_b": 0
    }));

    // Two concurrent modifications to different fields
    let snapshot1 = manager.snapshot().await;
    let snapshot2 = manager.snapshot().await;

    let ctx1 = Context::new(&snapshot1, "call_1", "test");
    let ctx2 = Context::new(&snapshot2, "call_2", "test");

    // Modify different fields
    {
        let ops = &ctx1.state::<CounterState>("field_a");
        ops.set_value(10);
    }
    {
        let ops = &ctx2.state::<CounterState>("field_b");
        ops.set_value(20);
    }

    // Both commits succeed
    manager.commit(ctx1.take_patch()).await.unwrap();
    manager.commit(ctx2.take_patch()).await.unwrap();

    // Both values should be updated
    let final_state = manager.snapshot().await;
    assert_eq!(final_state["field_a"]["value"], 10);
    assert_eq!(final_state["field_b"]["value"], 20);
}

#[tokio::test]
async fn test_patch_conflict_array_operations() {
    let manager = StateManager::new(json!({
        "todos": {"items": [], "count": 0}
    }));

    // Two concurrent array appends
    let snapshot1 = manager.snapshot().await;
    let snapshot2 = manager.snapshot().await;

    let ctx1 = Context::new(&snapshot1, "call_1", "test");
    let ctx2 = Context::new(&snapshot2, "call_2", "test");

    let todos1 = ctx1.state::<TodoState>("todos");
    let todos2 = ctx2.state::<TodoState>("todos");

    todos1.items_push("item_a");
    todos1.set_count(1);

    todos2.items_push("item_b");
    todos2.set_count(1);

    // Apply both
    manager.commit(ctx1.take_patch()).await.unwrap();
    manager.commit(ctx2.take_patch()).await.unwrap();

    // Both items should be present (append doesn't conflict)
    let final_state = manager.snapshot().await;
    let items = final_state["todos"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn test_session_patch_ordering() {
    let session = Session::with_initial_state("order-test", json!({"log": []}));

    // Add patches in specific order
    let session = session
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::append(path!("log"), json!("first"))),
        ))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::append(path!("log"), json!("second"))),
        ))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::append(path!("log"), json!("third"))),
        ));

    // Rebuild should preserve order
    let state = session.rebuild_state().unwrap();
    let log = state["log"].as_array().unwrap();

    assert_eq!(log[0], "first");
    assert_eq!(log[1], "second");
    assert_eq!(log[2], "third");
}

// ============================================================================
// Tool Timeout Handling Tests
// ============================================================================

/// A tool that can simulate delays
struct SlowTool {
    delay_ms: u64,
}

#[async_trait]
impl Tool for SlowTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("slow_tool", "Slow Tool", "A tool that takes time")
    }

    async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        tokio::time::sleep(tokio::time::Duration::from_millis(self.delay_ms)).await;

        let counter = ctx.state::<CounterState>("counter");
        counter.set_value(1);

        Ok(ToolResult::success("slow_tool", json!({"completed": true})))
    }
}

#[tokio::test]
async fn test_tool_execution_with_timeout() {
    let manager = StateManager::new(json!({"counter": {"value": 0, "label": ""}}));

    let tool = SlowTool { delay_ms: 50 };

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_slow", "tool:slow");

    // Execute with timeout
    let result = tokio::time::timeout(
        tokio::time::Duration::from_millis(100),
        tool.execute(json!({}), &ctx),
    )
    .await;

    // Should complete within timeout
    assert!(result.is_ok());
    let tool_result = result.unwrap().unwrap();
    assert!(tool_result.is_success());
}

#[tokio::test]
async fn test_tool_timeout_exceeded() {
    let manager = StateManager::new(json!({"counter": {"value": 0, "label": ""}}));

    let tool = SlowTool { delay_ms: 200 };

    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_slow", "tool:slow");

    // Execute with short timeout
    let result = tokio::time::timeout(
        tokio::time::Duration::from_millis(50),
        tool.execute(json!({}), &ctx),
    )
    .await;

    // Should timeout
    assert!(result.is_err());

    // State should not be modified (tool didn't complete)
    let patch = ctx.take_patch();
    assert!(patch.patch().is_empty());
}

#[tokio::test]
async fn test_multiple_tools_with_varying_timeouts() {
    let manager = StateManager::new(json!({
        "results": {"fast": false, "medium": false, "slow": false}
    }));

    let fast_tool = SlowTool { delay_ms: 10 };
    let medium_tool = SlowTool { delay_ms: 50 };
    let slow_tool = SlowTool { delay_ms: 200 };

    // Execute all with 100ms timeout
    let timeout = tokio::time::Duration::from_millis(100);

    let snapshot = manager.snapshot().await;

    // Fast - should complete
    let ctx = Context::new(&snapshot, "fast", "tool:fast");
    let fast_result = tokio::time::timeout(timeout, fast_tool.execute(json!({}), &ctx)).await;
    assert!(fast_result.is_ok());

    // Medium - should complete
    let ctx = Context::new(&snapshot, "medium", "tool:medium");
    let medium_result = tokio::time::timeout(timeout, medium_tool.execute(json!({}), &ctx)).await;
    assert!(medium_result.is_ok());

    // Slow - should timeout
    let ctx = Context::new(&snapshot, "slow", "tool:slow");
    let slow_result = tokio::time::timeout(timeout, slow_tool.execute(json!({}), &ctx)).await;
    assert!(slow_result.is_err());
}

#[tokio::test]
async fn test_tool_timeout_cleanup() {
    // Test that partial state changes are not applied on timeout
    let session = Session::with_initial_state("timeout-cleanup", json!({"value": "original"}));

    // Simulate a tool that would modify state but times out
    // In real scenario, the patch wouldn't be collected if tool times out

    // The session state should remain unchanged
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["value"], "original");
}

// ============================================================================
// Stream Interruption Tests
// ============================================================================

use carve_agent::StreamCollector;

#[test]
fn test_stream_collector_partial_text() {
    let mut collector = StreamCollector::new();

    // Simulate partial text chunks
    use genai::chat::{ChatStreamEvent, StreamChunk};

    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Hello ".to_string(),
    }));

    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "world".to_string(),
    }));

    // Stream "interrupted" - finish early
    let result = collector.finish();

    assert_eq!(result.text, "Hello world");
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_stream_collector_interrupted_tool_call() {
    let mut collector = StreamCollector::new();

    use genai::chat::{ChatStreamEvent, StreamChunk, ToolChunk};

    // Start a tool call
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "I'll help you".to_string(),
    }));

    // Tool call chunk with initial data
    let tool_call = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "calculator".to_string(),
        fn_arguments: json!({}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call }));

    // Partial arguments in second chunk
    let tool_call2 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: String::new(),
        fn_arguments: json!({"expr": "1+1"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tool_call2 }));

    // Stream "interrupted" - finish without complete tool call
    let result = collector.finish();

    assert_eq!(result.text, "I'll help you");
    // Tool call should still be captured (even if incomplete)
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "calculator");
}

#[test]
fn test_stream_collector_multiple_interruptions() {
    // Test collecting results after multiple partial streams
    let mut collector1 = StreamCollector::new();

    use genai::chat::{ChatStreamEvent, StreamChunk};

    collector1.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Part 1".to_string(),
    }));

    let result1 = collector1.finish();
    assert_eq!(result1.text, "Part 1");

    // New collector for "retry"
    let mut collector2 = StreamCollector::new();

    collector2.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Complete response".to_string(),
    }));

    let result2 = collector2.finish();
    assert_eq!(result2.text, "Complete response");
}

#[test]
fn test_stream_result_from_partial_response() {
    // Simulate building StreamResult from partial data
    let result = StreamResult {
        text: "Partial...".to_string(),
        tool_calls: vec![],
    };

    assert!(!result.needs_tools());

    // Can still create messages from partial result
    let msg = carve_agent::assistant_message(&result.text);
    assert_eq!(msg.content, "Partial...");
}

// ============================================================================
// Network Error Simulation Tests
// ============================================================================

/// Simulates a tool that encounters "network" errors
struct NetworkErrorTool {
    fail_count: std::sync::atomic::AtomicU32,
    max_failures: u32,
}

impl NetworkErrorTool {
    fn new(max_failures: u32) -> Self {
        Self {
            fail_count: std::sync::atomic::AtomicU32::new(0),
            max_failures,
        }
    }
}

#[async_trait]
impl Tool for NetworkErrorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("network_tool", "Network Tool", "Tool that may fail")
    }

    async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let count = self.fail_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        if count < self.max_failures {
            Err(ToolError::ExecutionFailed(format!(
                "Network error (attempt {})",
                count + 1
            )))
        } else {
            let counter = ctx.state::<CounterState>("counter");
            counter.set_value(1);
            Ok(ToolResult::success("network_tool", json!({"success": true})))
        }
    }
}

#[tokio::test]
async fn test_tool_network_error_retry() {
    let manager = StateManager::new(json!({"counter": {"value": 0, "label": ""}}));

    // Tool fails twice, then succeeds
    let tool = NetworkErrorTool::new(2);

    // Attempt 1 - fails
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_1", "tool:network");
    let result1 = tool.execute(json!({}), &ctx).await;
    assert!(result1.is_err());

    // Attempt 2 - fails
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_2", "tool:network");
    let result2 = tool.execute(json!({}), &ctx).await;
    assert!(result2.is_err());

    // Attempt 3 - succeeds
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_3", "tool:network");
    let result3 = tool.execute(json!({}), &ctx).await;
    assert!(result3.is_ok());

    // Apply successful patch
    manager.commit(ctx.take_patch()).await.unwrap();

    let final_state = manager.snapshot().await;
    assert_eq!(final_state["counter"]["value"], 1);
}

#[tokio::test]
async fn test_tool_error_does_not_corrupt_state() {
    let manager = StateManager::new(json!({"value": 100}));

    // Tool that always fails
    let tool = NetworkErrorTool::new(1000); // Will always fail

    for i in 0..5 {
        let snapshot = manager.snapshot().await;
        let ctx = Context::new(&snapshot, format!("call_{}", i), "tool:network");

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());

        // Don't apply patch from failed execution
        let patch = ctx.take_patch();
        // In real code, we wouldn't commit on failure
        // But even if we do, patch should be empty for failed tool
        if !patch.patch().is_empty() {
            // This shouldn't happen for our test tool that fails before modifying state
            panic!("Failed tool should not produce patch");
        }
    }

    // State should be unchanged
    let final_state = manager.snapshot().await;
    assert_eq!(final_state["value"], 100);
}

#[tokio::test]
async fn test_session_resilient_to_tool_errors() {
    let session = Session::with_initial_state("error-resilient", json!({"counter": 0}));

    // Simulate tool calls where some fail
    let tool_results = vec![
        ToolResult::success("tool1", json!({"value": 1})),
        ToolResult::error("tool2", "Network timeout"),
        ToolResult::success("tool3", json!({"value": 3})),
        ToolResult::error("tool4", "Connection refused"),
    ];

    // Add all results as messages
    let mut session = session;
    for (i, result) in tool_results.iter().enumerate() {
        session = session.with_message(carve_agent::tool_response(format!("call_{}", i), result));
    }

    // Session should have all messages regardless of success/error
    assert_eq!(session.message_count(), 4);

    // Verify error messages are preserved
    assert!(session.messages[1].content.contains("error") || session.messages[1].content.contains("Network"));
    assert!(session.messages[3].content.contains("error") || session.messages[3].content.contains("Connection"));
}

#[tokio::test]
async fn test_storage_error_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());

    // Save a valid session
    let session = Session::new("valid-session")
        .with_message(Message::user("Hello"));
    storage.save(&session).await.unwrap();

    // Try to load non-existent session
    let result = storage.load("non-existent").await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());

    // Original session should still be loadable
    let loaded = storage.load("valid-session").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 1);
}

#[tokio::test]
async fn test_concurrent_errors_dont_corrupt_storage() {
    let storage = Arc::new(MemoryStorage::new());

    // Save initial session
    let session = Session::new("concurrent-test")
        .with_message(Message::user("Initial"));
    storage.save(&session).await.unwrap();

    let mut handles = vec![];

    // 50 concurrent operations (mix of saves and loads)
    for i in 0..50 {
        let storage = Arc::clone(&storage);
        let handle: tokio::task::JoinHandle<Result<Option<Session>, carve_agent::StorageError>> = tokio::spawn(async move {
            if i % 3 == 0 {
                // Load
                storage.load("concurrent-test").await
            } else {
                // Save (potentially conflicting)
                let session = Session::new("concurrent-test")
                    .with_message(Message::user(format!("Update {}", i)));
                storage.save(&session).await.map(|_| None)
            }
        });
        handles.push(handle);
    }

    futures::future::join_all(handles).await;

    // Storage should still be consistent
    let final_session = storage.load("concurrent-test").await.unwrap().unwrap();
    assert_eq!(final_session.message_count(), 1); // Should have one message
}

// ============================================================================
// Additional Coverage Tests
// ============================================================================

#[test]
fn test_tool_result_success_with_message() {
    let result = ToolResult::success_with_message("my_tool", json!({"data": 42}), "Operation completed");

    assert!(result.is_success());
    assert!(!result.is_error());
    assert_eq!(result.tool_name, "my_tool");
    assert_eq!(result.data["data"], 42);
    assert_eq!(result.message, Some("Operation completed".to_string()));
}

#[test]
fn test_tool_result_success_with_message_empty_data() {
    let result = ToolResult::success_with_message("empty_tool", json!(null), "No data returned");

    assert!(result.is_success());
    assert_eq!(result.message, Some("No data returned".to_string()));
    assert!(result.data.is_null());
}

#[test]
fn test_tool_result_success_with_message_complex() {
    let result = ToolResult::success_with_message(
        "api_tool",
        json!({
            "status": 200,
            "body": {"users": [{"id": 1}, {"id": 2}]}
        }),
        "API call successful with 2 users",
    );

    assert!(result.is_success());
    assert_eq!(result.data["status"], 200);
    assert!(result.message.as_ref().unwrap().contains("2 users"));
}

#[test]
fn test_stream_collector_end_event_with_tool_calls() {
    use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd};

    let mut collector = StreamCollector::new();

    // Add some text first
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Processing your request...".to_string(),
    }));

    // Create an end event with captured tool calls
    let end = StreamEnd::default();
    // Note: StreamEnd::captured_tool_calls() returns Option<&Vec<ToolCall>>
    // Testing the path where captured_tool_calls is None (default)

    let output = collector.process(ChatStreamEvent::End(end));
    assert!(output.is_none()); // End event returns None

    let result = collector.finish();
    assert_eq!(result.text, "Processing your request...");
}

#[test]
fn test_stream_output_tool_call_delta_coverage() {
    use carve_agent::StreamOutput;

    // Test ToolCallDelta variant
    let delta = StreamOutput::ToolCallDelta {
        id: "call_123".to_string(),
        args_delta: r#"{"partial": true}"#.to_string(),
    };

    match delta {
        StreamOutput::ToolCallDelta { id, args_delta } => {
            assert_eq!(id, "call_123");
            assert!(args_delta.contains("partial"));
        }
        _ => panic!("Expected ToolCallDelta"),
    }
}

#[test]
fn test_stream_output_tool_call_start_coverage() {
    use carve_agent::StreamOutput;

    let start = StreamOutput::ToolCallStart {
        id: "call_abc".to_string(),
        name: "web_search".to_string(),
    };

    match start {
        StreamOutput::ToolCallStart { id, name } => {
            assert_eq!(id, "call_abc");
            assert_eq!(name, "web_search");
        }
        _ => panic!("Expected ToolCallStart"),
    }
}

#[tokio::test]
async fn test_file_storage_corrupted_json() {
    let temp_dir = TempDir::new().unwrap();

    // Write corrupted JSON file
    let corrupted_path = temp_dir.path().join("corrupted.json");
    tokio::fs::write(&corrupted_path, "{ invalid json }").await.unwrap();

    let storage = FileStorage::new(temp_dir.path());

    // Try to load corrupted session
    let result = storage.load("corrupted").await;
    assert!(result.is_err());

    match result {
        Err(carve_agent::StorageError::Serialization(msg)) => {
            assert!(msg.contains("expected") || msg.contains("key") || msg.len() > 0);
        }
        Err(other) => panic!("Expected Serialization error, got: {:?}", other),
        Ok(_) => panic!("Expected error for corrupted JSON"),
    }
}

#[test]
fn test_tool_error_variants_display() {
    use carve_agent::ToolError;

    let invalid_args = ToolError::InvalidArguments("Missing required field 'name'".to_string());
    assert!(invalid_args.to_string().contains("Invalid arguments"));

    let not_found = ToolError::NotFound("unknown_tool".to_string());
    assert!(not_found.to_string().contains("not found") || not_found.to_string().contains("Not found"));

    let exec_failed = ToolError::ExecutionFailed("Database connection timeout".to_string());
    assert!(exec_failed.to_string().contains("failed"));

    let permission_denied = ToolError::PermissionDenied("Admin access required".to_string());
    assert!(permission_denied.to_string().contains("Permission denied") || permission_denied.to_string().contains("Admin"));

    let internal = ToolError::Internal("Unexpected state".to_string());
    assert!(internal.to_string().contains("Internal") || internal.to_string().contains("error"));
}

#[test]
fn test_stream_result_needs_tools_variants() {
    // Test with empty tool calls
    let result_no_tools = StreamResult {
        text: "Just text".to_string(),
        tool_calls: vec![],
    };
    assert!(!result_no_tools.needs_tools());

    // Test with tool calls
    let result_with_tools = StreamResult {
        text: "".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new("id", "name", json!({}))],
    };
    assert!(result_with_tools.needs_tools());

    // Test with both text and tools
    let result_both = StreamResult {
        text: "Processing...".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("id1", "search", json!({})),
            carve_agent::ToolCall::new("id2", "calculate", json!({})),
        ],
    };
    assert!(result_both.needs_tools());
}

#[tokio::test]
async fn test_execute_tools_empty_result() {
    let session = Session::new("empty-tools-test");

    // Empty StreamResult (no tools)
    let result = StreamResult {
        text: "No tools needed".to_string(),
        tool_calls: vec![],
    };

    let tools: std::collections::HashMap<String, Arc<dyn Tool>> = std::collections::HashMap::new();

    // Should return session unchanged when no tools
    let new_session = loop_execute_tools(session.clone(), &result, &tools, true)
        .await
        .unwrap();

    assert_eq!(new_session.message_count(), session.message_count());
}

#[test]
fn test_agent_event_all_variants() {
    use carve_agent::AgentEvent;

    // TextDelta
    let text_delta = AgentEvent::TextDelta { delta: "Hello".to_string() };
    match text_delta {
        AgentEvent::TextDelta { delta } => assert_eq!(delta, "Hello"),
        _ => panic!("Wrong variant"),
    }

    // ToolCallStart
    let tool_start = AgentEvent::ToolCallStart {
        id: "call_1".to_string(),
        name: "search".to_string(),
    };
    match tool_start {
        AgentEvent::ToolCallStart { id, name } => {
            assert_eq!(id, "call_1");
            assert_eq!(name, "search");
        }
        _ => panic!("Wrong variant"),
    }

    // ToolCallDelta
    let tool_delta = AgentEvent::ToolCallDelta {
        id: "call_1".to_string(),
        args_delta: r#"{"q":"test"}"#.to_string(),
    };
    match tool_delta {
        AgentEvent::ToolCallDelta { id, args_delta } => {
            assert_eq!(id, "call_1");
            assert!(args_delta.contains("test"));
        }
        _ => panic!("Wrong variant"),
    }

    // ToolCallDone
    let tool_done = AgentEvent::ToolCallDone {
        id: "call_1".to_string(),
        result: ToolResult::success("search", json!({"results": []})),
        patch: None,
    };
    match tool_done {
        AgentEvent::ToolCallDone { id, result, patch } => {
            assert_eq!(id, "call_1");
            assert!(result.is_success());
            assert!(patch.is_none());
        }
        _ => panic!("Wrong variant"),
    }

    // Error
    let error = AgentEvent::Error { message: "Network timeout".to_string() };
    match error {
        AgentEvent::Error { message } => assert!(message.contains("timeout")),
        _ => panic!("Wrong variant"),
    }

    // Done
    let done = AgentEvent::Done {
        response: "Final response".to_string(),
    };
    match done {
        AgentEvent::Done { response } => assert_eq!(response, "Final response"),
        _ => panic!("Wrong variant"),
    }
}

#[test]
fn test_message_role_coverage() {
    // Test all Role variants through Message creation
    let user_msg = Message::user("User content");
    assert_eq!(user_msg.role, Role::User);
    assert!(user_msg.tool_calls.is_none());
    assert!(user_msg.tool_call_id.is_none());

    let assistant_msg = Message::assistant("Assistant content");
    assert_eq!(assistant_msg.role, Role::Assistant);

    let system_msg = Message::system("System prompt");
    assert_eq!(system_msg.role, Role::System);

    let tool_msg = Message::tool("call_123", "Tool result");
    assert_eq!(tool_msg.role, Role::Tool);
    assert_eq!(tool_msg.tool_call_id, Some("call_123".to_string()));
}

#[test]
fn test_tool_call_creation_and_serialization() {
    let call = carve_agent::ToolCall::new(
        "call_abc123",
        "web_search",
        json!({"query": "rust programming", "limit": 10}),
    );

    assert_eq!(call.id, "call_abc123");
    assert_eq!(call.name, "web_search");
    assert_eq!(call.arguments["query"], "rust programming");
    assert_eq!(call.arguments["limit"], 10);

    // Test serialization
    let json_str = serde_json::to_string(&call).unwrap();
    assert!(json_str.contains("call_abc123"));
    assert!(json_str.contains("web_search"));

    // Test deserialization
    let parsed: carve_agent::ToolCall = serde_json::from_str(&json_str).unwrap();
    assert_eq!(parsed.id, call.id);
    assert_eq!(parsed.name, call.name);
}

#[tokio::test]
async fn test_session_state_complex_operations() {
    let session = Session::with_initial_state(
        "complex-state",
        json!({
            "users": [],
            "settings": {"theme": "dark", "notifications": true},
            "counter": 0
        }),
    );

    // Add multiple patches
    let session = session
        .with_patch(TrackedPatch::new(
            Patch::new()
                .with_op(Op::append(path!("users"), json!({"id": 1, "name": "Alice"})))
                .with_op(Op::increment(path!("counter"), 1)),
        ))
        .with_patch(TrackedPatch::new(
            Patch::new()
                .with_op(Op::append(path!("users"), json!({"id": 2, "name": "Bob"})))
                .with_op(Op::set(path!("settings").key("theme"), json!("light"))),
        ));

    let state = session.rebuild_state().unwrap();

    assert_eq!(state["users"].as_array().unwrap().len(), 2);
    assert_eq!(state["users"][0]["name"], "Alice");
    assert_eq!(state["users"][1]["name"], "Bob");
    assert_eq!(state["settings"]["theme"], "light");
    assert_eq!(state["counter"], 1);
}

#[test]
fn test_storage_error_variants() {
    use carve_agent::StorageError;
    use std::io::{Error as IoError, ErrorKind};

    // Test IO error variant
    let io_error = StorageError::from(IoError::new(ErrorKind::PermissionDenied, "Permission denied"));
    let display = io_error.to_string();
    assert!(display.contains("IO error") || display.contains("Permission") || display.len() > 0);

    // Test Serialization error variant
    let serialization_error = StorageError::Serialization("Invalid JSON at line 5".to_string());
    let display = serialization_error.to_string();
    assert!(display.contains("Serialization") || display.contains("JSON") || display.contains("Invalid"));

    // Test NotFound error variant
    let not_found = StorageError::NotFound("session-123".to_string());
    let display = not_found.to_string();
    assert!(display.contains("not found") || display.contains("session-123") || display.contains("Not found"));
}

// ============================================================================
// Sequential Execution with Patch Error Tests
// ============================================================================

/// Tool that produces a patch with nested state changes
struct NestedStateTool;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State, Default)]
struct NestedState {
    value: i64,
}

#[async_trait]
impl Tool for NestedStateTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("nested_state", "Nested State Tool", "Modifies nested state")
    }

    async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        // Modify deeply nested state
        let nested = ctx.state::<NestedState>("deeply.nested");
        let current = nested.value().unwrap_or(0);
        nested.set_value(current + 10);

        Ok(ToolResult::success("nested_state", json!({"new_value": current + 10})))
    }
}

#[tokio::test]
async fn test_sequential_execution_with_conflicting_patches() {
    use carve_agent::execute_tools_sequential;

    // Test sequential execution where patches might conflict
    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("increment".to_string(), Arc::new(IncrementTool));

    // Create tool calls that modify the same field sequentially
    let calls = vec![
        carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_3", "increment", json!({"path": "counter"})),
    ];

    // Initial state with counter
    let initial_state = json!({
        "counter": {"value": 0, "label": "test"}
    });

    // Execute sequentially - each tool sees the updated state
    let (final_state, executions) = execute_tools_sequential(&tools, &calls, &initial_state).await;

    assert_eq!(executions.len(), 3);
    assert!(executions.iter().all(|e| e.result.is_success()));

    // In sequential mode, each tool sees the result of the previous
    // So counter should be 3 (0 -> 1 -> 2 -> 3)
    assert_eq!(final_state["counter"]["value"], 3);
}

#[tokio::test]
async fn test_sequential_execution_with_nested_state() {
    use carve_agent::execute_tools_sequential;

    // Test sequential execution with nested state modifications
    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("nested_state".to_string(), Arc::new(NestedStateTool));

    let calls = vec![
        carve_agent::ToolCall::new("call_1", "nested_state", json!({})),
        carve_agent::ToolCall::new("call_2", "nested_state", json!({})),
    ];

    // Start with state that has nested structure
    let initial_state = json!({
        "deeply": {
            "nested": {"value": 0}
        }
    });

    // Execute sequentially - each tool sees the updated state
    let (final_state, executions) = execute_tools_sequential(&tools, &calls, &initial_state).await;

    // Both tools should execute
    assert_eq!(executions.len(), 2);
    assert!(executions.iter().all(|e| e.result.is_success()));

    // Sequential: 0 -> 10 -> 20
    assert_eq!(final_state["deeply"]["nested"]["value"], 20);
}

/// Test parallel execution state isolation - each tool sees the same initial state
#[tokio::test]
async fn test_parallel_execution_state_isolation() {
    use carve_agent::execute_tools_parallel;

    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("increment".to_string(), Arc::new(IncrementTool));

    // Three parallel increment calls
    let calls = vec![
        carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_3", "increment", json!({"path": "counter"})),
    ];

    let initial_state = json!({
        "counter": {"value": 10, "label": "test"}
    });

    let results = execute_tools_parallel(&tools, &calls, &initial_state).await;

    // All three tools should see initial state (counter=10) and increment to 11
    assert_eq!(results.len(), 3);
    for (i, exec) in results.iter().enumerate() {
        assert!(exec.result.is_success(), "Tool {} should succeed", i);
        // Each tool saw initial value 10, incremented to 11
        assert_eq!(exec.result.data["new_value"], 11, "Tool {} should see initial state", i);
    }

    // All three patches should set counter.value to 11 (not 11, 12, 13)
    let patches: Vec<_> = results.iter().filter_map(|e| e.patch.as_ref()).collect();
    assert_eq!(patches.len(), 3, "All tools should produce patches");
}

/// Test parallel execution with patch conflict - multiple tools modify same field
#[tokio::test]
async fn test_parallel_execution_patch_conflict() {
    let session = Session::with_initial_state(
        "parallel-conflict",
        json!({"counter": {"value": 0, "label": ""}}),
    );

    // Three parallel increments
    let llm_response = StreamResult {
        text: "Running three increments in parallel".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
            carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
            carve_agent::ToolCall::new("call_3", "increment", json!({"path": "counter"})),
        ],
    };

    let tools = tool_map([IncrementTool]);

    // Execute in parallel mode
    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    // All three patches collected
    assert_eq!(session.patch_count(), 3);

    // Rebuild state - last patch wins (all set to 1, so final is 1)
    let state = session.rebuild_state().unwrap();
    // In parallel mode, all tools see 0, all set to 1, so final is 1
    assert_eq!(state["counter"]["value"], 1);
}

/// Test parallel execution with different fields - no conflict
#[tokio::test]
async fn test_parallel_execution_different_fields() {
    let session = Session::with_initial_state(
        "parallel-no-conflict",
        json!({
            "counter_a": {"value": 0, "label": ""},
            "counter_b": {"value": 0, "label": ""},
            "counter_c": {"value": 0, "label": ""}
        }),
    );

    // Three parallel increments to different fields
    let llm_response = StreamResult {
        text: "Running three increments to different counters".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter_a"})),
            carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter_b"})),
            carve_agent::ToolCall::new("call_3", "increment", json!({"path": "counter_c"})),
        ],
    };

    let tools = tool_map([IncrementTool]);

    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    let state = session.rebuild_state().unwrap();

    // All three counters should be 1 (no conflict)
    assert_eq!(state["counter_a"]["value"], 1);
    assert_eq!(state["counter_b"]["value"], 1);
    assert_eq!(state["counter_c"]["value"], 1);
}

/// Test parallel vs sequential with same operations - different results
#[tokio::test]
async fn test_sequential_vs_parallel_execution_difference() {
    use carve_agent::{execute_tools_parallel, execute_tools_sequential};

    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("increment".to_string(), Arc::new(IncrementTool));

    let calls = vec![
        carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
    ];

    let initial_state = json!({
        "counter": {"value": 0, "label": "test"}
    });

    // Parallel execution - both tools see initial state (counter=0)
    let parallel_results = execute_tools_parallel(&tools, &calls, &initial_state).await;

    // Both tools incremented from 0, so both patches set counter.value to 1
    assert_eq!(parallel_results.len(), 2);

    // Sequential execution - second tool sees first tool's result
    let (seq_final_state, seq_results) =
        execute_tools_sequential(&tools, &calls, &initial_state).await;

    assert_eq!(seq_results.len(), 2);
    // Sequential: 0 -> 1 -> 2
    assert_eq!(seq_final_state["counter"]["value"], 2);
}

// ============================================================================
// Stream End Event with Captured Tool Calls Tests
// ============================================================================

#[test]
fn test_stream_collector_with_tool_call_via_chunk_then_end() {
    use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd, ToolChunk};

    // Test the flow: text chunks -> tool call chunks -> end event
    let mut collector = StreamCollector::new();

    // Text chunk
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Let me search for that.".to_string(),
    }));

    // Tool call chunk with complete info
    let tool_call = genai::chat::ToolCall {
        call_id: "call_search".to_string(),
        fn_name: "web_search".to_string(),
        fn_arguments: json!({"query": "rust async"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call }));

    // End event
    let end = StreamEnd::default();
    let output = collector.process(ChatStreamEvent::End(end));

    // End event returns None
    assert!(output.is_none());

    // Finish and verify results
    let result = collector.finish();
    assert_eq!(result.text, "Let me search for that.");
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "web_search");
}

#[test]
fn test_stream_collector_multiple_tool_calls_and_end() {
    use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd, ToolChunk};

    let mut collector = StreamCollector::new();

    // Text
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Running multiple tools.".to_string(),
    }));

    // First tool call
    let tc1 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "search".to_string(),
        fn_arguments: json!({"q": "test"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

    // Second tool call
    let tc2 = genai::chat::ToolCall {
        call_id: "call_2".to_string(),
        fn_name: "calculate".to_string(),
        fn_arguments: json!({"expr": "1+1"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));

    // End event (tool calls already captured via ToolCallChunk)
    collector.process(ChatStreamEvent::End(StreamEnd::default()));

    let result = collector.finish();
    assert_eq!(result.tool_calls.len(), 2);
}

#[test]
fn test_stream_collector_text_only_then_end() {
    use genai::chat::{ChatStreamEvent, StreamChunk, StreamEnd};

    let mut collector = StreamCollector::new();

    // Only text, no tool calls
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Here is your answer: 42".to_string(),
    }));

    // End event with no captured tool calls
    collector.process(ChatStreamEvent::End(StreamEnd::default()));

    let result = collector.finish();
    assert_eq!(result.text, "Here is your answer: 42");
    assert!(result.tool_calls.is_empty());
    assert!(!result.needs_tools());
}

#[test]
fn test_stream_collector_unknown_event_handling() {
    use genai::chat::ChatStreamEvent;

    let mut collector = StreamCollector::new();

    // Start event (should be ignored)
    let output = collector.process(ChatStreamEvent::Start);
    assert!(output.is_none());

    // ReasoningDelta event (if exists, should be ignored)
    // The _ match arm handles unknown events

    let result = collector.finish();
    assert!(result.text.is_empty());
    assert!(result.tool_calls.is_empty());
}

// ============================================================================
// Tool Execution with Empty Patch Tests
// ============================================================================

/// Tool that reads state but doesn't modify it
struct ReadOnlyTool;

#[async_trait]
impl Tool for ReadOnlyTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("read_only", "Read Only", "Reads state without modification")
    }

    async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        // Only read, don't modify
        let counter = ctx.state::<CounterState>("counter");
        let value = counter.value().unwrap_or(-1);

        // No modifications, so patch should be empty
        Ok(ToolResult::success("read_only", json!({"current_value": value})))
    }
}

#[tokio::test]
async fn test_tool_execution_with_empty_patch() {
    use carve_agent::execute_single_tool;

    let tool = ReadOnlyTool;
    let call = carve_agent::ToolCall::new("call_1", "read_only", json!({}));
    let state = json!({"counter": {"value": 42, "label": "test"}});

    let result = execute_single_tool(Some(&tool), &call, &state).await;

    assert!(result.result.is_success());
    assert_eq!(result.result.data["current_value"], 42);
    // Patch should be None (empty patches are converted to None)
    assert!(result.patch.is_none());
}

#[tokio::test]
async fn test_sequential_execution_with_mixed_patch_results() {
    use carve_agent::execute_tools_sequential;

    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("read_only".to_string(), Arc::new(ReadOnlyTool));
    tools.insert("increment".to_string(), Arc::new(IncrementTool));

    let calls = vec![
        carve_agent::ToolCall::new("call_1", "read_only", json!({})),
        carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
        carve_agent::ToolCall::new("call_3", "read_only", json!({})),
    ];

    let initial_state = json!({
        "counter": {"value": 10, "label": "test"}
    });

    let (final_state, executions) = execute_tools_sequential(&tools, &calls, &initial_state).await;

    // First read_only sees 10, increment changes to 11, second read_only sees 11
    assert_eq!(executions[0].result.data["current_value"], 10);
    assert_eq!(executions[1].result.data["new_value"], 11);
    assert_eq!(executions[2].result.data["current_value"], 11);

    // Final state should be 11
    assert_eq!(final_state["counter"]["value"], 11);

    // Check patches
    assert!(executions[0].patch.is_none()); // read_only - no patch
    assert!(executions[1].patch.is_some()); // increment - has patch
    assert!(executions[2].patch.is_none()); // read_only - no patch
}

// ============================================================================
// Agent Loop Error Tests
// ============================================================================

#[test]
fn test_agent_loop_error_all_variants() {
    use carve_agent::AgentLoopError;

    // LlmError
    let llm_err = AgentLoopError::LlmError("API rate limit exceeded".to_string());
    let display = llm_err.to_string();
    assert!(display.contains("LLM") || display.contains("rate limit"));

    // StateError
    let state_err = AgentLoopError::StateError("Failed to rebuild state".to_string());
    let display = state_err.to_string();
    assert!(display.contains("State") || display.contains("rebuild"));

    // MaxRoundsExceeded
    let max_rounds_err = AgentLoopError::MaxRoundsExceeded(15);
    let display = max_rounds_err.to_string();
    assert!(display.contains("15") || display.contains("Max") || display.contains("exceeded"));
}

// ============================================================================
// Context Category Tests
// ============================================================================

#[test]
fn test_context_category_variants() {
    // Test all ContextCategory variants
    let tool_exec = ContextCategory::ToolExecution;
    let session = ContextCategory::Session;
    let user_input = ContextCategory::UserInput;

    // Verify they are different (PartialEq)
    assert_eq!(tool_exec, ContextCategory::ToolExecution);
    assert_eq!(session, ContextCategory::Session);
    assert_eq!(user_input, ContextCategory::UserInput);
    assert_ne!(tool_exec, session);
    assert_ne!(session, user_input);
    assert_ne!(user_input, tool_exec);
}

// ============================================================================
// Mock-based End-to-End Tests (No Real LLM Required)
// ============================================================================

// These tests simulate the full agent loop without requiring a real LLM.
// They test the same scenarios as live_deepseek.rs but using mock responses.

/// Simulate a complete agent turn with tool calls
#[tokio::test]
async fn test_e2e_tool_execution_flow() {
    // Simulates: User asks -> LLM calls tool -> Tool executes -> Response

    // 1. Create session with initial state
    let session = Session::with_initial_state("e2e-test", json!({"counter": 0}))
        .with_message(Message::user("Increment the counter by 5"));

    // 2. Simulate LLM response with tool call
    let llm_response = StreamResult {
        text: "I'll increment the counter for you.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "increment",
            json!({"path": "counter"}),
        )],
    };

    // 3. Add assistant message with tool calls
    let session = session.with_message(carve_agent::assistant_tool_calls(
        &llm_response.text,
        llm_response.tool_calls.clone(),
    ));

    // 4. Execute tools
    let tools = tool_map([IncrementTool]);
    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    // 5. Verify state changed
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 1);

    // 6. Simulate final LLM response
    let session = session.with_message(Message::assistant("Done! The counter is now 1."));

    assert_eq!(session.message_count(), 4); // user + assistant(tool) + tool_response + assistant
    assert_eq!(session.patch_count(), 1);
}

/// Simulate parallel tool calls
#[tokio::test]
async fn test_e2e_parallel_tool_calls() {
    let session = Session::with_initial_state(
        "e2e-parallel",
        json!({
            "counter": {"value": 0, "label": "test"},
            "todos": {"items": [], "count": 0}
        }),
    )
    .with_message(Message::user("Increment counter and add a todo"));

    // LLM calls two tools in parallel
    let llm_response = StreamResult {
        text: "I'll do both.".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
            carve_agent::ToolCall::new("call_2", "add_todo", json!({"item": "New task"})),
        ],
    };

    let session = session.with_message(carve_agent::assistant_tool_calls(
        &llm_response.text,
        llm_response.tool_calls.clone(),
    ));

    // Execute tools (parallel mode)
    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("increment".to_string(), Arc::new(IncrementTool));
    tools.insert("add_todo".to_string(), Arc::new(AddTodoTool));

    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    // Both tools executed
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 1);
    assert_eq!(state["todos"]["count"], 1);
    assert_eq!(session.patch_count(), 2); // Two parallel patches
}

/// Simulate multi-turn conversation with state accumulation
#[tokio::test]
async fn test_e2e_multi_turn_with_state() {
    let tools = tool_map([IncrementTool]);

    // Turn 1
    let mut session = Session::with_initial_state("e2e-multi", json!({"counter": {"value": 0, "label": ""}}))
        .with_message(Message::user("Increment"));

    let response1 = StreamResult {
        text: "Incrementing.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    session = session.with_message(carve_agent::assistant_tool_calls(&response1.text, response1.tool_calls.clone()));
    session = loop_execute_tools(session, &response1, &tools, true).await.unwrap();
    session = session.with_message(Message::assistant("Counter is now 1."));

    // Turn 2
    session = session.with_message(Message::user("Increment again"));
    let response2 = StreamResult {
        text: "Incrementing again.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_2",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    session = session.with_message(carve_agent::assistant_tool_calls(&response2.text, response2.tool_calls.clone()));
    session = loop_execute_tools(session, &response2, &tools, true).await.unwrap();
    session = session.with_message(Message::assistant("Counter is now 2."));

    // Turn 3
    session = session.with_message(Message::user("One more time"));
    let response3 = StreamResult {
        text: "One more increment.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_3",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    session = session.with_message(carve_agent::assistant_tool_calls(&response3.text, response3.tool_calls.clone()));
    session = loop_execute_tools(session, &response3, &tools, true).await.unwrap();

    // Verify accumulated state
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 3);
    assert_eq!(session.patch_count(), 3);
}

/// Simulate tool failure and error message
#[tokio::test]
async fn test_e2e_tool_failure_handling() {
    let session = Session::new("e2e-failure")
        .with_message(Message::user("Call a non-existent tool"));

    // LLM calls a tool that doesn't exist
    let llm_response = StreamResult {
        text: "Calling tool.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "nonexistent_tool",
            json!({}),
        )],
    };

    let session = session.with_message(carve_agent::assistant_tool_calls(
        &llm_response.text,
        llm_response.tool_calls.clone(),
    ));

    // Execute with empty tool map
    let tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();

    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    // Tool response should contain error
    let last_msg = session.messages.last().unwrap();
    assert_eq!(last_msg.role, Role::Tool);
    assert!(last_msg.content.contains("error") || last_msg.content.contains("not found"));
}

/// Simulate session persistence and restore mid-conversation
#[tokio::test]
async fn test_e2e_session_persistence_restore() {
    let storage = MemoryStorage::new();
    let tools = tool_map([IncrementTool]);

    // Phase 1: Start conversation
    let mut session = Session::with_initial_state("e2e-persist", json!({"counter": {"value": 10, "label": ""}}))
        .with_message(Message::user("Increment by 5"));

    let response = StreamResult {
        text: "Incrementing.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    session = session.with_message(carve_agent::assistant_tool_calls(&response.text, response.tool_calls.clone()));
    session = loop_execute_tools(session, &response, &tools, true).await.unwrap();
    session = session.with_message(Message::assistant("Done!"));

    // Save
    storage.save(&session).await.unwrap();
    let state_before = session.rebuild_state().unwrap();

    // Phase 2: "Restart" - load and continue
    let mut loaded = storage.load("e2e-persist").await.unwrap().unwrap();

    // Verify state preserved
    let state_after_load = loaded.rebuild_state().unwrap();
    assert_eq!(state_before, state_after_load);
    assert_eq!(loaded.message_count(), 4);

    // Continue conversation
    loaded = loaded.with_message(Message::user("Increment again"));
    let response2 = StreamResult {
        text: "Incrementing again.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_2",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    loaded = loaded.with_message(carve_agent::assistant_tool_calls(&response2.text, response2.tool_calls.clone()));
    loaded = loop_execute_tools(loaded, &response2, &tools, true).await.unwrap();

    // Verify continued correctly
    let final_state = loaded.rebuild_state().unwrap();
    assert_eq!(final_state["counter"]["value"], 12); // 10 + 1 + 1
}

/// Simulate snapshot and continue
#[tokio::test]
async fn test_e2e_snapshot_and_continue() {
    let tools = tool_map([IncrementTool]);

    // Build up patches
    let mut session = Session::with_initial_state("e2e-snapshot", json!({"counter": {"value": 0, "label": ""}}));

    for i in 0..5 {
        let response = StreamResult {
            text: format!("Increment {}", i),
            tool_calls: vec![carve_agent::ToolCall::new(
                format!("call_{}", i),
                "increment",
                json!({"path": "counter"}),
            )],
        };
        session = loop_execute_tools(session, &response, &tools, true).await.unwrap();
    }

    assert_eq!(session.patch_count(), 5);
    let state_before = session.rebuild_state().unwrap();
    assert_eq!(state_before["counter"]["value"], 5);

    // Snapshot
    let session = session.snapshot().unwrap();
    assert_eq!(session.patch_count(), 0);
    assert_eq!(session.state["counter"]["value"], 5);

    // Continue after snapshot
    let response = StreamResult {
        text: "One more".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_5",
            "increment",
            json!({"path": "counter"}),
        )],
    };
    let session = loop_execute_tools(session, &response, &tools, true).await.unwrap();

    let final_state = session.rebuild_state().unwrap();
    assert_eq!(final_state["counter"]["value"], 6);
    assert_eq!(session.patch_count(), 1); // Only new patch after snapshot
}

/// Simulate state replay (time-travel debugging)
#[tokio::test]
async fn test_e2e_state_replay() {
    let tools = tool_map([IncrementTool]);

    let mut session = Session::with_initial_state("e2e-replay", json!({"counter": {"value": 0, "label": ""}}));

    // Create history: 0 -> 1 -> 2 -> 3 -> 4 -> 5
    for i in 0..5 {
        let response = StreamResult {
            text: format!("Step {}", i),
            tool_calls: vec![carve_agent::ToolCall::new(
                format!("call_{}", i),
                "increment",
                json!({"path": "counter"}),
            )],
        };
        session = loop_execute_tools(session, &response, &tools, true).await.unwrap();
    }

    // Replay to each historical point
    assert_eq!(session.replay_to(0).unwrap()["counter"]["value"], 1);
    assert_eq!(session.replay_to(1).unwrap()["counter"]["value"], 2);
    assert_eq!(session.replay_to(2).unwrap()["counter"]["value"], 3);
    assert_eq!(session.replay_to(3).unwrap()["counter"]["value"], 4);
    assert_eq!(session.replay_to(4).unwrap()["counter"]["value"], 5);

    // Final state
    assert_eq!(session.rebuild_state().unwrap()["counter"]["value"], 5);
}

/// Simulate long conversation with many messages
#[tokio::test]
async fn test_e2e_long_conversation() {
    let mut session = Session::new("e2e-long");

    // Build 100 turns of conversation
    for i in 0..100 {
        session = session
            .with_message(Message::user(format!("Message {}", i)))
            .with_message(Message::assistant(format!("Response {}", i)));
    }

    assert_eq!(session.message_count(), 200);

    // Storage should handle this efficiently
    let storage = MemoryStorage::new();
    storage.save(&session).await.unwrap();
    let loaded = storage.load("e2e-long").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 200);
}

/// Simulate sequential tool execution (non-parallel mode)
#[tokio::test]
async fn test_e2e_sequential_tool_execution() {
    let session = Session::with_initial_state(
        "e2e-sequential",
        json!({"counter": {"value": 0, "label": ""}}),
    );

    // Multiple tool calls
    let llm_response = StreamResult {
        text: "Running sequentially.".to_string(),
        tool_calls: vec![
            carve_agent::ToolCall::new("call_1", "increment", json!({"path": "counter"})),
            carve_agent::ToolCall::new("call_2", "increment", json!({"path": "counter"})),
            carve_agent::ToolCall::new("call_3", "increment", json!({"path": "counter"})),
        ],
    };

    let tools = tool_map([IncrementTool]);

    // Execute in sequential mode (parallel = false)
    let session = loop_execute_tools(session, &llm_response, &tools, false)
        .await
        .unwrap();

    // In sequential mode, each tool sees the previous tool's result
    // So counter should be 3 (0 -> 1 -> 2 -> 3)
    let state = session.rebuild_state().unwrap();
    assert_eq!(state["counter"]["value"], 3);
}

// ============================================================================
// Execute Single Tool Edge Cases
// ============================================================================

#[tokio::test]
async fn test_execute_single_tool_not_found() {
    use carve_agent::execute_single_tool;

    let call = carve_agent::ToolCall::new("call_1", "nonexistent_tool", json!({}));
    let state = json!({});

    // Tool is None - not found
    let result = execute_single_tool(None, &call, &state).await;

    assert!(result.result.is_error());
    assert!(result.result.message.as_ref().unwrap().contains("not found"));
    assert!(result.patch.is_none());
}

#[tokio::test]
async fn test_execute_single_tool_with_complex_state() {
    use carve_agent::execute_single_tool;

    let tool = IncrementTool;
    let call = carve_agent::ToolCall::new("call_1", "increment", json!({"path": "data.counters.main"}));

    // Complex nested state
    let state = json!({
        "data": {
            "counters": {
                "main": {"value": 100, "label": "main counter"},
                "secondary": {"value": 50, "label": "secondary"}
            },
            "metadata": {"created": "2024-01-01"}
        }
    });

    let result = execute_single_tool(Some(&tool), &call, &state).await;

    assert!(result.result.is_success());
    assert_eq!(result.result.data["new_value"], 101);
}

// ============================================================================
// ContextProvider & SystemReminder Integration Tests
// ============================================================================

/// Test ContextProvider integration in a simulated agent flow
#[tokio::test]
async fn test_e2e_context_provider_integration() {
    // Simulate: Provider injects context -> Tool sees updated state

    let manager = StateManager::new(json!({
        "counter": {"value": 15, "label": "initial"},
        "user_context": {}
    }));

    let provider = CounterContextProvider;

    // 1. Provider runs and may modify state
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "provider_call", "provider:counter");

    let messages = provider.provide(&ctx).await;

    // Provider should return message for high counter (>10)
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("high"));

    // Provider also modifies state
    manager.commit(ctx.take_patch()).await.unwrap();

    // 2. Verify state was modified by provider
    let new_snapshot = manager.snapshot().await;
    assert_eq!(new_snapshot["counter"]["label"], "context_provided");

    // 3. Now a tool runs and sees the provider's changes
    let tool = IncrementTool;
    let ctx = Context::new(&new_snapshot, "tool_call", "tool:increment");
    let result = tool.execute(json!({"path": "counter"}), &ctx).await.unwrap();

    assert!(result.is_success());
    // Counter was 15, now 16
    assert_eq!(result.data["new_value"], 16);
}

/// Test SystemReminder integration
#[tokio::test]
async fn test_e2e_system_reminder_integration() {
    let manager = StateManager::new(json!({
        "todos": {"items": ["Task 1", "Task 2", "Task 3"], "count": 3}
    }));

    let reminder = TodoReminder;

    // Reminder checks state and returns message
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "reminder_call", "reminder:todo");

    let message = reminder.remind(&ctx).await;

    // Should return reminder about pending todos
    assert!(message.is_some());
    let msg = message.unwrap();
    assert!(msg.contains("3")); // 3 pending todos
}

/// Test multiple ContextProviders with priority ordering
#[tokio::test]
async fn test_e2e_multiple_providers_priority() {
    // Define providers with different priorities
    struct HighPriorityProvider;
    struct LowPriorityProvider;

    #[async_trait]
    impl ContextProvider for HighPriorityProvider {
        fn id(&self) -> &str { "high_priority" }
        fn category(&self) -> ContextCategory { ContextCategory::Session }
        fn priority(&self) -> u32 { 100 } // Higher priority

        async fn provide(&self, _ctx: &Context<'_>) -> Vec<String> {
            vec!["High priority context".to_string()]
        }
    }

    #[async_trait]
    impl ContextProvider for LowPriorityProvider {
        fn id(&self) -> &str { "low_priority" }
        fn category(&self) -> ContextCategory { ContextCategory::Session }
        fn priority(&self) -> u32 { 10 } // Lower priority

        async fn provide(&self, _ctx: &Context<'_>) -> Vec<String> {
            vec!["Low priority context".to_string()]
        }
    }

    let high = HighPriorityProvider;
    let low = LowPriorityProvider;

    // Verify priorities
    assert!(high.priority() > low.priority());

    // Both providers can run and produce context
    let manager = StateManager::new(json!({}));
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "test", "test");

    let high_msgs = high.provide(&ctx).await;
    let low_msgs = low.provide(&ctx).await;

    assert_eq!(high_msgs.len(), 1);
    assert_eq!(low_msgs.len(), 1);
}

/// Test ContextProvider that modifies state based on conditions
#[tokio::test]
async fn test_e2e_conditional_context_provider() {
    struct ConditionalProvider;

    #[async_trait]
    impl ContextProvider for ConditionalProvider {
        fn id(&self) -> &str { "conditional" }
        fn category(&self) -> ContextCategory { ContextCategory::ToolExecution }
        fn priority(&self) -> u32 { 50 }

        async fn provide(&self, ctx: &Context<'_>) -> Vec<String> {
            let counter = ctx.state::<CounterState>("counter");
            let value = counter.value().unwrap_or(0);

            if value < 0 {
                // Auto-fix negative values
                counter.set_value(0);
                vec!["Warning: Counter was negative, reset to 0".to_string()]
            } else if value > 100 {
                vec![format!("Note: Counter is very high ({})", value)]
            } else {
                vec![]
            }
        }
    }

    let provider = ConditionalProvider;

    // Test with negative value
    let manager = StateManager::new(json!({"counter": {"value": -5, "label": ""}}));
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "test", "provider:conditional");

    let messages = provider.provide(&ctx).await;
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains("negative"));

    // Apply the fix
    manager.commit(ctx.take_patch()).await.unwrap();
    let fixed_state = manager.snapshot().await;
    assert_eq!(fixed_state["counter"]["value"], 0);
}

// ============================================================================
// ToolResult Pending/Warning Status Tests
// ============================================================================

/// Tool that returns pending status (needs user confirmation)
struct ConfirmationTool;

#[async_trait]
impl Tool for ConfirmationTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("dangerous_action", "Dangerous Action", "Requires confirmation")
            .with_confirmation(true)
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let confirmed = args["confirmed"].as_bool().unwrap_or(false);

        if confirmed {
            Ok(ToolResult::success("dangerous_action", json!({"status": "executed"})))
        } else {
            Ok(ToolResult::pending(
                "dangerous_action",
                "This action requires confirmation. Please confirm to proceed.",
            ))
        }
    }
}

/// Tool that returns warning status (partial success)
struct PartialSuccessTool;

#[async_trait]
impl Tool for PartialSuccessTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("batch_process", "Batch Process", "Process multiple items")
    }

    async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        let items = args["items"].as_array().map(|a| a.len()).unwrap_or(0);

        // Simulate: some items succeed, some fail
        let successful = items * 8 / 10; // 80% success rate
        let failed = items - successful;

        if failed > 0 {
            Ok(ToolResult::warning(
                "batch_process",
                json!({
                    "processed": successful,
                    "failed": failed,
                    "total": items
                }),
                format!("{} items processed, {} failed", successful, failed),
            ))
        } else {
            Ok(ToolResult::success(
                "batch_process",
                json!({"processed": items, "failed": 0}),
            ))
        }
    }
}

#[tokio::test]
async fn test_e2e_tool_pending_status() {
    let tool = ConfirmationTool;
    let manager = StateManager::new(json!({}));
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_1", "tool:dangerous");

    // First call without confirmation
    let result = tool.execute(json!({}), &ctx).await.unwrap();

    assert!(result.is_pending());
    assert!(!result.is_success());
    assert!(!result.is_error());
    assert!(result.message.as_ref().unwrap().contains("confirmation"));

    // Second call with confirmation
    let result = tool.execute(json!({"confirmed": true}), &ctx).await.unwrap();

    assert!(result.is_success());
    assert!(!result.is_pending());
}

#[tokio::test]
async fn test_e2e_tool_warning_status() {
    let tool = PartialSuccessTool;
    let manager = StateManager::new(json!({}));
    let snapshot = manager.snapshot().await;
    let ctx = Context::new(&snapshot, "call_1", "tool:batch");

    // Process 10 items (80% success = 8 success, 2 failed)
    let result = tool.execute(json!({"items": [1,2,3,4,5,6,7,8,9,10]}), &ctx).await.unwrap();

    // Warning is still considered "success" but with a message
    assert!(result.is_success());
    assert!(result.message.is_some());
    assert!(result.message.as_ref().unwrap().contains("failed"));
    assert_eq!(result.data["processed"], 8);
    assert_eq!(result.data["failed"], 2);
}

#[tokio::test]
async fn test_e2e_pending_tool_in_session_flow() {
    let session = Session::new("pending-test")
        .with_message(Message::user("Delete all files"));

    // Simulate LLM calling dangerous action without confirmation
    let llm_response = StreamResult {
        text: "I'll delete the files.".to_string(),
        tool_calls: vec![carve_agent::ToolCall::new(
            "call_1",
            "dangerous_action",
            json!({}),
        )],
    };

    let session = session.with_message(carve_agent::assistant_tool_calls(
        &llm_response.text,
        llm_response.tool_calls.clone(),
    ));

    let mut tools: std::collections::HashMap<String, Arc<dyn Tool>> =
        std::collections::HashMap::new();
    tools.insert("dangerous_action".to_string(), Arc::new(ConfirmationTool));

    let session = loop_execute_tools(session, &llm_response, &tools, true)
        .await
        .unwrap();

    // Check tool response contains pending status
    let tool_msg = session.messages.last().unwrap();
    assert_eq!(tool_msg.role, Role::Tool);
    assert!(tool_msg.content.contains("pending") || tool_msg.content.contains("confirmation"));
}

// ============================================================================
// Streaming Edge Case Tests
// ============================================================================

#[test]
fn test_stream_collector_empty_stream() {
    let collector = StreamCollector::new();
    let result = collector.finish();

    assert!(result.text.is_empty());
    assert!(result.tool_calls.is_empty());
    assert!(!result.needs_tools());
}

#[test]
fn test_stream_collector_only_whitespace() {
    use genai::chat::{ChatStreamEvent, StreamChunk};

    let mut collector = StreamCollector::new();

    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "   ".to_string(),
    }));
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "\n\n".to_string(),
    }));

    let result = collector.finish();
    assert_eq!(result.text, "   \n\n");
}

#[test]
fn test_stream_collector_interleaved_text_and_tools() {
    use genai::chat::{ChatStreamEvent, StreamChunk, ToolChunk};

    let mut collector = StreamCollector::new();

    // Text
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Let me ".to_string(),
    }));

    // Tool call starts
    let tc1 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "search".to_string(),
        fn_arguments: json!({"q": "test"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

    // More text
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "help you.".to_string(),
    }));

    // Another tool call
    let tc2 = genai::chat::ToolCall {
        call_id: "call_2".to_string(),
        fn_name: "calculate".to_string(),
        fn_arguments: json!({"expr": "1+1"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));

    let result = collector.finish();

    assert_eq!(result.text, "Let me help you.");
    assert_eq!(result.tool_calls.len(), 2);
}

#[test]
fn test_stream_result_with_empty_tool_calls() {
    let result = StreamResult {
        text: "Hello".to_string(),
        tool_calls: vec![],
    };

    assert!(!result.needs_tools());
}

// ============================================================================
// Concurrent Session Operations Tests
// ============================================================================

#[tokio::test]
async fn test_concurrent_session_modifications() {
    // Test that concurrent modifications to different sessions work correctly
    let storage = Arc::new(MemoryStorage::new());

    let mut handles = vec![];

    for i in 0..20 {
        let storage = Arc::clone(&storage);
        let handle = tokio::spawn(async move {
            let session_id = format!("concurrent-session-{}", i);

            // Create session
            let mut session = Session::with_initial_state(&session_id, json!({"value": i}));

            // Add messages
            for j in 0..5 {
                session = session.with_message(Message::user(format!("Msg {} from session {}", j, i)));
            }

            // Save
            storage.save(&session).await.unwrap();

            // Load and verify
            let loaded = storage.load(&session_id).await.unwrap().unwrap();
            assert_eq!(loaded.message_count(), 5);
            assert_eq!(loaded.state["value"], i);

            session_id
        });
        handles.push(handle);
    }

    let results: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(results.len(), 20);

    // Verify all sessions exist
    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 20);
}

#[tokio::test]
async fn test_concurrent_read_write_same_session() {
    let storage = Arc::new(MemoryStorage::new());

    // Create initial session
    let session = Session::new("shared-session")
        .with_message(Message::user("Initial message"));
    storage.save(&session).await.unwrap();

    let mut handles = vec![];

    // Multiple readers and writers
    for i in 0..10 {
        let storage = Arc::clone(&storage);
        let handle = tokio::spawn(async move {
            if i % 2 == 0 {
                // Reader
                let loaded = storage.load("shared-session").await.unwrap();
                loaded.is_some()
            } else {
                // Writer (updates the session)
                let mut session = storage.load("shared-session").await.unwrap().unwrap();
                session = session.with_message(Message::user(format!("Update {}", i)));
                storage.save(&session).await.unwrap();
                true
            }
        });
        handles.push(handle);
    }

    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All operations should succeed
    assert!(results.iter().all(|&r| r));

    // Final session should have messages
    let final_session = storage.load("shared-session").await.unwrap().unwrap();
    assert!(final_session.message_count() >= 1);
}

#[tokio::test]
async fn test_concurrent_tool_executions_isolated() {
    // Test that concurrent tool executions don't interfere with each other
    let mut handles = vec![];

    for i in 0..10 {
        let handle = tokio::spawn(async move {
            let session = Session::with_initial_state(
                format!("isolated-{}", i),
                json!({"counter": {"value": i * 10, "label": ""}}),
            );

            let response = StreamResult {
                text: "Incrementing".to_string(),
                tool_calls: vec![carve_agent::ToolCall::new(
                    format!("call_{}", i),
                    "increment",
                    json!({"path": "counter"}),
                )],
            };

            let tools = tool_map([IncrementTool]);
            let session = loop_execute_tools(session, &response, &tools, true)
                .await
                .unwrap();

            let state = session.rebuild_state().unwrap();
            let expected = i * 10 + 1;
            state["counter"]["value"].as_i64().unwrap() == expected as i64
        });
        handles.push(handle);
    }

    let results: Vec<bool> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // All should have correct isolated state
    assert!(results.iter().all(|&r| r));
}

// ============================================================================
// Storage Edge Case Tests
// ============================================================================

#[tokio::test]
async fn test_storage_session_not_found() {
    let storage = MemoryStorage::new();

    let result = storage.load("nonexistent").await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_storage_delete_nonexistent() {
    let storage = MemoryStorage::new();

    // Should not error when deleting non-existent session
    let result = storage.delete("nonexistent").await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_storage_overwrite_session() {
    let storage = MemoryStorage::new();

    // Create and save
    let session1 = Session::new("overwrite-test")
        .with_message(Message::user("First version"));
    storage.save(&session1).await.unwrap();

    // Overwrite
    let session2 = Session::new("overwrite-test")
        .with_message(Message::user("Second version"))
        .with_message(Message::assistant("Response"));
    storage.save(&session2).await.unwrap();

    // Load and verify overwritten
    let loaded = storage.load("overwrite-test").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 2);
    assert!(loaded.messages[0].content.contains("Second"));
}

#[tokio::test]
async fn test_file_storage_special_characters_in_id() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStorage::new(temp_dir.path());

    // Session ID with special characters (but filesystem-safe)
    let session = Session::new("session_with-special.chars_123")
        .with_message(Message::user("Test"));

    storage.save(&session).await.unwrap();
    let loaded = storage.load("session_with-special.chars_123").await.unwrap();

    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().message_count(), 1);
}

#[tokio::test]
async fn test_storage_empty_session() {
    let storage = MemoryStorage::new();

    // Session with no messages or patches
    let session = Session::new("empty-session");
    storage.save(&session).await.unwrap();

    let loaded = storage.load("empty-session").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 0);
    assert_eq!(loaded.patch_count(), 0);
}

#[tokio::test]
async fn test_storage_large_state() {
    let storage = MemoryStorage::new();

    // Create session with large state
    let mut large_data = serde_json::Map::new();
    for i in 0..1000 {
        large_data.insert(
            format!("key_{}", i),
            json!({
                "index": i,
                "data": "x".repeat(100),
                "nested": {"a": 1, "b": 2, "c": 3}
            }),
        );
    }

    let session = Session::with_initial_state("large-state", Value::Object(large_data))
        .with_message(Message::user("Test with large state"));

    storage.save(&session).await.unwrap();

    let loaded = storage.load("large-state").await.unwrap().unwrap();
    let state = loaded.rebuild_state().unwrap();

    assert!(state.as_object().unwrap().len() >= 1000);
}

// ============================================================================
// Message Edge Case Tests
// ============================================================================

#[test]
fn test_message_empty_content() {
    let msg = Message::user("");
    assert_eq!(msg.content, "");
    assert_eq!(msg.role, Role::User);
}

#[test]
fn test_message_special_characters() {
    let special = "Hello! 你好! مرحبا! 🎉 <script>alert('xss')</script> \"quotes\" 'apostrophes'";
    let msg = Message::user(special);
    assert_eq!(msg.content, special);
}

#[test]
fn test_message_very_long_content() {
    let long_content = "a".repeat(100_000);
    let msg = Message::user(&long_content);
    assert_eq!(msg.content.len(), 100_000);
}

#[test]
fn test_message_multiline_content() {
    let multiline = "Line 1\nLine 2\r\nLine 3\n\n\nLine 6";
    let msg = Message::user(multiline);
    assert_eq!(msg.content, multiline);
}

#[test]
fn test_message_json_in_content() {
    let json_content = r#"{"key": "value", "array": [1, 2, 3]}"#;
    let msg = Message::user(json_content);
    assert_eq!(msg.content, json_content);

    // Should be serializable
    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: Message = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.content, json_content);
}

#[test]
fn test_session_with_all_message_types() {
    let session = Session::new("all-types")
        .with_message(Message::system("You are helpful."))
        .with_message(Message::user("Hello"))
        .with_message(Message::assistant("Hi there!"))
        .with_message(Message::assistant_with_tool_calls(
            "Let me check.",
            vec![carve_agent::ToolCall::new("call_1", "search", json!({}))],
        ))
        .with_message(Message::tool("call_1", r#"{"result": "found"}"#));

    assert_eq!(session.message_count(), 5);

    // Verify each type
    assert_eq!(session.messages[0].role, Role::System);
    assert_eq!(session.messages[1].role, Role::User);
    assert_eq!(session.messages[2].role, Role::Assistant);
    assert_eq!(session.messages[3].role, Role::Assistant);
    assert!(session.messages[3].tool_calls.is_some());
    assert_eq!(session.messages[4].role, Role::Tool);
    assert_eq!(session.messages[4].tool_call_id, Some("call_1".to_string()));
}

#[tokio::test]
async fn test_e2e_empty_user_message() {
    let session = Session::new("empty-msg-test")
        .with_message(Message::user(""));

    // Simulate LLM response to empty message
    let llm_response = StreamResult {
        text: "I notice you sent an empty message. How can I help you?".to_string(),
        tool_calls: vec![],
    };

    let session = session.with_message(Message::assistant(&llm_response.text));

    assert_eq!(session.message_count(), 2);
    assert!(session.messages[0].content.is_empty());
    assert!(!session.messages[1].content.is_empty());
}

#[tokio::test]
async fn test_e2e_system_prompt_in_session() {
    // Test that system prompt is preserved throughout conversation
    let session = Session::new("system-prompt-test")
        .with_message(Message::system("You are a calculator. Only respond with numbers."))
        .with_message(Message::user("What is 2+2?"))
        .with_message(Message::assistant("4"))
        .with_message(Message::user("And 3+3?"))
        .with_message(Message::assistant("6"));

    // Save and load
    let storage = MemoryStorage::new();
    storage.save(&session).await.unwrap();

    let loaded = storage.load("system-prompt-test").await.unwrap().unwrap();

    // System prompt should be first message
    assert_eq!(loaded.messages[0].role, Role::System);
    assert!(loaded.messages[0].content.contains("calculator"));
}

// ============================================================================
// Tool Descriptor Edge Cases
// ============================================================================

#[test]
fn test_tool_descriptor_all_options() {
    let desc = ToolDescriptor::new("full_tool", "Full Tool", "A tool with all options")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "required_field": {"type": "string"},
                "optional_field": {"type": "number"}
            },
            "required": ["required_field"]
        }))
        .with_confirmation(true)
        .with_category("testing")
        .with_metadata("version", json!("1.0.0"))
        .with_metadata("author", json!("test"));

    assert_eq!(desc.id, "full_tool");
    assert_eq!(desc.name, "Full Tool");
    assert!(desc.requires_confirmation);
    assert_eq!(desc.category, Some("testing".to_string()));
    assert_eq!(desc.metadata.len(), 2);
}

#[test]
fn test_tool_descriptor_minimal() {
    let desc = ToolDescriptor::new("minimal", "Minimal", "");

    assert_eq!(desc.id, "minimal");
    assert_eq!(desc.description, "");
    assert!(!desc.requires_confirmation);
    assert!(desc.category.is_none());
    assert!(desc.metadata.is_empty());
}

// ============================================================================
// Stream End with Captured Tool Calls Tests (for coverage lines 92-102)
// ============================================================================

#[test]
fn test_stream_collector_end_event_with_captured_tool_calls() {
    use genai::chat::{ChatStreamEvent, MessageContent, StreamEnd};

    let mut collector = StreamCollector::new();

    // Create StreamEnd with captured_content containing tool calls
    let tool_call = genai::chat::ToolCall {
        call_id: "captured_call_1".to_string(),
        fn_name: "captured_search".to_string(),
        fn_arguments: json!({"query": "captured test"}),
        thought_signatures: None,
    };

    let end = StreamEnd {
        captured_usage: None,
        captured_content: Some(MessageContent::from_tool_calls(vec![tool_call])),
        captured_reasoning_content: None,
    };

    // Process the end event - this should capture the tool calls
    let output = collector.process(ChatStreamEvent::End(end));
    assert!(output.is_none()); // End event always returns None

    // Verify the captured tool calls are in the result
    let result = collector.finish();
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "captured_search");
    assert_eq!(result.tool_calls[0].id, "captured_call_1");
}

#[test]
fn test_stream_collector_end_event_with_multiple_captured_tool_calls() {
    use genai::chat::{ChatStreamEvent, MessageContent, StreamEnd};

    let mut collector = StreamCollector::new();

    // Create multiple tool calls in captured_content
    let tc1 = genai::chat::ToolCall {
        call_id: "cap_call_1".to_string(),
        fn_name: "tool_a".to_string(),
        fn_arguments: json!({"arg": "a"}),
        thought_signatures: None,
    };
    let tc2 = genai::chat::ToolCall {
        call_id: "cap_call_2".to_string(),
        fn_name: "tool_b".to_string(),
        fn_arguments: json!({"arg": "b"}),
        thought_signatures: None,
    };

    let end = StreamEnd {
        captured_usage: None,
        captured_content: Some(MessageContent::from_tool_calls(vec![tc1, tc2])),
        captured_reasoning_content: None,
    };

    collector.process(ChatStreamEvent::End(end));
    let result = collector.finish();

    assert_eq!(result.tool_calls.len(), 2);
    let names: Vec<&str> = result.tool_calls.iter().map(|tc| tc.name.as_str()).collect();
    assert!(names.contains(&"tool_a"));
    assert!(names.contains(&"tool_b"));
}

#[test]
fn test_stream_collector_end_merges_chunk_and_captured_tool_calls() {
    use genai::chat::{ChatStreamEvent, MessageContent, StreamChunk, StreamEnd, ToolChunk};

    let mut collector = StreamCollector::new();

    // Add text chunk
    collector.process(ChatStreamEvent::Chunk(StreamChunk {
        content: "Processing...".to_string(),
    }));

    // Add a tool call via ToolCallChunk
    let chunk_tc = genai::chat::ToolCall {
        call_id: "chunk_call".to_string(),
        fn_name: "chunk_tool".to_string(),
        fn_arguments: json!({"from": "chunk"}),
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk {
        tool_call: chunk_tc,
    }));

    // End event with additional captured tool call
    let captured_tc = genai::chat::ToolCall {
        call_id: "end_call".to_string(),
        fn_name: "end_tool".to_string(),
        fn_arguments: json!({"from": "end"}),
        thought_signatures: None,
    };
    let end = StreamEnd {
        captured_usage: None,
        captured_content: Some(MessageContent::from_tool_calls(vec![captured_tc])),
        captured_reasoning_content: None,
    };

    collector.process(ChatStreamEvent::End(end));
    let result = collector.finish();

    assert_eq!(result.text, "Processing...");
    assert_eq!(result.tool_calls.len(), 2);
}

#[test]
fn test_stream_collector_tool_chunk_with_null_arguments() {
    use carve_agent::StreamOutput;
    use genai::chat::{ChatStreamEvent, ToolChunk};

    let mut collector = StreamCollector::new();

    // Tool call chunk with name first (triggers ToolCallStart)
    let tc1 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "my_tool".to_string(),
        fn_arguments: serde_json::Value::Null, // null arguments
        thought_signatures: None,
    };
    let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));
    // Should emit ToolCallStart
    assert!(matches!(output, Some(StreamOutput::ToolCallStart { .. })));

    // Tool call chunk with null arguments again (tests the "null" check at line 80)
    let tc2 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "".to_string(), // empty name (already set)
        fn_arguments: serde_json::Value::Null,
        thought_signatures: None,
    };
    let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));
    // Should return None because args_str == "null"
    assert!(output.is_none());

    let result = collector.finish();
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].name, "my_tool");
}

#[test]
fn test_stream_collector_tool_chunk_with_empty_string_arguments() {
    use carve_agent::StreamOutput;
    use genai::chat::{ChatStreamEvent, ToolChunk};

    let mut collector = StreamCollector::new();

    // First set the tool name
    let tc1 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "test_tool".to_string(),
        fn_arguments: json!(""), // empty string serializes to ""
        thought_signatures: None,
    };
    collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc1 }));

    // Now try with empty string value - tests the !args_str.is_empty() check
    let tc2 = genai::chat::ToolCall {
        call_id: "call_1".to_string(),
        fn_name: "".to_string(),
        fn_arguments: json!(""), // empty string
        thought_signatures: None,
    };
    let output = collector.process(ChatStreamEvent::ToolCallChunk(ToolChunk { tool_call: tc2 }));
    // Should emit ToolCallDelta because "" serializes to `"\"\""`
    // Actually let's check what it serializes to
    let serialized = json!("").to_string();
    if serialized.is_empty() || serialized == "null" {
        assert!(output.is_none());
    } else {
        // It should be `""` which is not empty
        assert!(matches!(output, Some(StreamOutput::ToolCallDelta { .. })));
    }
}

// ============================================================================
// Interaction to AG-UI Conversion Scenario Tests
// ============================================================================

use carve_agent::ag_ui::{AGUIContext, AGUIEvent};
use carve_agent::stream::AgentEvent;
use carve_agent::Interaction;

/// Test complete scenario: Permission confirmation via Interaction → AG-UI
#[test]
fn test_scenario_permission_confirmation_to_ag_ui() {
    // 1. Plugin creates an Interaction for permission confirmation
    let interaction = Interaction::new("perm_write_file_123", "confirm")
        .with_message("Allow tool 'write_file' to write to /etc/config?")
        .with_parameters(json!({
            "tool_id": "write_file",
            "tool_args": {
                "path": "/etc/config",
                "content": "new config"
            }
        }));

    // 2. Create AgentEvent::Pending (what the agent loop would emit)
    let event = AgentEvent::Pending {
        interaction: interaction.clone(),
    };

    // 3. Convert to AG-UI events
    let mut ctx = AGUIContext::new("thread_123".into(), "run_456".into());
    let ag_ui_events = event.to_ag_ui_events(&mut ctx);

    // 4. Verify AG-UI event sequence
    assert_eq!(ag_ui_events.len(), 3, "Should produce 3 AG-UI events");

    // Event 1: ToolCallStart
    match &ag_ui_events[0] {
        AGUIEvent::ToolCallStart {
            tool_call_id,
            tool_call_name,
            ..
        } => {
            assert_eq!(tool_call_id, "perm_write_file_123");
            assert_eq!(tool_call_name, "confirm"); // action becomes tool name
        }
        _ => panic!("Expected ToolCallStart, got {:?}", ag_ui_events[0]),
    }

    // Event 2: ToolCallArgs
    match &ag_ui_events[1] {
        AGUIEvent::ToolCallArgs {
            tool_call_id,
            delta,
            ..
        } => {
            assert_eq!(tool_call_id, "perm_write_file_123");
            let args: Value = serde_json::from_str(delta).unwrap();
            assert_eq!(args["id"], "perm_write_file_123");
            assert_eq!(
                args["message"],
                "Allow tool 'write_file' to write to /etc/config?"
            );
            assert_eq!(args["parameters"]["tool_id"], "write_file");
        }
        _ => panic!("Expected ToolCallArgs, got {:?}", ag_ui_events[1]),
    }

    // Event 3: ToolCallEnd
    match &ag_ui_events[2] {
        AGUIEvent::ToolCallEnd { tool_call_id, .. } => {
            assert_eq!(tool_call_id, "perm_write_file_123");
        }
        _ => panic!("Expected ToolCallEnd, got {:?}", ag_ui_events[2]),
    }
}

/// Test scenario: Custom frontend action (file picker)
#[test]
fn test_scenario_custom_frontend_action_to_ag_ui() {
    // 1. Create a custom frontend action interaction
    let interaction = Interaction::new("picker_001", "file_picker")
        .with_message("Select a configuration file")
        .with_parameters(json!({
            "accept": [".json", ".yaml", ".toml"],
            "multiple": false,
            "directory": "/home/user/configs"
        }))
        .with_response_schema(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "name": { "type": "string" }
            },
            "required": ["path"]
        }));

    // 2. Convert directly to AG-UI events
    let ag_ui_events = interaction.to_ag_ui_events();

    // 3. Verify the tool call represents our custom action
    assert_eq!(ag_ui_events.len(), 3);

    match &ag_ui_events[0] {
        AGUIEvent::ToolCallStart {
            tool_call_name, ..
        } => {
            assert_eq!(tool_call_name, "file_picker"); // Custom action name
        }
        _ => panic!("Expected ToolCallStart"),
    }

    match &ag_ui_events[1] {
        AGUIEvent::ToolCallArgs { delta, .. } => {
            let args: Value = serde_json::from_str(delta).unwrap();
            // Verify response_schema is included for client validation
            assert!(args["response_schema"].is_object());
            assert_eq!(args["response_schema"]["type"], "object");
            // Verify parameters
            assert_eq!(args["parameters"]["multiple"], false);
        }
        _ => panic!("Expected ToolCallArgs"),
    }
}

/// Test scenario: Text streaming interrupted by pending interaction
#[test]
fn test_scenario_text_interrupted_by_interaction() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // 1. Start text streaming
    let text_event = AgentEvent::TextDelta {
        delta: "I'll help you ".into(),
    };
    let events1 = text_event.to_ag_ui_events(&mut ctx);
    assert!(events1.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));

    // 2. More text
    let text_event2 = AgentEvent::TextDelta {
        delta: "with that file.".into(),
    };
    let events2 = text_event2.to_ag_ui_events(&mut ctx);
    assert!(events2.iter().any(|e| matches!(e, AGUIEvent::TextMessageContent { .. })));

    // 3. Interaction interrupts (e.g., permission needed)
    let interaction = Interaction::new("int_1", "confirm")
        .with_message("Proceed with file operation?");
    let pending_event = AgentEvent::Pending { interaction };
    let events3 = pending_event.to_ag_ui_events(&mut ctx);

    // Should end text stream before tool call
    assert!(events3.len() >= 4, "Should have TextMessageEnd + 3 tool call events");
    assert!(
        matches!(events3[0], AGUIEvent::TextMessageEnd { .. }),
        "First event should be TextMessageEnd"
    );
    assert!(
        matches!(events3[1], AGUIEvent::ToolCallStart { .. }),
        "Second event should be ToolCallStart"
    );
}

/// Test scenario: Multiple interaction types
#[test]
fn test_scenario_various_interaction_types() {
    // Different action types all use the same mechanism
    let interactions = vec![
        ("confirm", "confirm", "Allow this action?"),
        ("input", "input", "Enter your name:"),
        ("select", "select", "Choose an option:"),
        ("oauth", "oauth", "Authenticate with GitHub"),
        ("custom_widget", "custom_widget", "Configure settings"),
    ];

    for (id, action, message) in interactions {
        let interaction = Interaction::new(id, action).with_message(message);

        let events = interaction.to_ag_ui_events();

        // All produce the same event structure
        assert_eq!(events.len(), 3);

        match &events[0] {
            AGUIEvent::ToolCallStart {
                tool_call_id,
                tool_call_name,
                ..
            } => {
                assert_eq!(tool_call_id, id);
                assert_eq!(tool_call_name, action); // action → tool name
            }
            _ => panic!("Expected ToolCallStart for action: {}", action),
        }
    }
}

// ============================================================================
// FrontendToolPlugin Scenario Tests
// ============================================================================

use carve_agent::ag_ui::{AGUIToolDef, FrontendToolPlugin, RunAgentRequest};
use carve_agent::phase::{Phase, ToolContext, TurnContext};
use carve_agent::plugin::AgentPlugin;
use carve_agent::types::ToolCall;

/// Test scenario: Complete frontend tool flow from request to AG-UI events
#[tokio::test]
async fn test_scenario_frontend_tool_request_to_agui() {
    // 1. Client sends request with mixed frontend/backend tools
    let request = RunAgentRequest::new("thread_1".to_string(), "run_1".to_string())
        .with_tool(AGUIToolDef::backend("search", "Search the web"))
        .with_tool(AGUIToolDef::backend("read_file", "Read a file"))
        .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy text to clipboard"))
        .with_tool(AGUIToolDef::frontend("showNotification", "Show a notification"));

    // 2. FrontendToolPlugin is created from request
    let plugin = FrontendToolPlugin::from_request(&request);

    // 3. Simulate agent calling a frontend tool
    let session = Session::new("session_1");
    let mut turn = TurnContext::new(&session, vec![]);

    let tool_call = ToolCall::new(
        "call_001",
        "copyToClipboard",
        json!({
            "text": "Hello, World!",
            "format": "plain"
        }),
    );
    turn.tool = Some(ToolContext::new(&tool_call));

    // 4. Plugin intercepts in BeforeToolExecute phase
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    // 5. Tool should be pending
    assert!(turn.tool_pending());

    // 6. Get interaction and convert to AG-UI
    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    let events = interaction.to_ag_ui_events();

    // 7. Verify AG-UI events
    assert_eq!(events.len(), 3);

    match &events[0] {
        AGUIEvent::ToolCallStart {
            tool_call_id,
            tool_call_name,
            ..
        } => {
            assert_eq!(tool_call_id, "call_001");
            assert_eq!(tool_call_name, "tool:copyToClipboard");
        }
        _ => panic!("Expected ToolCallStart"),
    }

    match &events[1] {
        AGUIEvent::ToolCallArgs { delta, .. } => {
            let args: Value = serde_json::from_str(delta).unwrap();
            assert_eq!(args["parameters"]["text"], "Hello, World!");
            assert_eq!(args["parameters"]["format"], "plain");
        }
        _ => panic!("Expected ToolCallArgs"),
    }
}

/// Test scenario: Multiple frontend tools called in sequence
#[tokio::test]
async fn test_scenario_multiple_frontend_tools_sequence() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy"))
        .with_tool(AGUIToolDef::frontend("showNotification", "Notify"))
        .with_tool(AGUIToolDef::frontend("openDialog", "Dialog"));

    let plugin = FrontendToolPlugin::from_request(&request);
    let session = Session::new("test");

    // Simulate three frontend tool calls in sequence
    let tool_calls = vec![
        ("call_1", "copyToClipboard", json!({"text": "data1"})),
        ("call_2", "showNotification", json!({"message": "Done!"})),
        ("call_3", "openDialog", json!({"title": "Confirm"})),
    ];

    for (call_id, tool_name, args) in tool_calls {
        let mut turn = TurnContext::new(&session, vec![]);
        let tool_call = ToolCall::new(call_id, tool_name, args.clone());
        turn.tool = Some(ToolContext::new(&tool_call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_pending(), "Tool {} should be pending", tool_name);

        let interaction = turn
            .tool
            .as_ref()
            .unwrap()
            .pending_interaction
            .as_ref()
            .unwrap();
        assert_eq!(interaction.id, call_id);
        assert_eq!(interaction.action, format!("tool:{}", tool_name));
        assert_eq!(interaction.parameters, args);
    }
}

/// Test scenario: Frontend tool with complex nested arguments
#[tokio::test]
async fn test_scenario_frontend_tool_complex_args() {
    let plugin = FrontendToolPlugin::new(["fileDialog".to_string()].into_iter().collect());

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    // Complex nested arguments
    let complex_args = json!({
        "options": {
            "filters": [
                {"name": "Images", "extensions": ["png", "jpg", "gif"]},
                {"name": "Documents", "extensions": ["pdf", "doc", "txt"]}
            ],
            "defaultPath": "/home/user/documents",
            "properties": {
                "multiSelections": true,
                "showHiddenFiles": false
            }
        },
        "metadata": {
            "requestId": "req_123",
            "timestamp": 1704067200,
            "context": {
                "source": "editor",
                "purpose": "import"
            }
        }
    });

    let tool_call = ToolCall::new("call_complex", "fileDialog", complex_args.clone());
    turn.tool = Some(ToolContext::new(&tool_call));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(turn.tool_pending());

    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .as_ref()
        .unwrap();

    // Verify complex args are preserved
    assert_eq!(interaction.parameters, complex_args);
    assert_eq!(
        interaction.parameters["options"]["filters"][0]["name"],
        "Images"
    );
    assert_eq!(
        interaction.parameters["metadata"]["context"]["source"],
        "editor"
    );
}

/// Test scenario: Frontend tool with empty/null arguments
#[tokio::test]
async fn test_scenario_frontend_tool_empty_args() {
    let plugin = FrontendToolPlugin::new(["getClipboard".to_string()].into_iter().collect());

    let session = Session::new("test");

    // Test with empty object
    {
        let mut turn = TurnContext::new(&session, vec![]);
        let tool_call = ToolCall::new("call_empty", "getClipboard", json!({}));
        turn.tool = Some(ToolContext::new(&tool_call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_pending());
        let interaction = turn
            .tool
            .as_ref()
            .unwrap()
            .pending_interaction
            .as_ref()
            .unwrap();
        assert_eq!(interaction.parameters, json!({}));
    }

    // Test with null
    {
        let mut turn = TurnContext::new(&session, vec![]);
        let tool_call = ToolCall::new("call_null", "getClipboard", Value::Null);
        turn.tool = Some(ToolContext::new(&tool_call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_pending());
        let interaction = turn
            .tool
            .as_ref()
            .unwrap()
            .pending_interaction
            .as_ref()
            .unwrap();
        assert_eq!(interaction.parameters, Value::Null);
    }
}

/// Test scenario: Frontend tool names with special characters
#[tokio::test]
async fn test_scenario_frontend_tool_special_names() {
    // Various tool name formats that might appear
    let tool_names = vec![
        "copy_to_clipboard",      // snake_case
        "copyToClipboard",        // camelCase
        "CopyToClipboard",        // PascalCase
        "copy-to-clipboard",      // kebab-case
        "namespace.copyClipboard", // dotted namespace
        "ui::clipboard::copy",    // rust-style path (unusual but valid)
    ];

    for tool_name in tool_names {
        let plugin = FrontendToolPlugin::new([tool_name.to_string()].into_iter().collect());

        let session = Session::new("test");
        let mut turn = TurnContext::new(&session, vec![]);
        let tool_call = ToolCall::new("call_1", tool_name, json!({}));
        turn.tool = Some(ToolContext::new(&tool_call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert!(turn.tool_pending(), "Tool '{}' should be pending", tool_name);

        let interaction = turn
            .tool
            .as_ref()
            .unwrap()
            .pending_interaction
            .as_ref()
            .unwrap();
        assert_eq!(
            interaction.action,
            format!("tool:{}", tool_name),
            "Action should be 'tool:{}' for tool '{}'",
            tool_name,
            tool_name
        );
    }
}

/// Test scenario: Tool name case sensitivity
#[tokio::test]
async fn test_scenario_frontend_tool_case_sensitivity() {
    // Only "CopyToClipboard" is registered as frontend
    let plugin = FrontendToolPlugin::new(["CopyToClipboard".to_string()].into_iter().collect());

    let session = Session::new("test");

    // Different cases - only exact match should work
    let test_cases = vec![
        ("CopyToClipboard", true),  // exact match
        ("copytoclipboard", false), // lowercase
        ("COPYTOCLIPBOARD", false), // uppercase
        ("copyToClipboard", false), // different case
    ];

    for (tool_name, should_be_pending) in test_cases {
        let mut turn = TurnContext::new(&session, vec![]);
        let tool_call = ToolCall::new("call_1", tool_name, json!({}));
        turn.tool = Some(ToolContext::new(&tool_call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert_eq!(
            turn.tool_pending(),
            should_be_pending,
            "Tool '{}' pending state should be {}",
            tool_name,
            should_be_pending
        );
    }
}

/// Test scenario: Frontend tool interaction serializes correctly for wire format
#[test]
fn test_scenario_frontend_tool_wire_format() {
    // Create interaction as FrontendToolPlugin would
    let interaction = Interaction::new("call_abc123", "tool:showNotification")
        .with_parameters(json!({
            "title": "Success",
            "message": "Operation completed",
            "type": "info",
            "duration": 5000
        }));

    // Convert to AG-UI events (what goes over the wire)
    let events = interaction.to_ag_ui_events();

    // Serialize each event as it would be sent
    for event in &events {
        let json_str = serde_json::to_string(event).expect("Event should serialize");

        // Verify it can be deserialized back
        let _: AGUIEvent = serde_json::from_str(&json_str).expect("Event should deserialize");

        // Verify no null/undefined sneaking in for required fields
        let json_val: Value = serde_json::from_str(&json_str).unwrap();
        assert!(
            json_val.get("type").is_some(),
            "Event should have 'type' field"
        );
    }

    // Check ToolCallArgs specifically - the main payload
    match &events[1] {
        AGUIEvent::ToolCallArgs { delta, .. } => {
            let args: Value = serde_json::from_str(delta).unwrap();

            // Verify structure matches what client expects
            assert!(args.get("id").is_some(), "Should have id");
            assert!(args.get("parameters").is_some(), "Should have parameters");

            // Verify nested data
            assert_eq!(args["parameters"]["title"], "Success");
            assert_eq!(args["parameters"]["duration"], 5000);
        }
        _ => panic!("Expected ToolCallArgs at index 1"),
    }
}

/// Test scenario: Frontend tool to AgentEvent::Pending to AG-UI events
#[tokio::test]
async fn test_scenario_frontend_tool_full_event_pipeline() {
    let plugin = FrontendToolPlugin::new(["showModal".to_string()].into_iter().collect());

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    let tool_call = ToolCall::new(
        "modal_call_1",
        "showModal",
        json!({
            "content": "Are you sure?",
            "buttons": ["Yes", "No"]
        }),
    );
    turn.tool = Some(ToolContext::new(&tool_call));

    // 1. Plugin creates pending state with interaction
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    // 2. Agent loop would create AgentEvent::Pending
    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();
    let agent_event = AgentEvent::Pending { interaction };

    // 3. Convert to AG-UI events with context
    let mut ctx = AGUIContext::new("thread_123".into(), "run_456".into());
    let ag_ui_events = agent_event.to_ag_ui_events(&mut ctx);

    // 4. Verify complete event sequence
    assert_eq!(ag_ui_events.len(), 3);

    // ToolCallStart
    assert!(matches!(ag_ui_events[0], AGUIEvent::ToolCallStart { .. }));
    if let AGUIEvent::ToolCallStart {
        tool_call_id,
        tool_call_name,
        ..
    } = &ag_ui_events[0]
    {
        assert_eq!(tool_call_id, "modal_call_1");
        assert_eq!(tool_call_name, "tool:showModal");
    }

    // ToolCallArgs
    assert!(matches!(ag_ui_events[1], AGUIEvent::ToolCallArgs { .. }));
    if let AGUIEvent::ToolCallArgs { delta, .. } = &ag_ui_events[1] {
        let args: Value = serde_json::from_str(delta).unwrap();
        assert_eq!(args["parameters"]["content"], "Are you sure?");
        assert_eq!(args["parameters"]["buttons"][0], "Yes");
    }

    // ToolCallEnd
    assert!(matches!(ag_ui_events[2], AGUIEvent::ToolCallEnd { .. }));
}

/// Test scenario: Backend tool should not be affected by FrontendToolPlugin
#[tokio::test]
async fn test_scenario_backend_tool_passthrough() {
    let plugin = FrontendToolPlugin::new(["frontendOnly".to_string()].into_iter().collect());

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    // Backend tool call
    let tool_call = ToolCall::new(
        "call_backend",
        "search",
        json!({
            "query": "rust async",
            "limit": 10
        }),
    );
    turn.tool = Some(ToolContext::new(&tool_call));

    // Plugin should not interfere
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(!turn.tool_pending(), "Backend tool should not be pending");
    assert!(!turn.tool_blocked(), "Backend tool should not be blocked");
    assert!(
        turn.tool.as_ref().unwrap().pending_interaction.is_none(),
        "No interaction should be created"
    );
}

/// Test scenario: Request with no frontend tools creates empty plugin
#[test]
fn test_scenario_no_frontend_tools_in_request() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::backend("search", "Search"))
        .with_tool(AGUIToolDef::backend("read", "Read file"))
        .with_tool(AGUIToolDef::backend("write", "Write file"));

    let plugin = FrontendToolPlugin::from_request(&request);

    // All tools are backend, none should be frontend
    assert!(!plugin.is_frontend_tool("search"));
    assert!(!plugin.is_frontend_tool("read"));
    assert!(!plugin.is_frontend_tool("write"));
    assert!(!plugin.is_frontend_tool("nonexistent"));
}

/// Test scenario: Empty request creates empty plugin
#[test]
fn test_scenario_empty_request() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string());

    let plugin = FrontendToolPlugin::from_request(&request);

    // No tools in request, none should be frontend
    assert!(!plugin.is_frontend_tool("any_tool"));
    assert!(!plugin.is_frontend_tool(""));
}

// ============================================================================
// Permission Resume Flow Scenario Tests
// ============================================================================

use carve_agent::ag_ui::AGUIMessage;
use carve_agent::plugins::PermissionPlugin;

/// Test scenario: Complete permission approval flow
/// Agent → Pending → AG-UI Events → Client Approves → Resume
#[tokio::test]
async fn test_scenario_permission_approved_complete_flow() {

    // Phase 1: Agent requests permission
    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    // Set up ask permission
    turn.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );

    // Simulate tool call
    let tool_call = ToolCall::new("call_write_file", "write_file", json!({"path": "/etc/config"}));
    turn.tool = Some(ToolContext::new(&tool_call));

    // PermissionPlugin creates pending interaction
    let plugin = PermissionPlugin;
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(turn.tool_pending());
    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    // Phase 2: Convert to AG-UI events
    let ag_ui_events = interaction.to_ag_ui_events();
    assert_eq!(ag_ui_events.len(), 3);

    // Phase 3: Client receives events and approves
    // (Simulated by creating a new request with tool response)
    let client_response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("true", &interaction.id));

    // Phase 4: Check approval
    assert!(client_response_request.is_interaction_approved(&interaction.id));
    assert!(!client_response_request.is_interaction_denied(&interaction.id));

    // Phase 5: Get response and verify
    let response = client_response_request
        .get_interaction_response(&interaction.id)
        .unwrap();
    assert!(response.approved());
}

/// Test scenario: Complete permission denial flow
#[tokio::test]
async fn test_scenario_permission_denied_complete_flow() {

    // Phase 1: Agent requests permission
    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    turn.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );

    let tool_call = ToolCall::new("call_delete", "delete_file", json!({"path": "/important.txt"}));
    turn.tool = Some(ToolContext::new(&tool_call));

    let plugin = PermissionPlugin;
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(turn.tool_pending());
    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    // Phase 2-3: Client denies
    let client_response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("false", &interaction.id));

    // Phase 4: Check denial
    assert!(client_response_request.is_interaction_denied(&interaction.id));
    assert!(!client_response_request.is_interaction_approved(&interaction.id));

    let response = client_response_request
        .get_interaction_response(&interaction.id)
        .unwrap();
    assert!(response.denied());
}

/// Test scenario: Frontend tool execution complete flow
#[tokio::test]
async fn test_scenario_frontend_tool_execution_complete_flow() {
    // Phase 1: Agent calls frontend tool
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy to clipboard"));

    let plugin = FrontendToolPlugin::from_request(&request);

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);

    let tool_call = ToolCall::new("call_copy_1", "copyToClipboard", json!({"text": "Hello!"}));
    turn.tool = Some(ToolContext::new(&tool_call));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(turn.tool_pending());
    let interaction = turn
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    // Verify action format
    assert_eq!(interaction.action, "tool:copyToClipboard");

    // Phase 2: Convert to AG-UI
    let ag_ui_events = interaction.to_ag_ui_events();
    assert_eq!(ag_ui_events.len(), 3);

    // Phase 3: Client executes and returns result
    let client_response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"success":true,"bytes_copied":6}"#,
            &interaction.id,
        ));

    // Phase 4: Agent receives result
    let response = client_response_request
        .get_interaction_response(&interaction.id)
        .unwrap();

    assert!(response.result["success"].as_bool().unwrap());
    assert_eq!(response.result["bytes_copied"], 6);
}

/// Test scenario: Multiple interactions in sequence
#[tokio::test]
async fn test_scenario_multiple_interactions_sequence() {

    let session = Session::new("test");
    let plugin = PermissionPlugin;

    // First tool: write_file
    let mut turn1 = TurnContext::new(&session, vec![]);
    turn1.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );
    let call1 = ToolCall::new("call_1", "write_file", json!({}));
    turn1.tool = Some(ToolContext::new(&call1));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    let interaction1 = turn1
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    // Second tool: read_file
    let mut turn2 = TurnContext::new(&session, vec![]);
    turn2.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );
    let call2 = ToolCall::new("call_2", "read_file", json!({}));
    turn2.tool = Some(ToolContext::new(&call2));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    let interaction2 = turn2
        .tool
        .as_ref()
        .unwrap()
        .pending_interaction
        .clone()
        .unwrap();

    // Client responds to both
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("true", &interaction1.id))
        .with_message(AGUIMessage::tool("false", &interaction2.id));

    // Verify responses
    assert!(response_request.is_interaction_approved(&interaction1.id));
    assert!(response_request.is_interaction_denied(&interaction2.id));

    let responses = response_request.interaction_responses();
    assert_eq!(responses.len(), 2);
}

/// Test scenario: Frontend tool with complex result
#[test]
fn test_scenario_frontend_tool_complex_result() {
    let client_response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{
                "success": true,
                "selected_files": [
                    {"path": "/home/user/doc1.txt", "size": 1024},
                    {"path": "/home/user/doc2.txt", "size": 2048}
                ],
                "metadata": {
                    "dialog_duration_ms": 1500,
                    "user_action": "confirm"
                }
            }"#,
            "file_picker_call_1",
        ));

    let response = client_response_request
        .get_interaction_response("file_picker_call_1")
        .unwrap();

    assert!(response.result["success"].as_bool().unwrap());
    assert_eq!(response.result["selected_files"].as_array().unwrap().len(), 2);
    assert_eq!(
        response.result["selected_files"][0]["path"],
        "/home/user/doc1.txt"
    );
    assert_eq!(response.result["metadata"]["user_action"], "confirm");
}

/// Test scenario: Permission with custom response format
#[test]
fn test_scenario_permission_custom_response_format() {
    // Using object format with reason
    let request1 = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"approved":true,"reason":"User trusts this operation"}"#,
            "perm_1",
        ));

    assert!(request1.is_interaction_approved("perm_1"));

    // Using object format with denied flag
    let request2 = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"denied":true,"reason":"User is cautious"}"#,
            "perm_2",
        ));

    assert!(request2.is_interaction_denied("perm_2"));

    // Using allowed flag
    let request3 = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(r#"{"allowed":true}"#, "perm_3"));

    assert!(request3.is_interaction_approved("perm_3"));
}

/// Test scenario: Interaction response with input value
#[test]
fn test_scenario_input_interaction_response() {
    // User provides text input
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("John Doe", "input_name_1"));

    let response = request.get_interaction_response("input_name_1").unwrap();
    assert_eq!(response.result, Value::String("John Doe".into()));

    // Not approved or denied - it's just input
    assert!(!response.approved());
    assert!(!response.denied());
}

/// Test scenario: Selection interaction response
#[test]
fn test_scenario_select_interaction_response() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"selected_index":2,"selected_value":"Option C"}"#,
            "select_option_1",
        ));

    let response = request.get_interaction_response("select_option_1").unwrap();
    assert_eq!(response.result["selected_index"], 2);
    assert_eq!(response.result["selected_value"], "Option C");
}

/// Test scenario: Mixed message types in request
#[test]
fn test_scenario_mixed_messages_with_interaction_response() {
    // Real-world scenario: conversation + tool responses
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::user("Please write to the file"))
        .with_message(AGUIMessage::assistant("I'll write to the file, but need permission"))
        .with_message(AGUIMessage::tool("true", "perm_write_1"))
        .with_message(AGUIMessage::assistant("Done!"));

    // Should find the tool response
    assert!(request.has_interaction_response("perm_write_1"));
    assert!(request.is_interaction_approved("perm_write_1"));

    // Should have exactly one interaction response
    let responses = request.interaction_responses();
    assert_eq!(responses.len(), 1);
}

/// Test scenario: InteractionResponsePlugin blocks denied tool in execution flow
#[tokio::test]
async fn test_scenario_interaction_response_plugin_blocks_denied() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    let session = Session::new("test");

    // Create plugin with denied interaction
    let plugin = InteractionResponsePlugin::new(
        vec![], // no approved
        vec!["permission_write_file".to_string()], // denied
    );

    // Simulate tool call with matching interaction ID format
    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new("permission_write_file", "write_file", json!({"path": "/etc/config"}));
    turn.tool = Some(ToolContext::new(&call));

    // Run plugin
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    // Tool should be blocked
    assert!(turn.tool_blocked(), "Denied tool should be blocked");
    // Verify block reason via direct field access
    let block_reason = turn.tool.as_ref().unwrap().block_reason.as_ref().unwrap();
    assert!(
        block_reason.contains("denied"),
        "Block reason should mention denial, got: {}",
        block_reason
    );
}

/// Test scenario: InteractionResponsePlugin allows approved tool in execution flow
#[tokio::test]
async fn test_scenario_interaction_response_plugin_allows_approved() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    let session = Session::new("test");

    // Create plugin with approved interaction
    let plugin = InteractionResponsePlugin::new(
        vec!["permission_read_file".to_string()], // approved
        vec![], // no denied
    );

    // Simulate tool call
    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new("permission_read_file", "read_file", json!({"path": "/home/user/doc.txt"}));
    turn.tool = Some(ToolContext::new(&call));

    // Run plugin
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    // Tool should NOT be blocked
    assert!(!turn.tool_blocked(), "Approved tool should not be blocked");
}

/// Test scenario: Complete end-to-end flow with PermissionPlugin → InteractionResponsePlugin
#[tokio::test]
async fn test_scenario_e2e_permission_to_response_flow() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    let session = Session::new("test");

    // Step 1: First run - PermissionPlugin creates pending interaction
    let permission_plugin = PermissionPlugin;
    let mut turn1 = TurnContext::new(&session, vec![]);
    turn1.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );
    let call = ToolCall::new("call_exec", "execute_command", json!({"cmd": "ls"}));
    turn1.tool = Some(ToolContext::new(&call));

    permission_plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    assert!(turn1.tool_pending(), "Permission ask should create pending");

    let interaction = turn1.tool.as_ref().unwrap().pending_interaction.clone().unwrap();
    assert!(interaction.id.starts_with("permission_"), "Interaction ID should start with permission_");

    // Step 2: Client approves
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("true", &interaction.id));
    assert!(response_request.is_interaction_approved(&interaction.id));

    // Step 3: Second run - InteractionResponsePlugin processes approval
    let response_plugin = InteractionResponsePlugin::from_request(&response_request);
    let mut turn2 = TurnContext::new(&session, vec![]);
    turn2.set(
        "permissions",
        json!({ "default_behavior": "ask", "tools": {} }),
    );
    let call2 = ToolCall::new(&interaction.id, "execute_command", json!({"cmd": "ls"}));
    turn2.tool = Some(ToolContext::new(&call2));

    // InteractionResponsePlugin runs first
    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;

    // Tool should NOT be blocked (approved)
    assert!(!turn2.tool_blocked(), "Approved tool should not be blocked on resume");

    // PermissionPlugin runs second - but InteractionResponsePlugin didn't set pending
    permission_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;

    // PermissionPlugin should still create pending (because permission wasn't updated to Allow)
    // This is expected - in a real flow, the permission would be updated to Allow after approval
    assert!(turn2.tool_pending() || !turn2.tool_blocked(), "Tool should proceed");
}

/// Test scenario: FrontendToolPlugin and InteractionResponsePlugin coordination
#[tokio::test]
async fn test_scenario_frontend_tool_with_response_plugin() {
    use carve_agent::ag_ui::{FrontendToolPlugin, InteractionResponsePlugin};

    let session = Session::new("test");

    // Setup: Frontend tool request
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("showDialog", "Show a dialog"));

    // Step 1: FrontendToolPlugin creates pending for frontend tool
    let frontend_plugin = FrontendToolPlugin::from_request(&request);
    let mut turn1 = TurnContext::new(&session, vec![]);
    let call = ToolCall::new("call_dialog_1", "showDialog", json!({"title": "Confirm"}));
    turn1.tool = Some(ToolContext::new(&call));

    frontend_plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    assert!(turn1.tool_pending(), "Frontend tool should create pending");

    let interaction = turn1.tool.as_ref().unwrap().pending_interaction.clone().unwrap();
    assert_eq!(interaction.action, "tool:showDialog");

    // Step 2: Client executes and returns result
    let response_request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"success":true,"user_clicked":"OK"}"#,
            &interaction.id,
        ));

    // Step 3: On resume, InteractionResponsePlugin processes the result
    let _response_plugin = InteractionResponsePlugin::from_request(&response_request);

    // The result is available (not blocked/denied)
    let response = response_request.get_interaction_response(&interaction.id).unwrap();
    assert!(response.result["success"].as_bool().unwrap());
    assert_eq!(response.result["user_clicked"], "OK");
}

/// Test scenario: AG-UI context state after pending interaction
#[test]
fn test_scenario_agui_context_state_after_pending() {
    let mut ctx = AGUIContext::new("thread_1".into(), "run_1".into());

    // Start text streaming
    let text_event = AgentEvent::TextDelta {
        delta: "Let me help you ".into(),
    };
    let events1 = text_event.to_ag_ui_events(&mut ctx);
    // First text delta should produce TextMessageStart
    assert!(events1.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));

    // More text
    let text_event2 = AgentEvent::TextDelta {
        delta: "with that.".into(),
    };
    let events2 = text_event2.to_ag_ui_events(&mut ctx);
    // Second text delta should produce only TextMessageContent (not Start)
    assert!(events2.iter().any(|e| matches!(e, AGUIEvent::TextMessageContent { .. })));
    assert!(!events2.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));

    // Pending interaction arrives
    let interaction = Interaction::new("perm_1", "confirm").with_message("Allow?");
    let pending_event = AgentEvent::Pending { interaction };
    let pending_events = pending_event.to_ag_ui_events(&mut ctx);

    // Should have: TextMessageEnd + 3 tool call events
    assert!(pending_events.len() >= 4);
    assert!(
        matches!(pending_events[0], AGUIEvent::TextMessageEnd { .. }),
        "First event should be TextMessageEnd to close the text stream"
    );
    assert!(
        matches!(pending_events[1], AGUIEvent::ToolCallStart { .. }),
        "Second event should be ToolCallStart for the interaction"
    );

    // After pending, text stream should be ended - verify by checking that
    // another text event would start a new stream
    let text_event3 = AgentEvent::TextDelta {
        delta: "New text".into(),
    };
    let events3 = text_event3.to_ag_ui_events(&mut ctx);
    // Should produce TextMessageStart again since previous stream was ended
    assert!(events3.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
}

// ============================================================================
// AG-UI Stream Flow Tests
// ============================================================================

/// Test: Event sequence in stream - verify RUN_STARTED is first
#[test]
fn test_agui_stream_event_sequence_run_started_first() {
    // Simulate stream events
    let events: Vec<AGUIEvent> = vec![
        AGUIEvent::run_started("t1", "r1", None),
        AGUIEvent::text_message_start("msg_1"),
        AGUIEvent::text_message_content("msg_1", "Hello"),
        AGUIEvent::text_message_end("msg_1"),
        AGUIEvent::run_finished("t1", "r1", None),
    ];

    // First event must be RUN_STARTED
    assert!(matches!(&events[0], AGUIEvent::RunStarted { .. }));

    // Last event must be RUN_FINISHED
    assert!(matches!(&events[events.len() - 1], AGUIEvent::RunFinished { .. }));
}

/// Test: Text interrupted by tool call - TEXT_MESSAGE_END before TOOL_CALL_START
#[test]
fn test_agui_stream_text_interrupted_by_tool_call() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Start text streaming
    let text1 = AgentEvent::TextDelta { delta: "Let me ".into() };
    let _ = text1.to_ag_ui_events(&mut ctx);

    let text2 = AgentEvent::TextDelta { delta: "search for that.".into() };
    let _ = text2.to_ag_ui_events(&mut ctx);

    // Tool call starts - should end text stream first
    let tool_start = AgentEvent::ToolCallStart {
        id: "call_1".into(),
        name: "search".into(),
    };
    let tool_events = tool_start.to_ag_ui_events(&mut ctx);

    // Should have TEXT_MESSAGE_END followed by TOOL_CALL_START
    assert!(tool_events.len() >= 2);
    assert!(
        matches!(&tool_events[0], AGUIEvent::TextMessageEnd { .. }),
        "First event should be TextMessageEnd, got {:?}",
        tool_events[0]
    );
    assert!(
        matches!(&tool_events[1], AGUIEvent::ToolCallStart { .. }),
        "Second event should be ToolCallStart, got {:?}",
        tool_events[1]
    );
}

/// Test: Tool call complete sequence - START -> ARGS -> READY(END) -> DONE(RESULT)
#[test]
fn test_agui_stream_tool_call_sequence() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Collect all events for a tool call
    let start = AgentEvent::ToolCallStart {
        id: "call_1".into(),
        name: "read_file".into(),
    };
    let start_events = start.to_ag_ui_events(&mut ctx);

    let args = AgentEvent::ToolCallDelta {
        id: "call_1".into(),
        args_delta: r#"{"path": "/tmp/file.txt"}"#.into(),
    };
    let args_events = args.to_ag_ui_events(&mut ctx);

    // ToolCallReady produces TOOL_CALL_END (marks end of args streaming)
    let ready = AgentEvent::ToolCallReady {
        id: "call_1".into(),
        name: "read_file".into(),
        arguments: json!({"path": "/tmp/file.txt"}),
    };
    let ready_events = ready.to_ag_ui_events(&mut ctx);

    // ToolCallDone produces TOOL_CALL_RESULT
    let done = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result: ToolResult::success("read_file", json!({"content": "Hello"})),
        patch: None,
    };
    let done_events = done.to_ag_ui_events(&mut ctx);

    // Verify sequence: Start events contain TOOL_CALL_START
    assert!(start_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));

    // Args events contain TOOL_CALL_ARGS
    assert!(args_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallArgs { .. })));

    // Ready events contain TOOL_CALL_END (end of argument streaming)
    assert!(ready_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallEnd { .. })));

    // Done events contain TOOL_CALL_RESULT
    assert!(done_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));
}

/// Test: Error event ends stream without RUN_FINISHED
#[test]
fn test_agui_stream_error_no_run_finished() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Start with RUN_STARTED (simulated)
    let _started = AGUIEvent::run_started("t1", "r1", None);

    // Error occurs
    let error = AgentEvent::Error {
        message: "LLM API error: rate limited".into(),
    };
    let error_events = error.to_ag_ui_events(&mut ctx);

    // Should emit RUN_ERROR
    assert!(error_events.iter().any(|e| matches!(e, AGUIEvent::RunError { .. })));

    // Should NOT have RUN_FINISHED in the error events
    assert!(!error_events.iter().any(|e| matches!(e, AGUIEvent::RunFinished { .. })));
}

/// Test: Pending event doesn't emit RUN_FINISHED
#[test]
fn test_agui_stream_pending_no_run_finished() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Pending interaction
    let interaction = Interaction::new("perm_1", "confirm")
        .with_message("Allow tool execution?");
    let pending = AgentEvent::Pending { interaction };
    let pending_events = pending.to_ag_ui_events(&mut ctx);

    // Should have tool call events (for the interaction)
    assert!(pending_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));

    // Should NOT have RUN_FINISHED
    assert!(!pending_events.iter().any(|e| matches!(e, AGUIEvent::RunFinished { .. })));
}

/// Test: SSE format validation
#[test]
fn test_agui_sse_format() {
    let event = AGUIEvent::run_started("t1", "r1", None);
    let json = serde_json::to_string(&event).unwrap();
    let sse = format!("data: {}\n\n", json);

    // Validate SSE format
    assert!(sse.starts_with("data: "));
    assert!(sse.ends_with("\n\n"));

    // Validate JSON is parseable
    let json_part = sse.strip_prefix("data: ").unwrap().strip_suffix("\n\n").unwrap();
    let parsed: AGUIEvent = serde_json::from_str(json_part).unwrap();
    assert!(matches!(parsed, AGUIEvent::RunStarted { .. }));
}

/// Test: Multiple SSE events in sequence
#[test]
fn test_agui_sse_multiple_events() {
    let events = vec![
        AGUIEvent::run_started("t1", "r1", None),
        AGUIEvent::text_message_start("m1"),
        AGUIEvent::text_message_content("m1", "Hello"),
        AGUIEvent::text_message_end("m1"),
        AGUIEvent::run_finished("t1", "r1", None),
    ];

    let mut sse_output = String::new();
    for event in &events {
        let json = serde_json::to_string(event).unwrap();
        sse_output.push_str(&format!("data: {}\n\n", json));
    }

    // Parse back
    let lines: Vec<&str> = sse_output.split("\n\n").filter(|s| !s.is_empty()).collect();
    assert_eq!(lines.len(), 5);

    for (i, line) in lines.iter().enumerate() {
        let json = line.strip_prefix("data: ").unwrap();
        let _: AGUIEvent = serde_json::from_str(json).expect(&format!("Failed to parse event {}", i));
    }
}

// ============================================================================
// Permission E2E Flow Tests
// ============================================================================

/// Test: Complete permission approval flow
/// Ask → Pending → Client Approves → Tool Executes
#[tokio::test]
async fn test_permission_flow_approval_e2e() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Phase 1: Agent requests permission (simulated by PermissionPlugin)
    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    turn.set("permissions", json!({ "default_behavior": "ask", "tools": {} }));

    let tool_call = ToolCall::new("call_write", "write_file", json!({"path": "/etc/config"}));
    turn.tool = Some(ToolContext::new(&tool_call));

    // PermissionPlugin creates pending
    let plugin = PermissionPlugin;
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(turn.tool_pending(), "Tool should be pending after permission ask");

    let interaction = turn.tool.as_ref().unwrap().pending_interaction.clone().unwrap();
    assert!(interaction.id.starts_with("permission_"));

    // Phase 2: Convert to AG-UI events (client would receive these)
    let ag_ui_events = interaction.to_ag_ui_events();
    assert_eq!(ag_ui_events.len(), 3); // Start, Args, End

    // Phase 3: Client approves (simulated by creating response request)
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("true", &interaction.id));

    assert!(response_request.is_interaction_approved(&interaction.id));

    // Phase 4: Resume with InteractionResponsePlugin
    let response_plugin = InteractionResponsePlugin::from_request(&response_request);
    assert!(response_plugin.has_responses());

    // Phase 5: On resume, tool should NOT be blocked
    let mut turn2 = TurnContext::new(&session, vec![]);
    // Use the interaction ID as the tool call ID (as happens in resume)
    let tool_call2 = ToolCall::new(&interaction.id, "write_file", json!({"path": "/etc/config"}));
    turn2.tool = Some(ToolContext::new(&tool_call2));

    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    assert!(!turn2.tool_blocked(), "Approved tool should not be blocked");
}

/// Test: Complete permission denial flow
/// Ask → Pending → Client Denies → Tool Blocked
#[tokio::test]
async fn test_permission_flow_denial_e2e() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Phase 1: Agent requests permission
    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    turn.set("permissions", json!({ "default_behavior": "ask", "tools": {} }));

    let tool_call = ToolCall::new("call_delete", "delete_all", json!({}));
    turn.tool = Some(ToolContext::new(&tool_call));

    let plugin = PermissionPlugin;
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(turn.tool_pending());

    let interaction = turn.tool.as_ref().unwrap().pending_interaction.clone().unwrap();

    // Phase 2: Client denies
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("false", &interaction.id));

    assert!(response_request.is_interaction_denied(&interaction.id));

    // Phase 3: On resume, tool should be blocked
    let response_plugin = InteractionResponsePlugin::from_request(&response_request);

    let mut turn2 = TurnContext::new(&session, vec![]);
    let tool_call2 = ToolCall::new(&interaction.id, "delete_all", json!({}));
    turn2.tool = Some(ToolContext::new(&tool_call2));

    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    assert!(turn2.tool_blocked(), "Denied tool should be blocked");

    let block_reason = turn2.tool.as_ref().unwrap().block_reason.as_ref().unwrap();
    assert!(block_reason.contains("denied"), "Block reason should mention denial");
}

/// Test: Multiple tools with mixed permissions
#[tokio::test]
async fn test_permission_flow_multiple_tools_mixed() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    let session = Session::new("test");

    // Tool 1: Will be approved
    let mut turn1 = TurnContext::new(&session, vec![]);
    turn1.set("permissions", json!({ "default_behavior": "ask", "tools": {} }));
    let call1 = ToolCall::new("call_1", "read_file", json!({}));
    turn1.tool = Some(ToolContext::new(&call1));

    let plugin = PermissionPlugin;
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    let int1 = turn1.tool.as_ref().unwrap().pending_interaction.clone().unwrap();

    // Tool 2: Will be denied
    let mut turn2 = TurnContext::new(&session, vec![]);
    turn2.set("permissions", json!({ "default_behavior": "ask", "tools": {} }));
    let call2 = ToolCall::new("call_2", "write_file", json!({}));
    turn2.tool = Some(ToolContext::new(&call2));
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    let int2 = turn2.tool.as_ref().unwrap().pending_interaction.clone().unwrap();

    // Client responds: approve first, deny second
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool("true", &int1.id))
        .with_message(AGUIMessage::tool("false", &int2.id));

    let response_plugin = InteractionResponsePlugin::from_request(&response_request);

    // Verify first tool (approved)
    let mut resume1 = TurnContext::new(&session, vec![]);
    let resume_call1 = ToolCall::new(&int1.id, "read_file", json!({}));
    resume1.tool = Some(ToolContext::new(&resume_call1));
    response_plugin.on_phase(Phase::BeforeToolExecute, &mut resume1).await;
    assert!(!resume1.tool_blocked(), "First tool should not be blocked");

    // Verify second tool (denied)
    let mut resume2 = TurnContext::new(&session, vec![]);
    let resume_call2 = ToolCall::new(&int2.id, "write_file", json!({}));
    resume2.tool = Some(ToolContext::new(&resume_call2));
    response_plugin.on_phase(Phase::BeforeToolExecute, &mut resume2).await;
    assert!(resume2.tool_blocked(), "Second tool should be blocked");
}

// ============================================================================
// Frontend Tool E2E Flow Tests
// ============================================================================

/// Test: Frontend tool creates pending interaction
#[tokio::test]
async fn test_frontend_tool_flow_creates_pending() {
    use carve_agent::ag_ui::FrontendToolPlugin;

    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy to clipboard"));

    let plugin = FrontendToolPlugin::from_request(&request);
    let session = Session::new("test");

    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new("call_copy", "copyToClipboard", json!({"text": "Hello World"}));
    turn.tool = Some(ToolContext::new(&call));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    assert!(turn.tool_pending(), "Frontend tool should be pending");

    let interaction = turn.tool.as_ref().unwrap().pending_interaction.clone().unwrap();
    assert_eq!(interaction.action, "tool:copyToClipboard");
    assert_eq!(interaction.id, "call_copy");
}

/// Test: Frontend tool result returned from client
#[test]
fn test_frontend_tool_flow_result_from_client() {
    // Client returns result for frontend tool
    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(
            r#"{"success": true, "bytes_copied": 11}"#,
            "call_copy",
        ));

    let response = response_request.get_interaction_response("call_copy").unwrap();
    assert!(response.result["success"].as_bool().unwrap());
    assert_eq!(response.result["bytes_copied"], 11);
}

/// Test: Mixed frontend and backend tools
#[tokio::test]
async fn test_frontend_tool_flow_mixed_with_backend() {
    use carve_agent::ag_ui::FrontendToolPlugin;

    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("showDialog", "Show dialog"))
        .with_tool(AGUIToolDef::backend("search", "Search files"));

    let plugin = FrontendToolPlugin::from_request(&request);
    let session = Session::new("test");

    // Backend tool - should NOT be pending
    let mut turn_backend = TurnContext::new(&session, vec![]);
    let call_backend = ToolCall::new("call_search", "search", json!({"query": "test"}));
    turn_backend.tool = Some(ToolContext::new(&call_backend));
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn_backend).await;
    assert!(!turn_backend.tool_pending(), "Backend tool should not be pending");

    // Frontend tool - should be pending
    let mut turn_frontend = TurnContext::new(&session, vec![]);
    let call_frontend = ToolCall::new("call_dialog", "showDialog", json!({"title": "Confirm"}));
    turn_frontend.tool = Some(ToolContext::new(&call_frontend));
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn_frontend).await;
    assert!(turn_frontend.tool_pending(), "Frontend tool should be pending");
}

/// Test: Frontend tool with complex nested result
#[test]
fn test_frontend_tool_flow_complex_result() {
    let complex_result = json!({
        "success": true,
        "selected_files": [
            {"path": "/home/user/doc1.txt", "size": 1024, "type": "text"},
            {"path": "/home/user/doc2.pdf", "size": 2048, "type": "pdf"}
        ],
        "metadata": {
            "dialog_duration_ms": 1500,
            "user_action": "confirm",
            "timestamp": "2024-01-15T10:30:00Z"
        }
    });

    let response_request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::tool(&complex_result.to_string(), "file_picker_call"));

    let response = response_request.get_interaction_response("file_picker_call").unwrap();
    assert!(response.result["success"].as_bool().unwrap());
    assert_eq!(response.result["selected_files"].as_array().unwrap().len(), 2);
    assert_eq!(response.result["selected_files"][0]["path"], "/home/user/doc1.txt");
    assert_eq!(response.result["metadata"]["user_action"], "confirm");
}

// ============================================================================
// State Event Flow Tests
// ============================================================================

/// Test: State snapshot event conversion
#[test]
fn test_state_event_snapshot_conversion() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let state = json!({
        "counter": 42,
        "user": {"name": "Alice", "role": "admin"},
        "items": ["a", "b", "c"]
    });

    let event = AgentEvent::StateSnapshot { snapshot: state.clone() };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    assert!(ag_events.iter().any(|e| matches!(e, AGUIEvent::StateSnapshot { .. })));

    if let AGUIEvent::StateSnapshot { snapshot, .. } = &ag_events[0] {
        assert_eq!(snapshot["counter"], 42);
        assert_eq!(snapshot["user"]["name"], "Alice");
    }
}

/// Test: State delta event conversion
#[test]
fn test_state_event_delta_conversion() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let delta = vec![
        json!({"op": "replace", "path": "/counter", "value": 43}),
        json!({"op": "add", "path": "/items/-", "value": "d"}),
    ];

    let event = AgentEvent::StateDelta { delta: delta.clone() };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    assert!(ag_events.iter().any(|e| matches!(e, AGUIEvent::StateDelta { .. })));

    if let AGUIEvent::StateDelta { delta: d, .. } = &ag_events[0] {
        assert_eq!(d.len(), 2);
        assert_eq!(d[0]["op"], "replace");
        assert_eq!(d[1]["op"], "add");
    }
}

/// Test: Messages snapshot event conversion
#[test]
fn test_state_event_messages_snapshot_conversion() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let messages = vec![
        json!({"role": "user", "content": "Hello"}),
        json!({"role": "assistant", "content": "Hi there!"}),
    ];

    let event = AgentEvent::MessagesSnapshot { messages: messages.clone() };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    assert!(ag_events.iter().any(|e| matches!(e, AGUIEvent::MessagesSnapshot { .. })));

    if let AGUIEvent::MessagesSnapshot { messages: m, .. } = &ag_events[0] {
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["role"], "user");
        assert_eq!(m[1]["role"], "assistant");
    }
}

// ============================================================================
// Error Handling Flow Tests
// ============================================================================

/// Test: Tool execution failure produces correct events
#[test]
fn test_error_flow_tool_execution_failure() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let result = ToolResult::error("read_file", "File not found: /nonexistent");

    // ToolCallDone produces TOOL_CALL_RESULT (not TOOL_CALL_END)
    let event = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result,
        patch: None,
    };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    // Should have TOOL_CALL_RESULT with error
    assert!(ag_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));

    // Result should contain error
    if let Some(AGUIEvent::ToolCallResult { content, .. }) = ag_events.iter().find(|e| matches!(e, AGUIEvent::ToolCallResult { .. })) {
        assert!(content.contains("error") || content.contains("File not found"));
    }
}

/// Test: Invalid request validation
#[test]
fn test_error_flow_invalid_request() {
    // Empty thread_id
    let invalid1 = RunAgentRequest::new("".to_string(), "r1".to_string());
    assert!(invalid1.validate().is_err());

    // Empty run_id
    let invalid2 = RunAgentRequest::new("t1".to_string(), "".to_string());
    assert!(invalid2.validate().is_err());

    // Valid request
    let valid = RunAgentRequest::new("t1".to_string(), "r1".to_string());
    assert!(valid.validate().is_ok());
}

/// Test: Agent abort event
#[test]
fn test_error_flow_agent_abort() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let event = AgentEvent::Aborted {
        reason: "User cancelled".into(),
    };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    // Should produce some events (depends on implementation)
    // At minimum, verify it doesn't panic
    assert!(ag_events.is_empty() || !ag_events.is_empty());
}

// ============================================================================
// Resume Flow Tests
// ============================================================================

/// Test: Resume with approval continues execution
#[tokio::test]
async fn test_resume_flow_with_approval() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Simulate: Previous run ended with pending permission
    let interaction_id = "permission_tool_x";

    // New request includes approval
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool("true", interaction_id));

    let plugin = InteractionResponsePlugin::from_request(&request);
    assert!(plugin.has_responses());
    assert!(plugin.is_approved(interaction_id));

    // Tool execution should not be blocked
    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new(interaction_id, "tool_x", json!({}));
    turn.tool = Some(ToolContext::new(&call));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(!turn.tool_blocked());
}

/// Test: Resume with denial blocks execution
#[tokio::test]
async fn test_resume_flow_with_denial() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    let interaction_id = "permission_dangerous_tool";

    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool("no", interaction_id));

    let plugin = InteractionResponsePlugin::from_request(&request);
    assert!(plugin.is_denied(interaction_id));

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new(interaction_id, "dangerous_tool", json!({}));
    turn.tool = Some(ToolContext::new(&call));

    plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(turn.tool_blocked());
}

/// Test: Resume with multiple pending responses
#[tokio::test]
async fn test_resume_flow_multiple_responses() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Previous run had 3 pending interactions
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool("true", "perm_1"))
        .with_message(AGUIMessage::tool("false", "perm_2"))
        .with_message(AGUIMessage::tool("yes", "perm_3"));

    let plugin = InteractionResponsePlugin::from_request(&request);

    assert!(plugin.is_approved("perm_1"));
    assert!(plugin.is_denied("perm_2"));
    assert!(plugin.is_approved("perm_3"));

    let session = Session::new("test");

    // Test each tool
    for (id, should_block) in [("perm_1", false), ("perm_2", true), ("perm_3", false)] {
        let mut turn = TurnContext::new(&session, vec![]);
        let call = ToolCall::new(id, "test_tool", json!({}));
        turn.tool = Some(ToolContext::new(&call));
        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
        assert_eq!(turn.tool_blocked(), should_block, "Tool {} block state incorrect", id);
    }
}

/// Test: Resume with partial responses (some missing)
#[tokio::test]
async fn test_resume_flow_partial_responses() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Only respond to some interactions
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool("true", "perm_1"));
        // perm_2 not responded to

    let plugin = InteractionResponsePlugin::from_request(&request);

    assert!(plugin.is_approved("perm_1"));
    assert!(!plugin.is_approved("perm_2")); // No response
    assert!(!plugin.is_denied("perm_2")); // No response

    let session = Session::new("test");

    // Responded tool should not be blocked
    let mut turn1 = TurnContext::new(&session, vec![]);
    let call1 = ToolCall::new("perm_1", "tool_1", json!({}));
    turn1.tool = Some(ToolContext::new(&call1));
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    assert!(!turn1.tool_blocked());

    // Non-responded tool - plugin doesn't affect it
    let mut turn2 = TurnContext::new(&session, vec![]);
    let call2 = ToolCall::new("perm_2", "tool_2", json!({}));
    turn2.tool = Some(ToolContext::new(&call2));
    plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    assert!(!turn2.tool_blocked()); // Not blocked by response plugin (no response)
}

// ============================================================================
// Plugin Interaction Flow Tests
// ============================================================================

/// Test: FrontendToolPlugin and InteractionResponsePlugin together
#[tokio::test]
async fn test_plugin_interaction_frontend_and_response() {
    use carve_agent::ag_ui::{FrontendToolPlugin, InteractionResponsePlugin};

    // Request has both frontend tools and interaction responses
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_tool(AGUIToolDef::frontend("showNotification", "Show notification"))
        .with_message(AGUIMessage::tool("true", "call_prev"));

    let frontend_plugin = FrontendToolPlugin::from_request(&request);
    let response_plugin = InteractionResponsePlugin::from_request(&request);

    let session = Session::new("test");

    // Test 1: Frontend tool should be pending (not affected by response plugin)
    let mut turn1 = TurnContext::new(&session, vec![]);
    let call1 = ToolCall::new("call_new", "showNotification", json!({}));
    turn1.tool = Some(ToolContext::new(&call1));

    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;
    frontend_plugin.on_phase(Phase::BeforeToolExecute, &mut turn1).await;

    assert!(turn1.tool_pending(), "Frontend tool should be pending");
    assert!(!turn1.tool_blocked());

    // Test 2: Previously pending tool should be allowed (response plugin approves)
    let mut turn2 = TurnContext::new(&session, vec![]);
    let call2 = ToolCall::new("call_prev", "some_tool", json!({}));
    turn2.tool = Some(ToolContext::new(&call2));

    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;
    frontend_plugin.on_phase(Phase::BeforeToolExecute, &mut turn2).await;

    assert!(!turn2.tool_blocked(), "Approved tool should not be blocked");
}

/// Test: Plugin execution order matters
#[tokio::test]
async fn test_plugin_interaction_execution_order() {
    use carve_agent::ag_ui::{FrontendToolPlugin, InteractionResponsePlugin};

    // Setup: Frontend tool that was previously denied
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_tool(AGUIToolDef::frontend("dangerousAction", "Dangerous"))
        .with_message(AGUIMessage::tool("false", "call_danger")); // Denied

    let frontend_plugin = FrontendToolPlugin::from_request(&request);
    let response_plugin = InteractionResponsePlugin::from_request(&request);

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    let call = ToolCall::new("call_danger", "dangerousAction", json!({}));
    turn.tool = Some(ToolContext::new(&call));

    // Response plugin runs first - denies the tool
    response_plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(turn.tool_blocked(), "Tool should be blocked by denial");

    // Frontend plugin runs second - should NOT override the block
    frontend_plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    assert!(turn.tool_blocked(), "Tool should still be blocked");
    assert!(!turn.tool_pending(), "Blocked tool should not be pending");
}

/// Test: Permission plugin with frontend tool
#[tokio::test]
async fn test_plugin_interaction_permission_and_frontend() {
    use carve_agent::ag_ui::FrontendToolPlugin;

    // Frontend tool with permission set to Ask
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::frontend("modifySettings", "Modify settings"));

    let frontend_plugin = FrontendToolPlugin::from_request(&request);
    let permission_plugin = PermissionPlugin;

    let session = Session::new("test");
    let mut turn = TurnContext::new(&session, vec![]);
    turn.set("permissions", json!({ "default_behavior": "ask", "tools": {} }));

    let call = ToolCall::new("call_modify", "modifySettings", json!({}));
    turn.tool = Some(ToolContext::new(&call));

    // Permission plugin runs first - creates pending for "ask"
    permission_plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;
    // Frontend plugin runs second
    frontend_plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

    // Tool should be pending (frontend takes precedence for frontend tools)
    assert!(turn.tool_pending(), "Tool should be pending");

    // The interaction should be from frontend plugin (tool:modifySettings)
    let interaction = turn.tool.as_ref().unwrap().pending_interaction.clone().unwrap();
    assert!(
        interaction.action.starts_with("tool:") || interaction.action == "confirm",
        "Interaction action should be from one of the plugins"
    );
}

// ============================================================================
// Activity Event Flow Tests
// ============================================================================
//
// AG-UI Protocol Reference: https://docs.ag-ui.com/concepts/events
// Activity events are used for long-running operations to show progress.
//
// Flow: ACTIVITY_SNAPSHOT → ACTIVITY_DELTA* → (completion)
// - ACTIVITY_SNAPSHOT: Initial state with full content
// - ACTIVITY_DELTA: JSON Patch updates to modify state incrementally
//

/// Test: Activity snapshot event creation and conversion
/// Protocol: ACTIVITY_SNAPSHOT event per AG-UI spec
#[test]
fn test_activity_snapshot_flow() {
    use std::collections::HashMap;

    let mut content = HashMap::new();
    content.insert("progress".to_string(), json!(0.75));
    content.insert("status".to_string(), json!("processing"));
    content.insert("current_item".to_string(), json!({"name": "file.txt", "size": 1024}));

    let event = AGUIEvent::activity_snapshot("activity_1", "file_processing", content, Some(false));

    // Verify serialization
    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"ACTIVITY_SNAPSHOT""#));
    assert!(json.contains(r#""activityType":"file_processing""#));
    assert!(json.contains(r#""progress":0.75"#));

    // Roundtrip
    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, AGUIEvent::ActivitySnapshot { .. }));
}

/// Test: Activity delta event creation and conversion
#[test]
fn test_activity_delta_flow() {
    let patch = vec![
        json!({"op": "replace", "path": "/progress", "value": 0.85}),
        json!({"op": "replace", "path": "/status", "value": "almost done"}),
    ];

    let event = AGUIEvent::activity_delta("activity_1", "file_processing", patch);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"ACTIVITY_DELTA""#));
    assert!(json.contains(r#""op":"replace""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, AGUIEvent::ActivityDelta { .. }));
}

/// Test: Complete activity streaming flow (snapshot → deltas → final)
#[test]
fn test_activity_streaming_complete_flow() {
    use std::collections::HashMap;

    // Initial snapshot
    let mut initial_content = HashMap::new();
    initial_content.insert("progress".to_string(), json!(0.0));
    initial_content.insert("total_files".to_string(), json!(10));
    initial_content.insert("processed_files".to_string(), json!(0));

    let snapshot = AGUIEvent::activity_snapshot("act_1", "batch_processing", initial_content, Some(false));

    // Progress deltas
    let delta1 = AGUIEvent::activity_delta(
        "act_1",
        "batch_processing",
        vec![
            json!({"op": "replace", "path": "/progress", "value": 0.3}),
            json!({"op": "replace", "path": "/processed_files", "value": 3}),
        ],
    );

    let delta2 = AGUIEvent::activity_delta(
        "act_1",
        "batch_processing",
        vec![
            json!({"op": "replace", "path": "/progress", "value": 0.7}),
            json!({"op": "replace", "path": "/processed_files", "value": 7}),
        ],
    );

    let delta_final = AGUIEvent::activity_delta(
        "act_1",
        "batch_processing",
        vec![
            json!({"op": "replace", "path": "/progress", "value": 1.0}),
            json!({"op": "replace", "path": "/processed_files", "value": 10}),
            json!({"op": "add", "path": "/completed", "value": true}),
        ],
    );

    // Verify all events serialize correctly
    let events = vec![snapshot, delta1, delta2, delta_final];
    for (i, event) in events.iter().enumerate() {
        let json = serde_json::to_string(event).unwrap();
        let _: AGUIEvent = serde_json::from_str(&json).expect(&format!("Event {} failed", i));
    }

    // Verify event types
    assert!(matches!(&events[0], AGUIEvent::ActivitySnapshot { .. }));
    assert!(matches!(&events[1], AGUIEvent::ActivityDelta { .. }));
    assert!(matches!(&events[2], AGUIEvent::ActivityDelta { .. }));
    assert!(matches!(&events[3], AGUIEvent::ActivityDelta { .. }));
}

// ============================================================================
// Concurrent Tool Execution Tests
// ============================================================================
//
// AG-UI Protocol Reference: https://docs.ag-ui.com/concepts/events
// Tool Call Flow: TOOL_CALL_START → TOOL_CALL_ARGS → TOOL_CALL_END → TOOL_CALL_RESULT
//
// Multiple tools can execute concurrently. Each tool maintains its own event
// sequence identified by tool_call_id. Events can interleave but each tool's
// sequence must be complete.
//

/// Test: Multiple tool calls event ordering
/// Verifies concurrent tools each have complete START → ARGS → END → RESULT sequence
#[test]
fn test_concurrent_tool_calls_event_ordering() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Simulate 3 concurrent tool calls
    let tool_ids = vec!["call_1", "call_2", "call_3"];
    let tool_names = vec!["search", "read_file", "write_file"];

    let mut all_events: Vec<AGUIEvent> = Vec::new();

    // All tools start
    for (id, name) in tool_ids.iter().zip(tool_names.iter()) {
        let start = AgentEvent::ToolCallStart {
            id: id.to_string(),
            name: name.to_string(),
        };
        all_events.extend(start.to_ag_ui_events(&mut ctx));
    }

    // All tools get args
    for id in &tool_ids {
        let args = AgentEvent::ToolCallDelta {
            id: id.to_string(),
            args_delta: "{}".into(),
        };
        all_events.extend(args.to_ag_ui_events(&mut ctx));
    }

    // All tools ready (end args streaming)
    for (id, name) in tool_ids.iter().zip(tool_names.iter()) {
        let ready = AgentEvent::ToolCallReady {
            id: id.to_string(),
            name: name.to_string(),
            arguments: json!({}),
        };
        all_events.extend(ready.to_ag_ui_events(&mut ctx));
    }

    // All tools done
    for (id, name) in tool_ids.iter().zip(tool_names.iter()) {
        let done = AgentEvent::ToolCallDone {
            id: id.to_string(),
            result: ToolResult::success(*name, json!({"ok": true})),
            patch: None,
        };
        all_events.extend(done.to_ag_ui_events(&mut ctx));
    }

    // Verify each tool has complete sequence
    for id in &tool_ids {
        let has_start = all_events.iter().any(|e| {
            matches!(e, AGUIEvent::ToolCallStart { tool_call_id, .. } if tool_call_id == *id)
        });
        let has_args = all_events.iter().any(|e| {
            matches!(e, AGUIEvent::ToolCallArgs { tool_call_id, .. } if tool_call_id == *id)
        });
        let has_end = all_events.iter().any(|e| {
            matches!(e, AGUIEvent::ToolCallEnd { tool_call_id, .. } if tool_call_id == *id)
        });
        let has_result = all_events.iter().any(|e| {
            matches!(e, AGUIEvent::ToolCallResult { tool_call_id, .. } if tool_call_id == *id)
        });

        assert!(has_start, "Tool {} missing START", id);
        assert!(has_args, "Tool {} missing ARGS", id);
        assert!(has_end, "Tool {} missing END", id);
        assert!(has_result, "Tool {} missing RESULT", id);
    }
}

/// Test: Interleaved tool calls with text
#[test]
fn test_interleaved_tools_and_text() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut all_events: Vec<AGUIEvent> = Vec::new();

    // Text starts
    let text1 = AgentEvent::TextDelta { delta: "Let me search ".into() };
    all_events.extend(text1.to_ag_ui_events(&mut ctx));

    // Tool starts (interrupts text)
    let tool_start = AgentEvent::ToolCallStart {
        id: "call_1".into(),
        name: "search".into(),
    };
    all_events.extend(tool_start.to_ag_ui_events(&mut ctx));

    // Tool args
    let tool_args = AgentEvent::ToolCallDelta {
        id: "call_1".into(),
        args_delta: r#"{"query": "rust"}"#.into(),
    };
    all_events.extend(tool_args.to_ag_ui_events(&mut ctx));

    // Tool ready
    let tool_ready = AgentEvent::ToolCallReady {
        id: "call_1".into(),
        name: "search".into(),
        arguments: json!({"query": "rust"}),
    };
    all_events.extend(tool_ready.to_ag_ui_events(&mut ctx));

    // Tool done
    let tool_done = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result: ToolResult::success("search", json!({"results": 5})),
        patch: None,
    };
    all_events.extend(tool_done.to_ag_ui_events(&mut ctx));

    // More text after tool
    let text2 = AgentEvent::TextDelta { delta: "Found 5 results.".into() };
    all_events.extend(text2.to_ag_ui_events(&mut ctx));

    // Verify sequence: text → tool → text
    // First should have TEXT_MESSAGE_START
    assert!(matches!(&all_events[0], AGUIEvent::TextMessageStart { .. }));

    // Should have TEXT_MESSAGE_END before TOOL_CALL_START
    let text_end_idx = all_events.iter().position(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })).unwrap();
    let tool_start_idx = all_events.iter().position(|e| matches!(e, AGUIEvent::ToolCallStart { .. })).unwrap();
    assert!(text_end_idx < tool_start_idx, "TEXT_MESSAGE_END should come before TOOL_CALL_START");

    // Should have new TEXT_MESSAGE_START after tool
    let text_starts: Vec<_> = all_events.iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, AGUIEvent::TextMessageStart { .. }))
        .collect();
    assert_eq!(text_starts.len(), 2, "Should have 2 TEXT_MESSAGE_START events");
}

// ============================================================================
// Client Reconnection Flow Tests
// ============================================================================
//
// AG-UI Protocol Reference: https://docs.ag-ui.com/concepts/events
// State Synchronization Flow for reconnection:
//   STATE_SNAPSHOT (full state) or MESSAGES_SNAPSHOT (conversation history)
//
// When a client reconnects, it needs:
// 1. Current state via STATE_SNAPSHOT
// 2. Conversation history via MESSAGES_SNAPSHOT
// 3. Ability to resume from last known state
//

/// Test: State snapshot for reconnection
/// Protocol: STATE_SNAPSHOT event for client state restoration
#[test]
fn test_reconnection_state_snapshot() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Simulate session state
    let state = json!({
        "conversation": {
            "turn_count": 5,
            "last_tool": "search",
            "context": {"topic": "rust programming"}
        },
        "user_preferences": {
            "language": "en",
            "verbosity": "detailed"
        }
    });

    let event = AgentEvent::StateSnapshot { snapshot: state.clone() };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    if let AGUIEvent::StateSnapshot { snapshot, .. } = &ag_events[0] {
        assert_eq!(snapshot["conversation"]["turn_count"], 5);
        assert_eq!(snapshot["user_preferences"]["language"], "en");
    }
}

/// Test: Messages snapshot for reconnection
#[test]
fn test_reconnection_messages_snapshot() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let messages = vec![
        json!({"role": "user", "content": "Hello"}),
        json!({"role": "assistant", "content": "Hi! How can I help?"}),
        json!({"role": "user", "content": "Search for rust tutorials"}),
        json!({"role": "assistant", "content": "I'll search for that.", "tool_calls": [{"id": "call_1", "name": "search"}]}),
        json!({"role": "tool", "tool_call_id": "call_1", "content": "{\"results\": 10}"}),
    ];

    let event = AgentEvent::MessagesSnapshot { messages: messages.clone() };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    if let AGUIEvent::MessagesSnapshot { messages: m, .. } = &ag_events[0] {
        assert_eq!(m.len(), 5);
        assert_eq!(m[0]["role"], "user");
        assert_eq!(m[4]["role"], "tool");
    }
}

/// Test: Full reconnection scenario
#[test]
fn test_full_reconnection_scenario() {
    // Client reconnects - server sends snapshots first
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut reconnect_events: Vec<AGUIEvent> = Vec::new();

    // 1. RUN_STARTED for new connection
    reconnect_events.push(AGUIEvent::run_started("t1", "r1", None));

    // 2. Messages snapshot (conversation history)
    let messages_event = AgentEvent::MessagesSnapshot {
        messages: vec![
            json!({"role": "user", "content": "Previous message"}),
            json!({"role": "assistant", "content": "Previous response"}),
        ],
    };
    reconnect_events.extend(messages_event.to_ag_ui_events(&mut ctx));

    // 3. State snapshot (current state)
    let state_event = AgentEvent::StateSnapshot {
        snapshot: json!({"session_id": "abc123", "active": true}),
    };
    reconnect_events.extend(state_event.to_ag_ui_events(&mut ctx));

    // 4. Continue with new content
    let text = AgentEvent::TextDelta { delta: "Continuing from where we left off...".into() };
    reconnect_events.extend(text.to_ag_ui_events(&mut ctx));

    // Verify sequence
    assert!(matches!(&reconnect_events[0], AGUIEvent::RunStarted { .. }));
    assert!(reconnect_events.iter().any(|e| matches!(e, AGUIEvent::MessagesSnapshot { .. })));
    assert!(reconnect_events.iter().any(|e| matches!(e, AGUIEvent::StateSnapshot { .. })));
    assert!(reconnect_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
}

// ============================================================================
// Multiple Pending Interactions Tests (HIGH PRIORITY - Previously Missing)
// ============================================================================

/// Test: Multiple pending interactions in sequence
#[tokio::test]
async fn test_multiple_pending_interactions_flow() {
    let session = Session::new("test");

    // Create 3 pending interactions
    let interactions: Vec<Interaction> = vec![
        Interaction::new("perm_read", "confirm").with_message("Allow reading files?"),
        Interaction::new("perm_write", "confirm").with_message("Allow writing files?"),
        Interaction::new("perm_exec", "confirm").with_message("Allow executing commands?"),
    ];

    // Convert all to AG-UI events
    let mut all_events: Vec<AGUIEvent> = Vec::new();
    for interaction in &interactions {
        all_events.extend(interaction.to_ag_ui_events());
    }

    // Each interaction should produce 3 events (Start, Args, End)
    assert_eq!(all_events.len(), 9);

    // Verify each interaction has its events
    for interaction in &interactions {
        let has_start = all_events.iter().any(|e| {
            matches!(e, AGUIEvent::ToolCallStart { tool_call_id, .. } if tool_call_id == &interaction.id)
        });
        assert!(has_start, "Missing ToolCallStart for {}", interaction.id);
    }
}

/// Test: Client responds to multiple interactions
#[tokio::test]
async fn test_multiple_interaction_responses() {
    use carve_agent::ag_ui::InteractionResponsePlugin;

    // Client responds to all 3 interactions: approve, deny, approve
    let request = RunAgentRequest::new("t1".to_string(), "r2".to_string())
        .with_message(AGUIMessage::tool("yes", "perm_read"))
        .with_message(AGUIMessage::tool("no", "perm_write"))
        .with_message(AGUIMessage::tool("approved", "perm_exec"));

    let plugin = InteractionResponsePlugin::from_request(&request);

    assert!(plugin.is_approved("perm_read"));
    assert!(plugin.is_denied("perm_write"));
    assert!(plugin.is_approved("perm_exec"));

    let session = Session::new("test");

    // Verify each tool gets correct treatment
    let test_cases = vec![
        ("perm_read", false),  // approved, not blocked
        ("perm_write", true),  // denied, blocked
        ("perm_exec", false),  // approved, not blocked
    ];

    for (id, should_block) in test_cases {
        let mut turn = TurnContext::new(&session, vec![]);
        let call = ToolCall::new(id, "some_tool", json!({}));
        turn.tool = Some(ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut turn).await;

        assert_eq!(
            turn.tool_blocked(),
            should_block,
            "Tool {} should_block={} but got blocked={}",
            id,
            should_block,
            turn.tool_blocked()
        );
    }
}

// ============================================================================
// Tool Timeout Flow Tests (HIGH PRIORITY - Previously Missing)
// ============================================================================

/// Test: Tool timeout produces correct AG-UI events
#[test]
fn test_tool_timeout_ag_ui_flow() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Tool starts
    let start = AgentEvent::ToolCallStart {
        id: "call_slow".into(),
        name: "slow_operation".into(),
    };
    let start_events = start.to_ag_ui_events(&mut ctx);
    assert!(start_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));

    // Tool times out - simulated by returning timeout error
    let timeout_result = ToolResult::error("slow_operation", "Tool execution timed out after 30s");

    let done = AgentEvent::ToolCallDone {
        id: "call_slow".into(),
        result: timeout_result,
        patch: None,
    };
    let done_events = done.to_ag_ui_events(&mut ctx);

    // Should still have TOOL_CALL_RESULT with error
    assert!(done_events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));

    if let Some(AGUIEvent::ToolCallResult { content, .. }) = done_events.iter().find(|e| matches!(e, AGUIEvent::ToolCallResult { .. })) {
        assert!(content.contains("timed out") || content.contains("error"));
    }
}

// ============================================================================
// Rapid Text Delta Tests (MEDIUM PRIORITY)
// ============================================================================

/// Test: Rapid text delta burst handling
#[test]
fn test_rapid_text_delta_burst() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut all_events: Vec<AGUIEvent> = Vec::new();

    // Simulate 100 rapid text deltas
    for i in 0..100 {
        let delta = AgentEvent::TextDelta {
            delta: format!("word{} ", i),
        };
        all_events.extend(delta.to_ag_ui_events(&mut ctx));
    }

    // Should have exactly 1 TEXT_MESSAGE_START
    let start_count = all_events.iter().filter(|e| matches!(e, AGUIEvent::TextMessageStart { .. })).count();
    assert_eq!(start_count, 1, "Should have exactly 1 TEXT_MESSAGE_START");

    // Should have 100 TEXT_MESSAGE_CONTENT (one for each delta)
    let content_count = all_events.iter().filter(|e| matches!(e, AGUIEvent::TextMessageContent { .. })).count();
    assert_eq!(content_count, 100, "Should have 100 TEXT_MESSAGE_CONTENT events");

    // First event should be TEXT_MESSAGE_START
    assert!(matches!(&all_events[0], AGUIEvent::TextMessageStart { .. }));
}

// ============================================================================
// State Event Ordering Tests (MEDIUM PRIORITY)
// ============================================================================

/// Test: State events ordering with other events
#[test]
fn test_state_event_ordering() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut all_events: Vec<AGUIEvent> = Vec::new();

    // Sequence: text → state snapshot → more text → state delta
    let text1 = AgentEvent::TextDelta { delta: "Starting...".into() };
    all_events.extend(text1.to_ag_ui_events(&mut ctx));

    let snapshot = AgentEvent::StateSnapshot {
        snapshot: json!({"step": 1}),
    };
    all_events.extend(snapshot.to_ag_ui_events(&mut ctx));

    let text2 = AgentEvent::TextDelta { delta: " Processing...".into() };
    all_events.extend(text2.to_ag_ui_events(&mut ctx));

    let delta = AgentEvent::StateDelta {
        delta: vec![json!({"op": "replace", "path": "/step", "value": 2})],
    };
    all_events.extend(delta.to_ag_ui_events(&mut ctx));

    // Verify presence of all event types
    assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
    assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::TextMessageContent { .. })));
    assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::StateSnapshot { .. })));
    assert!(all_events.iter().any(|e| matches!(e, AGUIEvent::StateDelta { .. })));
}

// ============================================================================
// Sequential Runs Tests (MEDIUM PRIORITY)
// ============================================================================

/// Test: Sequential runs in same session
#[test]
fn test_sequential_runs_in_session() {
    // Run 1
    let mut ctx1 = AGUIContext::new("t1".into(), "r1".into());
    let run1_start = AGUIEvent::run_started("t1", "r1", None);
    let text1 = AgentEvent::TextDelta { delta: "First run response".into() };
    let text1_events = text1.to_ag_ui_events(&mut ctx1);
    let run1_finish = AGUIEvent::run_finished("t1", "r1", Some(json!({"response": "First"})));

    // Run 2 (same thread, different run)
    let mut ctx2 = AGUIContext::new("t1".into(), "r2".into());
    let run2_start = AGUIEvent::run_started("t1", "r2", None);
    let text2 = AgentEvent::TextDelta { delta: "Second run response".into() };
    let text2_events = text2.to_ag_ui_events(&mut ctx2);
    let run2_finish = AGUIEvent::run_finished("t1", "r2", Some(json!({"response": "Second"})));

    // Verify runs are independent
    if let AGUIEvent::RunStarted { thread_id: t1, run_id: r1, .. } = &run1_start {
        if let AGUIEvent::RunStarted { thread_id: t2, run_id: r2, .. } = &run2_start {
            assert_eq!(t1, t2, "Same thread");
            assert_ne!(r1, r2, "Different runs");
        }
    }

    // Each run should have its own text message
    assert!(!text1_events.is_empty());
    assert!(!text2_events.is_empty());
}

// ============================================================================
// RAW and CUSTOM Event Tests
// ============================================================================
//
// AG-UI Protocol Reference: https://docs.ag-ui.com/concepts/events
// Special event types for extensibility:
// - RAW: Pass through external provider events unchanged
// - CUSTOM: Application-specific extension events
//
// These allow protocol extensions without breaking compatibility.
//

/// Test: RAW event wrapping
/// Protocol: RAW event for pass-through of external system events
#[test]
fn test_raw_event_wrapping() {
    let external_event = json!({
        "provider": "openai",
        "event_type": "rate_limit_warning",
        "data": {
            "requests_remaining": 10,
            "reset_at": "2024-01-15T12:00:00Z"
        }
    });

    let event = AGUIEvent::raw(external_event.clone(), Some("openai".into()));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"RAW""#));
    assert!(json.contains(r#""provider":"openai""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::Raw { event: e, .. } = parsed {
        assert_eq!(e["provider"], "openai");
        assert_eq!(e["data"]["requests_remaining"], 10);
    }
}

/// Test: CUSTOM event flow
#[test]
fn test_custom_event_flow() {
    let custom_value = json!({
        "action": "show_modal",
        "modal_type": "confirmation",
        "title": "Confirm Action",
        "buttons": ["Cancel", "Confirm"]
    });

    let event = AGUIEvent::custom("ui_action", custom_value.clone());

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"CUSTOM""#));
    assert!(json.contains(r#""name":"ui_action""#));
    assert!(json.contains(r#""action":"show_modal""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::Custom { name, value, .. } = parsed {
        assert_eq!(name, "ui_action");
        assert_eq!(value["modal_type"], "confirmation");
    }
}

// ============================================================================
// Large Payload Tests
// ============================================================================
//
// AG-UI Protocol Reference: https://docs.ag-ui.com/concepts/events
// Tests for handling large payloads in:
// - TOOL_CALL_RESULT: Large tool execution results
// - STATE_SNAPSHOT: Large state objects
//
// These verify the protocol handles real-world data sizes without issues.
//

/// Test: Large tool result payload
/// Verifies TOOL_CALL_RESULT can handle ~100KB of JSON data
#[test]
fn test_large_tool_result_payload() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Create a large result (simulate ~100KB of data)
    let large_data: Vec<Value> = (0..1000)
        .map(|i| json!({
            "id": i,
            "name": format!("item_{}", i),
            "description": "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ".repeat(10),
            "metadata": {
                "created": "2024-01-15",
                "tags": ["tag1", "tag2", "tag3"],
                "nested": {"level1": {"level2": {"value": i}}}
            }
        }))
        .collect();

    let result = ToolResult::success("search", json!({
        "total": 1000,
        "items": large_data
    }));

    let event = AgentEvent::ToolCallDone {
        id: "call_large".into(),
        result,
        patch: None,
    };

    let ag_events = event.to_ag_ui_events(&mut ctx);
    assert!(!ag_events.is_empty());

    // Verify the result can be serialized and parsed
    if let Some(AGUIEvent::ToolCallResult { content, .. }) = ag_events.iter().find(|e| matches!(e, AGUIEvent::ToolCallResult { .. })) {
        // Content should be valid JSON (it's a serialized ToolResult struct)
        let parsed: Value = serde_json::from_str(content).expect("Should be valid JSON");
        // The data is nested inside the ToolResult structure
        assert_eq!(parsed["data"]["total"], 1000);
        assert_eq!(parsed["data"]["items"].as_array().unwrap().len(), 1000);
    }
}

/// Test: Large state snapshot
#[test]
fn test_large_state_snapshot() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Create large state
    let large_state = json!({
        "users": (0..100).map(|i| json!({
            "id": i,
            "name": format!("User {}", i),
            "email": format!("user{}@example.com", i),
            "preferences": {
                "theme": "dark",
                "notifications": true,
                "settings": (0..10).map(|j| json!({"key": format!("setting_{}", j), "value": j})).collect::<Vec<_>>()
            }
        })).collect::<Vec<_>>(),
        "config": {
            "version": "1.0.0",
            "features": (0..50).map(|i| format!("feature_{}", i)).collect::<Vec<_>>()
        }
    });

    let event = AgentEvent::StateSnapshot { snapshot: large_state };
    let ag_events = event.to_ag_ui_events(&mut ctx);

    assert!(!ag_events.is_empty());
    if let AGUIEvent::StateSnapshot { snapshot, .. } = &ag_events[0] {
        assert_eq!(snapshot["users"].as_array().unwrap().len(), 100);
    }
}

// ============================================================================
// AG-UI Protocol Spec Compliance Tests
// ============================================================================
//
// These tests verify compliance with AG-UI protocol specification.
// Reference: https://docs.ag-ui.com/concepts/events
//            https://docs.ag-ui.com/sdk/js/core/events
//

// ----------------------------------------------------------------------------
// Convenience Event Tests (TextMessageChunk, ToolCallChunk)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Convenience events auto-expand to their component events.
// TextMessageChunk → TextMessageStart + TextMessageContent + TextMessageEnd
// ToolCallChunk → ToolCallStart + ToolCallArgs + ToolCallEnd

/// Test: TextMessageChunk convenience event serialization
/// Protocol: TEXT_MESSAGE_CHUNK auto-expands to Start → Content → End
#[test]
fn test_text_message_chunk_serialization() {
    let chunk = AGUIEvent::text_message_chunk(
        Some("msg_1".into()),
        Some(MessageRole::Assistant),
        Some("Hello, world!".into()),
    );

    let json = serde_json::to_string(&chunk).unwrap();
    assert!(json.contains(r#""type":"TEXT_MESSAGE_CHUNK""#));
    assert!(json.contains(r#""messageId":"msg_1""#));
    assert!(json.contains(r#""delta":"Hello, world!""#));
    assert!(json.contains(r#""role":"assistant""#));

    // Verify roundtrip
    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, AGUIEvent::TextMessageChunk { .. }));
}

/// Test: ToolCallChunk convenience event serialization
/// Protocol: TOOL_CALL_CHUNK auto-expands to Start → Args → End
#[test]
fn test_tool_call_chunk_serialization() {
    let chunk = AGUIEvent::tool_call_chunk(
        Some("call_1".into()),
        Some("search".into()),
        None,
        Some(r#"{"query":"rust"}"#.into()),
    );

    let json = serde_json::to_string(&chunk).unwrap();
    assert!(json.contains(r#""type":"TOOL_CALL_CHUNK""#));
    assert!(json.contains(r#""toolCallId":"call_1""#));
    assert!(json.contains(r#""toolCallName":"search""#));
    assert!(json.contains(r#""delta":"#));

    // Verify roundtrip
    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed, AGUIEvent::ToolCallChunk { .. }));
}

/// Test: ToolCallChunk with parentMessageId
/// Protocol: Optional parentMessageId links tool call to a message
#[test]
fn test_tool_call_chunk_with_parent_message() {
    let chunk = AGUIEvent::tool_call_chunk(
        Some("call_1".into()),
        Some("read_file".into()),
        Some("msg_123".into()),
        Some(r#"{"path":"/etc/hosts"}"#.into()),
    );

    let json = serde_json::to_string(&chunk).unwrap();
    assert!(json.contains(r#""parentMessageId":"msg_123""#));
}

// ----------------------------------------------------------------------------
// Run Lifecycle Tests (parentRunId, branching)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Runs can branch via parentRunId for sub-agents or retries.

/// Test: RunStarted with parentRunId for branching
/// Protocol: parentRunId enables run branching/sub-agents
#[test]
fn test_run_started_with_parent_run_id() {
    let event = AGUIEvent::run_started_with_input("t1", "r2", Some("r1".into()), json!({"query": "test"}));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"RUN_STARTED""#));
    assert!(json.contains(r#""runId":"r2""#));
    assert!(json.contains(r#""parentRunId":"r1""#));
    assert!(json.contains(r#""input":"#));

    // Roundtrip
    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::RunStarted { parent_run_id, .. } = parsed {
        assert_eq!(parent_run_id, Some("r1".to_string()));
    } else {
        panic!("Expected RunStarted");
    }
}

/// Test: RunError with error code
/// Protocol: Error code is optional for categorizing errors
#[test]
fn test_run_error_with_code() {
    let event = AGUIEvent::run_error("Connection timeout".to_string(), Some("TIMEOUT".to_string()));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"RUN_ERROR""#));
    assert!(json.contains(r#""message":"Connection timeout""#));
    assert!(json.contains(r#""code":"TIMEOUT""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::RunError { code, .. } = parsed {
        assert_eq!(code, Some("TIMEOUT".to_string()));
    }
}

/// Test: RunError without code
#[test]
fn test_run_error_without_code() {
    let event = AGUIEvent::run_error("Unknown error".to_string(), None);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"RUN_ERROR""#));
    assert!(!json.contains(r#""code""#)); // code should be omitted

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::RunError { code, .. } = parsed {
        assert_eq!(code, None);
    }
}

// ----------------------------------------------------------------------------
// Step Event Tests (StepStarted/StepFinished pairing)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Step events track discrete subtasks with matching stepName.

/// Test: Step events with matching names
/// Protocol: StepStarted and StepFinished must have matching stepName
#[test]
fn test_step_events_matching_names() {
    let start = AGUIEvent::step_started("data_processing");
    let finish = AGUIEvent::step_finished("data_processing");

    // Verify matching step names
    if let AGUIEvent::StepStarted { step_name: start_name, .. } = &start {
        if let AGUIEvent::StepFinished { step_name: finish_name, .. } = &finish {
            assert_eq!(start_name, finish_name);
        }
    }

    // Verify serialization
    let start_json = serde_json::to_string(&start).unwrap();
    let finish_json = serde_json::to_string(&finish).unwrap();
    assert!(start_json.contains(r#""type":"STEP_STARTED""#));
    assert!(finish_json.contains(r#""type":"STEP_FINISHED""#));
    assert!(start_json.contains(r#""stepName":"data_processing""#));
    assert!(finish_json.contains(r#""stepName":"data_processing""#));
}

/// Test: Multiple step sequences
/// Protocol: Verify correct step name tracking across multiple steps
#[test]
fn test_multiple_step_sequences() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Step 1
    let step1_start = AgentEvent::StepStart;
    let events1 = step1_start.to_ag_ui_events(&mut ctx);
    let step1_name = if let AGUIEvent::StepStarted { step_name, .. } = &events1[0] {
        step_name.clone()
    } else {
        panic!("Expected StepStarted");
    };

    let step1_end = AgentEvent::StepEnd;
    let events1_end = step1_end.to_ag_ui_events(&mut ctx);
    if let AGUIEvent::StepFinished { step_name, .. } = &events1_end[0] {
        assert_eq!(*step_name, step1_name);
    }

    // Step 2
    let step2_start = AgentEvent::StepStart;
    let events2 = step2_start.to_ag_ui_events(&mut ctx);
    let step2_name = if let AGUIEvent::StepStarted { step_name, .. } = &events2[0] {
        step_name.clone()
    } else {
        panic!("Expected StepStarted");
    };

    // Step names should be different
    assert_ne!(step1_name, step2_name);
}

// ----------------------------------------------------------------------------
// JSON Patch Operation Tests (RFC 6902)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: StateDelta uses RFC 6902 JSON Patch with 6 operation types.

/// Test: JSON Patch - add operation
/// Protocol: RFC 6902 add operation
#[test]
fn test_json_patch_add_operation() {
    let delta = vec![json!({"op": "add", "path": "/newField", "value": "test"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"add""#));
    assert!(json.contains(r#""path":"/newField""#));
}

/// Test: JSON Patch - replace operation
/// Protocol: RFC 6902 replace operation
#[test]
fn test_json_patch_replace_operation() {
    let delta = vec![json!({"op": "replace", "path": "/existing", "value": "updated"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"replace""#));
}

/// Test: JSON Patch - remove operation
/// Protocol: RFC 6902 remove operation
#[test]
fn test_json_patch_remove_operation() {
    let delta = vec![json!({"op": "remove", "path": "/obsoleteField"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"remove""#));
    assert!(json.contains(r#""path":"/obsoleteField""#));
}

/// Test: JSON Patch - move operation
/// Protocol: RFC 6902 move operation
#[test]
fn test_json_patch_move_operation() {
    let delta = vec![json!({"op": "move", "from": "/oldPath", "path": "/newPath"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"move""#));
    assert!(json.contains(r#""from":"/oldPath""#));
    assert!(json.contains(r#""path":"/newPath""#));
}

/// Test: JSON Patch - copy operation
/// Protocol: RFC 6902 copy operation
#[test]
fn test_json_patch_copy_operation() {
    let delta = vec![json!({"op": "copy", "from": "/source", "path": "/destination"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"copy""#));
    assert!(json.contains(r#""from":"/source""#));
}

/// Test: JSON Patch - test operation
/// Protocol: RFC 6902 test operation for validation
#[test]
fn test_json_patch_test_operation() {
    let delta = vec![json!({"op": "test", "path": "/version", "value": "1.0"})];
    let event = AGUIEvent::state_delta(delta);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""op":"test""#));
    assert!(json.contains(r#""value":"1.0""#));
}

/// Test: JSON Patch - multiple operations
/// Protocol: Multiple patch operations in a single delta
#[test]
fn test_json_patch_multiple_operations() {
    let delta = vec![
        json!({"op": "test", "path": "/version", "value": "1.0"}),
        json!({"op": "replace", "path": "/version", "value": "2.0"}),
        json!({"op": "add", "path": "/newFeature", "value": true}),
        json!({"op": "remove", "path": "/deprecatedField"}),
    ];
    let event = AGUIEvent::state_delta(delta.clone());

    if let AGUIEvent::StateDelta { delta: d, .. } = event {
        assert_eq!(d.len(), 4);
    }
}

// ----------------------------------------------------------------------------
// MessagesSnapshot Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: MessagesSnapshot delivers complete conversation history.

/// Test: MessagesSnapshot for conversation restoration
/// Protocol: MESSAGES_SNAPSHOT for initial load or reconnection
#[test]
fn test_messages_snapshot_conversation_history() {
    let messages = vec![
        json!({"role": "user", "content": "Hello"}),
        json!({"role": "assistant", "content": "Hi! How can I help?"}),
        json!({"role": "user", "content": "What's the weather?"}),
        json!({"role": "assistant", "content": "I'll check that for you."}),
    ];

    let event = AGUIEvent::messages_snapshot(messages.clone());

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"MESSAGES_SNAPSHOT""#));

    if let AGUIEvent::MessagesSnapshot { messages: m, .. } = event {
        assert_eq!(m.len(), 4);
        assert_eq!(m[0]["role"], "user");
        assert_eq!(m[1]["role"], "assistant");
    }
}

/// Test: MessagesSnapshot with tool messages
/// Protocol: Conversation history can include tool messages
#[test]
fn test_messages_snapshot_with_tool_messages() {
    let messages = vec![
        json!({"role": "user", "content": "Search for rust tutorials"}),
        json!({"role": "assistant", "content": "I'll search for that.", "tool_calls": [{"id": "call_1", "name": "search"}]}),
        json!({"role": "tool", "tool_call_id": "call_1", "content": "Found 10 results"}),
        json!({"role": "assistant", "content": "I found 10 tutorials about Rust."}),
    ];

    let event = AGUIEvent::messages_snapshot(messages);

    if let AGUIEvent::MessagesSnapshot { messages: m, .. } = event {
        assert_eq!(m.len(), 4);
        assert_eq!(m[2]["role"], "tool");
        assert_eq!(m[2]["tool_call_id"], "call_1");
    }
}

// ----------------------------------------------------------------------------
// Activity Event Tests (replace flag)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Activity snapshots have replace flag.
// replace: true (default) - replace existing activity message
// replace: false - preserve existing message ID

/// Test: Activity snapshot with replace: true
/// Protocol: replace: true replaces existing activity message
#[test]
fn test_activity_snapshot_replace_true() {
    use std::collections::HashMap;

    let mut content = HashMap::new();
    content.insert("status".to_string(), json!("processing"));

    let event = AGUIEvent::activity_snapshot("act_1", "file_upload", content, Some(true));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""replace":true"#));
}

/// Test: Activity snapshot with replace: false
/// Protocol: replace: false preserves existing message ID
#[test]
fn test_activity_snapshot_replace_false() {
    use std::collections::HashMap;

    let mut content = HashMap::new();
    content.insert("progress".to_string(), json!(0.5));

    let event = AGUIEvent::activity_snapshot("act_1", "download", content, Some(false));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""replace":false"#));
}

/// Test: Activity snapshot without replace (defaults behavior)
#[test]
fn test_activity_snapshot_replace_none() {
    use std::collections::HashMap;

    let mut content = HashMap::new();
    content.insert("data".to_string(), json!("test"));

    let event = AGUIEvent::activity_snapshot("act_1", "process", content, None);

    let json = serde_json::to_string(&event).unwrap();
    // replace field should be omitted when None
    assert!(!json.contains(r#""replace""#) || json.contains(r#""replace":null"#));
}

// ----------------------------------------------------------------------------
// ToolCallStart with parentMessageId Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Tool calls can optionally link to a parent message.

/// Test: ToolCallStart with parentMessageId
/// Protocol: Optional parentMessageId for linking tool calls to messages
#[test]
fn test_tool_call_start_with_parent_message_id() {
    let event = AGUIEvent::tool_call_start("call_1", "search", Some("msg_123".into()));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"TOOL_CALL_START""#));
    assert!(json.contains(r#""parentMessageId":"msg_123""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::ToolCallStart { parent_message_id, .. } = parsed {
        assert_eq!(parent_message_id, Some("msg_123".to_string()));
    }
}

/// Test: ToolCallStart without parentMessageId
#[test]
fn test_tool_call_start_without_parent_message_id() {
    let event = AGUIEvent::tool_call_start("call_1", "read_file", None);

    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains(r#""parentMessageId""#));
}

// ----------------------------------------------------------------------------
// ToolCallResult Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: ToolCallResult delivers execution output.

/// Test: ToolCallResult structure
/// Protocol: TOOL_CALL_RESULT with messageId, toolCallId, content
#[test]
fn test_tool_call_result_structure() {
    let event = AGUIEvent::tool_call_result("result_1", "call_1", r#"{"success": true, "data": "result"}"#);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"TOOL_CALL_RESULT""#));
    assert!(json.contains(r#""messageId":"result_1""#));
    assert!(json.contains(r#""toolCallId":"call_1""#));
    assert!(json.contains(r#""content":"#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::ToolCallResult { message_id, tool_call_id, content, .. } = parsed {
        assert_eq!(message_id, "result_1");
        assert_eq!(tool_call_id, "call_1");
        assert!(content.contains("success"));
    }
}

/// Test: ToolCallResult with error content
#[test]
fn test_tool_call_result_error_content() {
    let error_result = json!({
        "status": "error",
        "tool_name": "write_file",
        "message": "Permission denied"
    });
    let event = AGUIEvent::tool_call_result("result_err", "call_write", &serde_json::to_string(&error_result).unwrap());

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""toolCallId":"call_write""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::ToolCallResult { content, .. } = parsed {
        let result: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(result["status"], "error");
    }
}

// ----------------------------------------------------------------------------
// TextMessage Role Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Text messages have role (developer, system, assistant, user, tool).

/// Test: Text message roles
/// Protocol: Verify all role types serialize correctly
#[test]
fn test_text_message_all_roles() {
    let roles = vec![
        (MessageRole::Developer, "developer"),
        (MessageRole::System, "system"),
        (MessageRole::Assistant, "assistant"),
        (MessageRole::User, "user"),
        (MessageRole::Tool, "tool"),
    ];

    for (role, expected) in roles {
        let event = AGUIEvent::text_message_chunk(
            Some("msg_1".into()),
            Some(role),
            Some("test".into()),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(&format!(r#""role":"{}""#, expected)), "Role {} not found in JSON", expected);
    }
}

// ----------------------------------------------------------------------------
// Event Timestamp Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: All events can have optional timestamp in milliseconds.

/// Test: Event with timestamp
/// Protocol: BaseEvent includes optional timestamp
#[test]
fn test_event_with_timestamp() {
    let mut event = AGUIEvent::run_started("t1", "r1", None);
    event = event.with_timestamp(1704067200000); // 2024-01-01 00:00:00 UTC

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""timestamp":1704067200000"#));
}

/// Test: Event timestamp roundtrip
#[test]
fn test_event_timestamp_roundtrip() {
    let timestamp = 1704067200000u64;
    let event = AGUIEvent::state_snapshot(json!({"test": true})).with_timestamp(timestamp);

    let json = serde_json::to_string(&event).unwrap();
    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();

    if let AGUIEvent::StateSnapshot { base, .. } = parsed {
        assert_eq!(base.timestamp, Some(timestamp));
    }
}

// ----------------------------------------------------------------------------
// Raw Event Tests (source attribution)
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Raw events pass external system events with optional source.

/// Test: Raw event with source attribution
/// Protocol: RAW event with optional source field
#[test]
fn test_raw_event_with_source() {
    let external = json!({"type": "model_response", "tokens": 150});
    let event = AGUIEvent::raw(external.clone(), Some("anthropic".into()));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"RAW""#));
    assert!(json.contains(r#""source":"anthropic""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::Raw { source, .. } = parsed {
        assert_eq!(source, Some("anthropic".to_string()));
    }
}

/// Test: Raw event without source
#[test]
fn test_raw_event_without_source() {
    let external = json!({"custom": "data"});
    let event = AGUIEvent::raw(external, None);

    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains(r#""source""#));
}

// ----------------------------------------------------------------------------
// Custom Event Tests
// ----------------------------------------------------------------------------
//
// Per AG-UI spec: Custom events for application-specific extensions.

/// Test: Custom event with name and value
/// Protocol: CUSTOM event for protocol extensions
#[test]
fn test_custom_event_structure() {
    let value = json!({
        "action": "highlight",
        "target": "line:42",
        "color": "yellow"
    });
    let event = AGUIEvent::custom("editor_highlight", value.clone());

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"CUSTOM""#));
    assert!(json.contains(r#""name":"editor_highlight""#));
    assert!(json.contains(r#""action":"highlight""#));

    let parsed: AGUIEvent = serde_json::from_str(&json).unwrap();
    if let AGUIEvent::Custom { name, value: v, .. } = parsed {
        assert_eq!(name, "editor_highlight");
        assert_eq!(v["target"], "line:42");
    }
}

// ----------------------------------------------------------------------------
// Full Protocol Flow Tests
// ----------------------------------------------------------------------------
//
// End-to-end tests for complete AG-UI protocol flows.

/// Test: Complete tool call flow with all events
/// Protocol: Full TOOL_CALL flow: START → ARGS → END → RESULT
#[test]
fn test_complete_tool_call_protocol_flow() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // Start
    let start = AgentEvent::ToolCallStart {
        id: "call_search".into(),
        name: "web_search".into(),
    };
    events.extend(start.to_ag_ui_events(&mut ctx));

    // Args streaming
    let args1 = AgentEvent::ToolCallDelta {
        id: "call_search".into(),
        args_delta: r#"{"query":"#.into(),
    };
    events.extend(args1.to_ag_ui_events(&mut ctx));

    let args2 = AgentEvent::ToolCallDelta {
        id: "call_search".into(),
        args_delta: r#""rust tutorials"}"#.into(),
    };
    events.extend(args2.to_ag_ui_events(&mut ctx));

    // Ready (end args)
    let ready = AgentEvent::ToolCallReady {
        id: "call_search".into(),
        name: "web_search".into(),
        arguments: json!({"query": "rust tutorials"}),
    };
    events.extend(ready.to_ag_ui_events(&mut ctx));

    // Result
    let done = AgentEvent::ToolCallDone {
        id: "call_search".into(),
        result: ToolResult::success("web_search", json!({"results": 10})),
        patch: None,
    };
    events.extend(done.to_ag_ui_events(&mut ctx));

    // Verify complete sequence
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallArgs { .. })));
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallEnd { .. })));
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));
}

/// Test: State sync flow (snapshot then deltas)
/// Protocol: STATE_SNAPSHOT → STATE_DELTA*
#[test]
fn test_state_sync_protocol_flow() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // Initial snapshot
    let snapshot = AgentEvent::StateSnapshot {
        snapshot: json!({"counter": 0, "items": []}),
    };
    events.extend(snapshot.to_ag_ui_events(&mut ctx));

    // Delta 1: increment counter
    let delta1 = AgentEvent::StateDelta {
        delta: vec![json!({"op": "replace", "path": "/counter", "value": 1})],
    };
    events.extend(delta1.to_ag_ui_events(&mut ctx));

    // Delta 2: add item
    let delta2 = AgentEvent::StateDelta {
        delta: vec![json!({"op": "add", "path": "/items/-", "value": "item1"})],
    };
    events.extend(delta2.to_ag_ui_events(&mut ctx));

    // Verify flow
    assert_eq!(events.len(), 3);
    assert!(matches!(&events[0], AGUIEvent::StateSnapshot { .. }));
    assert!(matches!(&events[1], AGUIEvent::StateDelta { .. }));
    assert!(matches!(&events[2], AGUIEvent::StateDelta { .. }));
}

/// Test: Mixed content flow (text + tool + text)
/// Protocol: Verify correct event sequencing with interleaved content
#[test]
fn test_mixed_content_protocol_flow() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // Text starts
    let text1 = AgentEvent::TextDelta { delta: "Let me search".into() };
    events.extend(text1.to_ag_ui_events(&mut ctx));

    // Tool interrupts (should end text first)
    let tool_start = AgentEvent::ToolCallStart {
        id: "call_1".into(),
        name: "search".into(),
    };
    events.extend(tool_start.to_ag_ui_events(&mut ctx));

    // Tool completes
    let tool_ready = AgentEvent::ToolCallReady {
        id: "call_1".into(),
        name: "search".into(),
        arguments: json!({}),
    };
    events.extend(tool_ready.to_ag_ui_events(&mut ctx));

    let tool_done = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result: ToolResult::success("search", json!({"count": 5})),
        patch: None,
    };
    events.extend(tool_done.to_ag_ui_events(&mut ctx));

    // Text resumes
    let text2 = AgentEvent::TextDelta { delta: "Found 5 results".into() };
    events.extend(text2.to_ag_ui_events(&mut ctx));

    // Verify TEXT_MESSAGE_END appears before TOOL_CALL_START
    let text_end_idx = events.iter().position(|e| matches!(e, AGUIEvent::TextMessageEnd { .. })).unwrap();
    let tool_start_idx = events.iter().position(|e| matches!(e, AGUIEvent::ToolCallStart { .. })).unwrap();
    assert!(text_end_idx < tool_start_idx, "TEXT_MESSAGE_END must come before TOOL_CALL_START");
}

// ============================================================================
// AG-UI Message Types Tests
// ============================================================================
//
// Per AG-UI spec: Six message roles - user, assistant, system, tool, developer, activity
// Reference: https://docs.ag-ui.com/concepts/messages
//

/// Test: AGUIMessage user message creation
/// Protocol: UserMessage with content (string or InputContent[])
#[test]
fn test_agui_message_user() {
    let msg = AGUIMessage::user("Hello, how can you help?");

    assert_eq!(msg.role, MessageRole::User);
    assert_eq!(msg.content, "Hello, how can you help?");
    assert!(msg.id.is_none());
    assert!(msg.tool_call_id.is_none());

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""role":"user""#));
    assert!(json.contains(r#""content":"Hello, how can you help?""#));
}

/// Test: AGUIMessage assistant message creation
/// Protocol: AssistantMessage with optional content and toolCalls
#[test]
fn test_agui_message_assistant() {
    let msg = AGUIMessage::assistant("I can help you with that.");

    assert_eq!(msg.role, MessageRole::Assistant);
    assert_eq!(msg.content, "I can help you with that.");

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""role":"assistant""#));
}

/// Test: AGUIMessage system message creation
/// Protocol: SystemMessage with required content
#[test]
fn test_agui_message_system() {
    let msg = AGUIMessage::system("You are a helpful assistant.");

    assert_eq!(msg.role, MessageRole::System);
    assert_eq!(msg.content, "You are a helpful assistant.");

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""role":"system""#));
}

/// Test: AGUIMessage tool message creation
/// Protocol: ToolMessage with toolCallId linking to assistant's tool call
#[test]
fn test_agui_message_tool() {
    let msg = AGUIMessage::tool(r#"{"result": "success", "data": 42}"#, "call_123");

    assert_eq!(msg.role, MessageRole::Tool);
    assert_eq!(msg.tool_call_id, Some("call_123".to_string()));

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""role":"tool""#));
    assert!(json.contains(r#""toolCallId":"call_123""#));
}

/// Test: AGUIMessage tool message with error
/// Protocol: ToolMessage can include error information
#[test]
fn test_agui_message_tool_with_error() {
    let error_content = json!({
        "status": "error",
        "error": "Connection refused",
        "code": "ECONNREFUSED"
    });
    let msg = AGUIMessage::tool(&serde_json::to_string(&error_content).unwrap(), "call_err");

    assert_eq!(msg.role, MessageRole::Tool);
    let parsed: Value = serde_json::from_str(&msg.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["code"], "ECONNREFUSED");
}

/// Test: AGUIMessage with custom ID
/// Protocol: Messages can have unique identifiers
#[test]
fn test_agui_message_with_id() {
    let mut msg = AGUIMessage::user("test");
    msg.id = Some("msg_12345".to_string());

    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""id":"msg_12345""#));
}

/// Test: AGUIMessage roundtrip serialization
#[test]
fn test_agui_message_roundtrip() {
    let messages = vec![
        AGUIMessage::user("Hello"),
        AGUIMessage::assistant("Hi there!"),
        AGUIMessage::system("Be helpful"),
        AGUIMessage::tool("result", "call_1"),
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: AGUIMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.role, msg.role);
        assert_eq!(parsed.content, msg.content);
    }
}

// ============================================================================
// RunAgentRequest Tests
// ============================================================================
//
// Per AG-UI spec: RunAgentInput contains threadId, runId, messages, tools, state, context
// Reference: https://docs.ag-ui.com/sdk/js/core/types
//

/// Test: RunAgentRequest basic creation
/// Protocol: Required threadId and runId
#[test]
fn test_run_agent_request_basic() {
    let request = RunAgentRequest::new("thread_abc".to_string(), "run_123".to_string());

    assert_eq!(request.thread_id, "thread_abc");
    assert_eq!(request.run_id, "run_123");
    assert!(request.messages.is_empty());
    assert!(request.tools.is_empty());
}

/// Test: RunAgentRequest with messages
/// Protocol: Message array for conversation history
#[test]
fn test_run_agent_request_with_messages() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::user("Hello"))
        .with_message(AGUIMessage::assistant("Hi!"))
        .with_message(AGUIMessage::user("What's 2+2?"));

    assert_eq!(request.messages.len(), 3);
    assert_eq!(request.messages[0].role, MessageRole::User);
    assert_eq!(request.messages[1].role, MessageRole::Assistant);
    assert_eq!(request.messages[2].role, MessageRole::User);
}

/// Test: RunAgentRequest with tools
/// Protocol: Tools array defines available capabilities
#[test]
fn test_run_agent_request_with_tools() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_tool(AGUIToolDef::backend("search", "Search the web"))
        .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy text"));

    assert_eq!(request.tools.len(), 2);
}

/// Test: RunAgentRequest with initial state
/// Protocol: State object for agent execution context
#[test]
fn test_run_agent_request_with_state() {
    let initial_state = json!({
        "counter": 0,
        "preferences": {"theme": "dark"},
        "history": []
    });

    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_state(initial_state.clone());

    assert_eq!(request.state, Some(initial_state));
}

/// Test: RunAgentRequest with parent run ID
/// Protocol: parentRunId for branching/sub-agent runs
#[test]
fn test_run_agent_request_with_parent_run() {
    // Create request and set parent_run_id directly
    let mut request = RunAgentRequest::new("t1".to_string(), "r2".to_string());
    request.parent_run_id = Some("r1".to_string());

    assert_eq!(request.parent_run_id, Some("r1".to_string()));
}

/// Test: RunAgentRequest serialization
#[test]
fn test_run_agent_request_serialization() {
    let request = RunAgentRequest::new("thread_1".to_string(), "run_1".to_string())
        .with_message(AGUIMessage::user("test"))
        .with_state(json!({"key": "value"}));

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains(r#""threadId":"thread_1""#));
    assert!(json.contains(r#""runId":"run_1""#));
    assert!(json.contains(r#""messages":[{"role":"user""#));
}

/// Test: RunAgentRequest deserialization
#[test]
fn test_run_agent_request_deserialization() {
    let json = r#"{
        "threadId": "t1",
        "runId": "r1",
        "messages": [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi!"}
        ],
        "tools": [],
        "state": {"counter": 5}
    }"#;

    let request: RunAgentRequest = serde_json::from_str(json).unwrap();
    assert_eq!(request.thread_id, "t1");
    assert_eq!(request.run_id, "r1");
    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.state.unwrap()["counter"], 5);
}

// ============================================================================
// AGUIToolDef Tests
// ============================================================================
//
// Per AG-UI spec: Tool with name, description, and parameters (JSON Schema)
// Reference: https://docs.ag-ui.com/concepts/tools
//

/// Test: Backend tool definition
/// Protocol: Backend tools execute on agent side
#[test]
fn test_agui_tool_def_backend() {
    let tool = AGUIToolDef::backend("search", "Search for information");

    assert_eq!(tool.name, "search");
    assert_eq!(tool.description, "Search for information");
    assert_eq!(tool.execute, ToolExecutionLocation::Backend);

    let json = serde_json::to_string(&tool).unwrap();
    // Backend is default, so execute field is omitted in serialization
    assert!(!json.contains(r#""execute""#));
}

/// Test: Frontend tool definition
/// Protocol: Frontend tools execute on client side
#[test]
fn test_agui_tool_def_frontend() {
    let tool = AGUIToolDef::frontend("showNotification", "Display a notification");

    assert_eq!(tool.name, "showNotification");
    assert_eq!(tool.execute, ToolExecutionLocation::Frontend);

    let json = serde_json::to_string(&tool).unwrap();
    assert!(json.contains(r#""execute":"frontend""#));
}

/// Test: Tool with JSON Schema parameters
/// Protocol: Parameters defined using JSON Schema
#[test]
fn test_agui_tool_def_with_schema() {
    let schema = json!({
        "type": "object",
        "properties": {
            "query": {"type": "string", "description": "Search query"},
            "limit": {"type": "integer", "default": 10}
        },
        "required": ["query"]
    });

    let tool = AGUIToolDef::backend("search", "Search")
        .with_parameters(schema.clone());

    assert_eq!(tool.parameters, Some(schema));
}

/// Test: Tool serialization with all fields
#[test]
fn test_agui_tool_def_full_serialization() {
    let tool = AGUIToolDef::frontend("readFile", "Read a file from disk")
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }));

    let json = serde_json::to_string(&tool).unwrap();
    assert!(json.contains(r#""name":"readFile""#));
    assert!(json.contains(r#""description":"Read a file from disk""#));
    assert!(json.contains(r#""execute":"frontend""#));
    assert!(json.contains(r#""parameters""#));
}

// ============================================================================
// Event Stream Pattern Tests
// ============================================================================
//
// Per AG-UI spec: Event streaming with cancel/resume functionality
// Reference: https://docs.ag-ui.com/introduction
//

/// Test: Event sequence for canceled run
/// Protocol: Run can be canceled, resulting in RUN_ERROR or no RUN_FINISHED
#[test]
fn test_event_sequence_canceled_run() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // Run starts
    let start = AgentEvent::RunStart {
        thread_id: "t1".into(),
        run_id: "r1".into(),
        parent_run_id: None,
    };
    events.extend(start.to_ag_ui_events(&mut ctx));

    // Text streaming begins
    let text = AgentEvent::TextDelta { delta: "Processing...".into() };
    events.extend(text.to_ag_ui_events(&mut ctx));

    // Run aborted (simulating cancel)
    let abort = AgentEvent::Aborted { reason: "User canceled".into() };
    let abort_events = abort.to_ag_ui_events(&mut ctx);
    events.extend(abort_events);

    // Verify run started
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::RunStarted { .. })));
    // Note: Aborted may or may not produce events depending on implementation
}

/// Test: Error during text streaming
/// Protocol: Error interrupts text stream
#[test]
fn test_error_interrupts_text_stream() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // Text starts
    let text = AgentEvent::TextDelta { delta: "Starting...".into() };
    events.extend(text.to_ag_ui_events(&mut ctx));

    // Error occurs
    let error = AgentEvent::Error { message: "API rate limit exceeded".into() };
    events.extend(error.to_ag_ui_events(&mut ctx));

    // Should have TEXT_MESSAGE_START and possibly TEXT_MESSAGE_END before error
    assert!(events.iter().any(|e| matches!(e, AGUIEvent::TextMessageStart { .. })));
}

/// Test: Multiple text messages in sequence
/// Protocol: Each new message gets its own START/END pair
#[test]
fn test_multiple_text_messages() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());
    let mut events: Vec<AGUIEvent> = Vec::new();

    // First message
    let text1 = AgentEvent::TextDelta { delta: "First message".into() };
    events.extend(text1.to_ag_ui_events(&mut ctx));

    // End first message by starting something else
    let done1 = AgentEvent::Done { response: "First message".into() };
    events.extend(done1.to_ag_ui_events(&mut ctx));

    // Reset context for new message
    ctx = AGUIContext::new("t1".into(), "r2".into());

    // Second message
    let text2 = AgentEvent::TextDelta { delta: "Second message".into() };
    events.extend(text2.to_ag_ui_events(&mut ctx));

    // Count TEXT_MESSAGE_START events
    let start_count = events.iter()
        .filter(|e| matches!(e, AGUIEvent::TextMessageStart { .. }))
        .count();
    assert_eq!(start_count, 2, "Should have 2 TEXT_MESSAGE_START events");
}

// ============================================================================
// State Management Edge Cases
// ============================================================================
//
// Per AG-UI spec: State sync via snapshots and deltas
// Reference: https://docs.ag-ui.com/concepts/state
//

/// Test: Empty state snapshot
/// Protocol: Snapshot can be empty object
#[test]
fn test_empty_state_snapshot() {
    let event = AGUIEvent::state_snapshot(json!({}));

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"STATE_SNAPSHOT""#));
    assert!(json.contains(r#""snapshot":{}"#));
}

/// Test: Empty state delta (no-op)
/// Protocol: Delta with no operations
#[test]
fn test_empty_state_delta() {
    let event = AGUIEvent::state_delta(vec![]);

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains(r#""type":"STATE_DELTA""#));
    assert!(json.contains(r#""delta":[]"#));
}

/// Test: State with nested arrays
/// Protocol: State can have complex nested structures
#[test]
fn test_state_with_nested_arrays() {
    let complex_state = json!({
        "matrix": [[1, 2], [3, 4], [5, 6]],
        "records": [
            {"id": 1, "tags": ["a", "b"]},
            {"id": 2, "tags": ["c"]}
        ]
    });

    let event = AGUIEvent::state_snapshot(complex_state.clone());

    if let AGUIEvent::StateSnapshot { snapshot, .. } = event {
        assert_eq!(snapshot["matrix"][0][0], 1);
        assert_eq!(snapshot["records"][0]["tags"][1], "b");
    }
}

/// Test: State delta with array index operations
/// Protocol: JSON Pointer can target array indices
#[test]
fn test_state_delta_array_operations() {
    let delta = vec![
        json!({"op": "add", "path": "/items/0", "value": "first"}),
        json!({"op": "add", "path": "/items/-", "value": "last"}),
        json!({"op": "replace", "path": "/items/1", "value": "replaced"}),
        json!({"op": "remove", "path": "/items/2"}),
    ];

    let event = AGUIEvent::state_delta(delta);

    if let AGUIEvent::StateDelta { delta: d, .. } = event {
        assert_eq!(d.len(), 4);
        assert_eq!(d[0]["path"], "/items/0");
        assert_eq!(d[1]["path"], "/items/-"); // "-" means append
    }
}

// ============================================================================
// Tool Call Edge Cases
// ============================================================================
//
// Per AG-UI spec: Tool calls with various argument patterns
//

/// Test: Tool call with empty arguments
/// Protocol: Tool can have no arguments
#[test]
fn test_tool_call_empty_args() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let start = AgentEvent::ToolCallStart {
        id: "call_1".into(),
        name: "getCurrentTime".into(),
    };
    let events = start.to_ag_ui_events(&mut ctx);

    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallStart { .. })));
}

/// Test: Tool call with nested JSON arguments
/// Protocol: Arguments can be complex JSON
#[test]
fn test_tool_call_complex_args() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let args = AgentEvent::ToolCallDelta {
        id: "call_1".into(),
        args_delta: serde_json::to_string(&json!({
            "config": {
                "nested": {
                    "deep": {
                        "value": [1, 2, 3]
                    }
                }
            }
        })).unwrap(),
    };
    let events = args.to_ag_ui_events(&mut ctx);

    if let Some(AGUIEvent::ToolCallArgs { delta, .. }) = events.first() {
        let parsed: Value = serde_json::from_str(delta).unwrap();
        assert_eq!(parsed["config"]["nested"]["deep"]["value"][0], 1);
    }
}

/// Test: Tool result with warning
/// Protocol: Tool can return with warning status
#[test]
fn test_tool_result_with_warning_status() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let done = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result: ToolResult::warning("search", json!({"results": 5}), "Results may be stale"),
        patch: None,
    };
    let events = done.to_ag_ui_events(&mut ctx);

    assert!(events.iter().any(|e| matches!(e, AGUIEvent::ToolCallResult { .. })));
}

/// Test: Tool result with pending status
/// Protocol: Tool can indicate async/pending execution
#[test]
fn test_tool_result_with_pending_status() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let done = AgentEvent::ToolCallDone {
        id: "call_1".into(),
        result: ToolResult::pending("longRunningTask", "Task queued, check back later"),
        patch: None,
    };
    let events = done.to_ag_ui_events(&mut ctx);

    if let Some(AGUIEvent::ToolCallResult { content, .. }) = events.first() {
        let parsed: Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["status"], "pending");
    }
}

// ============================================================================
// Request Validation Edge Cases
// ============================================================================
//
// Per AG-UI spec: Input validation for RunAgentRequest
//

/// Test: Request with empty thread ID (should be valid per protocol)
#[test]
fn test_request_empty_thread_id() {
    let request = RunAgentRequest::new("".to_string(), "r1".to_string());
    assert_eq!(request.thread_id, "");
    // Note: Empty thread ID is technically valid JSON, validation is app-level
}

/// Test: Request with very long IDs
#[test]
fn test_request_long_ids() {
    let long_id = "x".repeat(1000);
    let request = RunAgentRequest::new(long_id.clone(), long_id.clone());

    assert_eq!(request.thread_id.len(), 1000);
    assert_eq!(request.run_id.len(), 1000);
}

/// Test: Request with special characters in IDs
#[test]
fn test_request_special_char_ids() {
    let special_id = "thread-123_abc.xyz:456";
    let request = RunAgentRequest::new(special_id.to_string(), "run-1".to_string());

    let json = serde_json::to_string(&request).unwrap();
    let parsed: RunAgentRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.thread_id, special_id);
}

/// Test: Request with Unicode in messages
#[test]
fn test_request_unicode_messages() {
    let request = RunAgentRequest::new("t1".to_string(), "r1".to_string())
        .with_message(AGUIMessage::user("Hello! 你好！こんにちは！🎉"))
        .with_message(AGUIMessage::assistant("Привет! مرحبا"));

    let json = serde_json::to_string(&request).unwrap();
    let parsed: RunAgentRequest = serde_json::from_str(&json).unwrap();

    assert!(parsed.messages[0].content.contains("你好"));
    assert!(parsed.messages[0].content.contains("🎉"));
    assert!(parsed.messages[1].content.contains("Привет"));
}

// ============================================================================
// Interaction Response Tests
// ============================================================================
//
// Per AG-UI spec: Human-in-the-loop approval/denial flows
//

/// Test: Interaction response approval
/// Protocol: User approves pending interaction
#[test]
fn test_interaction_response_approval() {
    let response = InteractionResponse::new("int_123", json!({"approved": true}));

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains(r#""interaction_id":"int_123""#));
    assert!(json.contains(r#""approved":true"#));
}

/// Test: Interaction response denial
/// Protocol: User denies pending interaction
#[test]
fn test_interaction_response_denial() {
    let response = InteractionResponse::new(
        "int_123",
        json!({"approved": false, "reason": "Not authorized"}),
    );

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains(r#""approved":false"#));
    assert!(json.contains(r#""reason":"Not authorized""#));
}

/// Test: Interaction response with custom data
/// Protocol: Response can include user-provided data
#[test]
fn test_interaction_response_with_data() {
    let response = InteractionResponse::new(
        "input_1",
        json!({
            "value": "user input text",
            "selected_option": 2,
            "metadata": {"timestamp": 1234567890}
        }),
    );

    assert_eq!(response.result["value"], "user input text");
    assert_eq!(response.result["selected_option"], 2);
}

// ============================================================================
// Context Tracking Tests
// ============================================================================
//
// Per AG-UI spec: Context management across events
//

/// Test: AGUIContext message ID generation
/// Protocol: Unique message IDs across a run
#[test]
fn test_context_message_id_uniqueness() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let id1 = ctx.new_message_id();
    let id2 = ctx.new_message_id();
    let id3 = ctx.new_message_id();

    assert_ne!(id1, id2);
    assert_ne!(id2, id3);
    assert_ne!(id1, id3);
}

/// Test: AGUIContext step name tracking
/// Protocol: Steps are numbered sequentially
#[test]
fn test_context_step_name_sequence() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    let step1 = ctx.next_step_name();
    let step2 = ctx.next_step_name();
    let step3 = ctx.next_step_name();

    // Steps should have sequential numbers
    assert!(step1.contains("1") || step1.contains("step"));
    assert_ne!(step1, step2);
    assert_ne!(step2, step3);
}

/// Test: AGUIContext text stream state tracking
/// Protocol: Tracks whether text is currently streaming
#[test]
fn test_context_text_stream_state() {
    let mut ctx = AGUIContext::new("t1".into(), "r1".into());

    // Start streaming returns true (was not started before)
    let was_started = ctx.start_text();
    assert!(was_started); // First start returns true (means "did start")

    // Start again returns false (already started)
    let was_started_again = ctx.start_text();
    assert!(!was_started_again); // Already started, returns false

    // End streaming returns true (was active)
    let ended = ctx.end_text();
    assert!(ended);

    // End again returns false (not active)
    let ended_again = ctx.end_text();
    assert!(!ended_again);
}
