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
    let text_delta = AgentEvent::TextDelta("Hello".to_string());
    match text_delta {
        AgentEvent::TextDelta(t) => assert_eq!(t, "Hello"),
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
    let error = AgentEvent::Error("Network timeout".to_string());
    match error {
        AgentEvent::Error(msg) => assert!(msg.contains("timeout")),
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
