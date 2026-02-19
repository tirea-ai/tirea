use super::*;
use crate::contracts::runtime::phase::Phase;
use crate::contracts::runtime::TerminationReason;
use crate::contracts::state::ActivityManager;
use crate::contracts::state::CheckpointReason;
use crate::contracts::storage::VersionPrecondition;
use crate::contracts::tool::{ToolDescriptor, ToolError, ToolResult};
use crate::contracts::ToolCallContext;
use crate::runtime::activity::ActivityHub;
use async_trait::async_trait;
use carve_agent_extension_interaction::InteractionOutbox;
use carve_agent_contract::testing::TestFixture;
use carve_state::{Op, Patch, State};
use genai::chat::{ChatStreamEvent, MessageContent, StreamChunk, StreamEnd, ToolChunk, Usage};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tokio::sync::Notify;


#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
struct TestCounterState {
    counter: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
struct ActivityProgressState {
    progress: f64,
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("echo", "Echo", "Echo the input").with_parameters(json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let msg = args["message"].as_str().unwrap_or("no message");
        Ok(ToolResult::success("echo", json!({ "echoed": msg })))
    }
}

struct CountingEchoTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingEchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "counting_echo",
            "Counting Echo",
            "Echo and increment call counter",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let msg = args["message"].as_str().unwrap_or("no message");
        Ok(ToolResult::success(
            "counting_echo",
            json!({ "echoed": msg }),
        ))
    }
}

struct ScopeSnapshotTool;

#[async_trait]
impl Tool for ScopeSnapshotTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "scope_snapshot",
            "Scope Snapshot",
            "Return tool scope caller context",
        )
    }

    async fn execute(
        &self,
        _args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let rt = ctx.run_config();
        let thread_id = rt
            .value(TOOL_SCOPE_CALLER_THREAD_ID_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let state = rt
            .value(TOOL_SCOPE_CALLER_STATE_KEY)
            .cloned()
            .unwrap_or(Value::Null);
        let messages_len = rt
            .value(TOOL_SCOPE_CALLER_MESSAGES_KEY)
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);

        Ok(ToolResult::success(
            "scope_snapshot",
            json!({
                "thread_id": thread_id,
                "state": state,
                "messages_len": messages_len
            }),
        ))
    }
}

struct ActivityGateTool {
    id: String,
    stream_id: String,
    ready: Arc<Notify>,
    proceed: Arc<Notify>,
}

#[async_trait]
impl Tool for ActivityGateTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(&self.id, "Activity Gate", "Emits activity updates")
    }

    async fn execute(
        &self,
        _args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let activity = ctx.activity(self.stream_id.clone(), "progress");
        let progress = activity.state::<ActivityProgressState>("");
        progress.set_progress(0.1);
        self.ready.notify_one();
        self.proceed.notified().await;
        progress.set_progress(1.0);
        Ok(ToolResult::success(&self.id, json!({ "ok": true })))
    }
}

fn tool_execution_result(call_id: &str, patch: Option<TrackedPatch>) -> ToolExecutionResult {
    ToolExecutionResult {
        execution: crate::engine::tool_execution::ToolExecution {
            call: crate::contracts::state::ToolCall::new(call_id, "test_tool", json!({})),
            result: ToolResult::success("test_tool", json!({"ok": true})),
            patch,
        },
        reminders: Vec::new(),
        pending_interaction: None,
        pending_patches: Vec::new(),
    }
}

fn skill_activation_result(
    call_id: &str,
    skill_id: &str,
    instruction: Option<&str>,
) -> ToolExecutionResult {
    let patch = instruction.map(|text| {
        let fix = TestFixture::new();
        let ctx = fix.ctx_with(call_id, "skill_test");
        let skill_state = ctx.state_of::<carve_agent_extension_skills::SkillState>();
        skill_state.append_user_messages_insert(call_id.to_string(), vec![text.to_string()]);
        ctx.take_patch()
    });
    let result = ToolResult::success("skill", json!({ "activated": true, "skill_id": skill_id }));

    ToolExecutionResult {
        execution: crate::engine::tool_execution::ToolExecution {
            call: crate::contracts::state::ToolCall::new(
                call_id,
                "skill",
                json!({ "skill": skill_id }),
            ),
            result,
            patch,
        },
        reminders: Vec::new(),
        pending_interaction: None,
        pending_patches: Vec::new(),
    }
}

#[test]
fn test_agent_config_default() {
    let config = AgentConfig::default();
    assert_eq!(config.max_rounds, 10);
    assert_eq!(config.tool_executor.name(), "parallel");
    assert!(config.system_prompt.is_empty());
}

#[test]
fn test_agent_config_builder() {
    let config = AgentConfig::new("gpt-4")
        .with_max_rounds(5)
        .with_parallel_tools(false)
        .with_system_prompt("You are helpful.");

    assert_eq!(config.model, "gpt-4");
    assert_eq!(config.max_rounds, 5);
    assert_eq!(config.tool_executor.name(), "sequential");
    assert_eq!(config.system_prompt, "You are helpful.");
}

#[test]
fn test_agent_config_with_fallback_models_and_retry_policy() {
    let policy = LlmRetryPolicy {
        max_attempts_per_model: 3,
        initial_backoff_ms: 100,
        max_backoff_ms: 500,
        retry_stream_start: true,
    };
    let config = AgentConfig::new("primary")
        .with_fallback_models(vec!["fallback-a".to_string()])
        .with_fallback_model("fallback-b")
        .with_llm_retry_policy(policy.clone());

    assert_eq!(config.model, "primary");
    assert_eq!(
        config.fallback_models,
        vec!["fallback-a".to_string(), "fallback-b".to_string()]
    );
    assert_eq!(config.llm_retry_policy.max_attempts_per_model, 3);
    assert_eq!(config.llm_retry_policy.initial_backoff_ms, 100);
    assert_eq!(config.llm_retry_policy.max_backoff_ms, 500);
    assert!(config.llm_retry_policy.retry_stream_start);
}

#[test]
fn test_tool_map() {
    let tools = tool_map([EchoTool]);

    assert!(tools.contains_key("echo"));
    assert_eq!(tools.len(), 1);
}

#[test]
fn test_tool_map_from_arc() {
    let echo: Arc<dyn Tool> = Arc::new(EchoTool);
    let tools = tool_map_from_arc([echo]);

    assert!(tools.contains_key("echo"));
}

#[test]
fn test_agent_loop_error_display() {
    let err = AgentLoopError::LlmError("timeout".to_string());
    assert!(err.to_string().contains("timeout"));

    let err = AgentLoopError::Stopped {
        state: Box::new(Thread::new("test")),
        reason: StopReason::MaxRoundsReached,
    };
    assert!(err.to_string().contains("MaxRoundsReached"));
}

#[test]
fn test_agent_loop_error_termination_reason_mapping() {
    let stopped = AgentLoopError::Stopped {
        state: Box::new(Thread::new("test")),
        reason: StopReason::MaxRoundsReached,
    };
    assert_eq!(
        stopped.termination_reason(),
        TerminationReason::Stopped(StopReason::MaxRoundsReached)
    );

    let cancelled = AgentLoopError::Cancelled {
        state: Box::new(Thread::new("test")),
    };
    assert_eq!(cancelled.termination_reason(), TerminationReason::Cancelled);

    let pending = AgentLoopError::PendingInteraction {
        state: Box::new(Thread::new("test")),
        interaction: Box::new(Interaction::new("int_1", "confirm")),
    };
    assert_eq!(
        pending.termination_reason(),
        TerminationReason::PendingInteraction
    );

    let llm = AgentLoopError::LlmError("offline".to_string());
    assert_eq!(llm.termination_reason(), TerminationReason::Error);

    let state = AgentLoopError::StateError("broken".to_string());
    assert_eq!(state.termination_reason(), TerminationReason::Error);
}

#[test]
fn test_llm_retry_error_classification() {
    assert!(is_retryable_llm_error("429 too many requests"));
    assert!(is_retryable_llm_error("gateway timeout"));
    assert!(!is_retryable_llm_error("401 unauthorized"));
    assert!(!is_retryable_llm_error("400 bad request"));
}

#[test]
fn test_execute_tools_empty() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Hello".to_string(),
            tool_calls: vec![],
            usage: None,
        };
        let tools = HashMap::new();

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();
        assert_eq!(thread.message_count(), 0);
    });
}

#[test]
fn test_execute_tools_with_calls() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "hello"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();

        assert_eq!(thread.message_count(), 1);
        assert_eq!(thread.messages[0].role, crate::contracts::state::Role::Tool);
    });
}

#[test]
fn test_execute_tools_injects_caller_scope_context_for_tools() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::with_initial_state("caller-s", json!({"k":"v"}))
            .with_message(crate::contracts::state::Message::user("hello"));
        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "scope_snapshot",
                json!({}),
            )],
            usage: None,
        };
        let tools = tool_map([ScopeSnapshotTool]);

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();
        assert_eq!(thread.message_count(), 2);
        let tool_msg = thread
            .messages
            .last()
            .expect("tool result message should exist");
        let tool_result: ToolResult =
            serde_json::from_str(&tool_msg.content).expect("tool result json");
        assert_eq!(
            tool_result.status,
            crate::contracts::tool::ToolStatus::Success
        );
        assert_eq!(tool_result.data["thread_id"], json!("caller-s"));
        assert_eq!(tool_result.data["state"]["k"], json!("v"));
        assert_eq!(tool_result.data["messages_len"], json!(1));
    });
}

#[tokio::test]
async fn test_activity_event_emitted_before_tool_completion() {
    use crate::contracts::runtime::AgentEvent;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(tx));

    let ready = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let tool = ActivityGateTool {
        id: "activity_gate".to_string(),
        stream_id: "stream_gate".to_string(),
        ready: ready.clone(),
        proceed: proceed.clone(),
    };

    let call = crate::contracts::state::ToolCall::new("call_1", "activity_gate", json!({}));
    let descriptors = vec![tool.descriptor()];
    let plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
    let state = json!({});

    let mut tool_future = Box::pin(execute_single_tool_with_phases(
        Some(&tool),
        &call,
        &state,
        &descriptors,
        &plugins,
        Some(activity_manager),
        None,
        "test",
        &[],
        0,
        Arc::new(Mutex::new(Vec::new())),
    ));

    tokio::select! {
        _ = ready.notified() => {
            let event = rx.recv().await.expect("activity event");
            match event {
                AgentEvent::ActivitySnapshot { message_id, content, .. } => {
                    assert_eq!(message_id, "stream_gate");
                    assert_eq!(content["progress"], 0.1);
                }
                _ => panic!("Expected ActivitySnapshot"),
            }
            proceed.notify_one();
        }
        _res = &mut tool_future => {
            panic!("Tool finished before activity event");
        }
    }

    let result = tool_future.await.expect("tool execution should succeed");
    assert!(result.execution.result.is_success());
}

#[tokio::test]
async fn test_parallel_tools_emit_activity_before_completion() {
    use crate::contracts::runtime::AgentEvent;
    use std::collections::HashSet;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(tx));

    let ready_a = Arc::new(Notify::new());
    let proceed_a = Arc::new(Notify::new());
    let tool_a = ActivityGateTool {
        id: "activity_gate_a".to_string(),
        stream_id: "stream_gate_a".to_string(),
        ready: ready_a.clone(),
        proceed: proceed_a.clone(),
    };

    let ready_b = Arc::new(Notify::new());
    let proceed_b = Arc::new(Notify::new());
    let tool_b = ActivityGateTool {
        id: "activity_gate_b".to_string(),
        stream_id: "stream_gate_b".to_string(),
        ready: ready_b.clone(),
        proceed: proceed_b.clone(),
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert(tool_a.id.clone(), Arc::new(tool_a));
    tools.insert(tool_b.id.clone(), Arc::new(tool_b));

    let calls = vec![
        crate::contracts::state::ToolCall::new("call_a", "activity_gate_a", json!({})),
        crate::contracts::state::ToolCall::new("call_b", "activity_gate_b", json!({})),
    ];
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
    let state = json!({});

    // Spawn the tool execution so it actually starts running while we await activity events.
    let tools_for_task = tools.clone();
    let calls_for_task = calls.clone();
    let tool_descriptors_for_task = tool_descriptors.clone();
    let plugins_for_task = plugins.clone();
    let state_for_task = state.clone();
    let handle = tokio::spawn(async move {
        execute_tools_parallel_with_phases(
            &tools_for_task,
            &calls_for_task,
            &state_for_task,
            &tool_descriptors_for_task,
            &plugins_for_task,
            Some(activity_manager),
            None,
            "test",
            &[],
            0,
            None,
            Arc::new(Mutex::new(Vec::new())),
        )
        .await
        .expect("parallel tool execution should succeed")
    });

    let ((), ()) = tokio::join!(ready_a.notified(), ready_b.notified());

    // Both tools have emitted their first activity update; observe both snapshots
    // before unblocking them.
    let mut seen: HashSet<String> = HashSet::new();
    while seen.len() < 2 {
        match rx.recv().await.expect("activity event") {
            AgentEvent::ActivitySnapshot {
                message_id,
                content,
                ..
            } => {
                assert_eq!(content["progress"], 0.1);
                seen.insert(message_id);
            }
            other => panic!("Expected ActivitySnapshot, got {:?}", other),
        }
    }
    assert!(seen.contains("stream_gate_a"));
    assert!(seen.contains("stream_gate_b"));

    proceed_a.notify_one();
    proceed_b.notify_one();

    let results = handle.await.expect("task join");
    assert_eq!(results.len(), 2);
    for r in results {
        assert!(r.execution.result.is_success());
    }
}

#[tokio::test]
async fn test_parallel_tool_executor_honors_cancellation_token() {
    let ready = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let tool = ActivityGateTool {
        id: "activity_gate".to_string(),
        stream_id: "parallel_cancel".to_string(),
        ready: ready.clone(),
        proceed,
    };

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    tools.insert("activity_gate".to_string(), Arc::new(tool));
    let calls = vec![crate::contracts::state::ToolCall::new(
        "call_1",
        "activity_gate",
        json!({}),
    )];
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let token = CancellationToken::new();
    let token_for_task = token.clone();
    let ready_for_task = ready.clone();

    let handle = tokio::spawn(async move {
        let result = execute_tools_parallel_with_phases(
            &tools,
            &calls,
            &json!({}),
            &tool_descriptors,
            &[],
            None,
            None,
            "cancel-test",
            &[],
            0,
            Some(&token_for_task),
            Arc::new(Mutex::new(Vec::new())),
        )
        .await;
        ready_for_task.notify_one();
        result
    });

    ready.notified().await;
    token.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_millis(300), handle)
        .await
        .expect("parallel executor should stop shortly after cancellation")
        .expect("task should not panic");
    assert!(
        matches!(result, Err(AgentLoopError::Cancelled { .. })),
        "expected cancellation error from tool executor"
    );
}

struct CounterTool;

#[async_trait]
impl Tool for CounterTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("counter", "Counter", "Increment a counter").with_parameters(json!({
            "type": "object",
            "properties": {
                "amount": { "type": "integer" }
            }
        }))
    }

    async fn execute(&self, args: Value, ctx: &ToolCallContext<'_>) -> Result<ToolResult, ToolError> {
        let amount = args["amount"].as_i64().unwrap_or(1);

        let state = ctx.state::<TestCounterState>("");
        let current = state.counter().unwrap_or(0);
        let new_value = current + amount;

        state.set_counter(new_value);

        Ok(ToolResult::success(
            "counter",
            json!({ "new_value": new_value }),
        ))
    }
}

#[test]
fn test_execute_tools_with_state_changes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::with_initial_state("test", json!({"counter": 0}));
        let result = StreamResult {
            text: "Incrementing".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "counter",
                json!({"amount": 5}),
            )],
            usage: None,
        };
        let tools = tool_map([CounterTool]);

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();

        assert_eq!(thread.message_count(), 1);
        assert_eq!(thread.patch_count(), 1);

        let state = thread.rebuild_state().unwrap();
        assert_eq!(state["counter"], 5);
    });
}

#[test]
fn test_step_result_variants() {
    let thread = Thread::new("test");

    let done = StepResult::Done {
        state: thread.clone(),
        response: "Hello".to_string(),
    };

    let tools_executed = StepResult::ToolsExecuted {
        state: thread,
        text: "Calling tools".to_string(),
        tool_calls: vec![],
    };

    match done {
        StepResult::Done { response, .. } => assert_eq!(response, "Hello"),
        _ => panic!("Expected Done"),
    }

    match tools_executed {
        StepResult::ToolsExecuted { text, .. } => assert_eq!(text, "Calling tools"),
        _ => panic!("Expected ToolsExecuted"),
    }
}

struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("failing", "Failing Tool", "Always fails")
    }

    async fn execute(
        &self,
        _args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed(
            "Intentional failure".to_string(),
        ))
    }
}

#[test]
fn test_execute_tools_with_failing_tool() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Calling failing tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "failing",
                json!({}),
            )],
            usage: None,
        };
        let tools = tool_map([FailingTool]);

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert!(msg.content.contains("error") || msg.content.contains("fail"));
    });
}

// ============================================================================
// Phase-based Plugin Tests
// ============================================================================

struct TestPhasePlugin {
    id: String,
}

impl TestPhasePlugin {
    fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

#[async_trait]
impl AgentPlugin for TestPhasePlugin {
    fn id(&self) -> &str {
        &self.id
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        match phase {
            Phase::StepStart => {
                step.system("Test system context");
            }
            Phase::BeforeInference => {
                step.thread("Test thread context");
            }
            Phase::AfterToolExecute => {
                if step.tool_name() == Some("echo") {
                    step.reminder("Check the echo result");
                }
            }
            _ => {}
        }
    }
}

#[test]
fn test_agent_config_with_phase_plugin() {
    let plugin: Arc<dyn AgentPlugin> = Arc::new(TestPhasePlugin::new("test"));
    let config = AgentConfig::new("gpt-4").with_plugin(plugin);

    assert!(config.has_plugins());
    assert_eq!(config.plugins.len(), 1);
}

struct BlockingPhasePlugin;

#[async_trait]
impl AgentPlugin for BlockingPhasePlugin {
    fn id(&self) -> &str {
        "blocker"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeToolExecute && step.tool_name() == Some("echo") {
            step.block("Echo tool is blocked");
        }
    }
}

#[test]
fn test_execute_tools_with_blocking_phase_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Blocked".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(BlockingPhasePlugin)];

        let thread = execute_tools_with_plugins(thread, &result, &tools, true, &plugins)
            .await
            .unwrap();

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert!(
            msg.content.contains("blocked") || msg.content.contains("Error"),
            "Expected blocked/error in message, got: {}",
            msg.content
        );
    });
}

struct InvalidAfterToolMutationPlugin;

#[async_trait]
impl AgentPlugin for InvalidAfterToolMutationPlugin {
    fn id(&self) -> &str {
        "invalid_after_tool_mutation"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::AfterToolExecute {
            step.block("too late");
        }
    }
}

#[test]
fn test_execute_tools_rejects_tool_gate_mutation_outside_before_tool_execute() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "invalid".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(InvalidAfterToolMutationPlugin)];

        let err = execute_tools_with_plugins(thread, &result, &tools, true, &plugins)
            .await
            .expect_err("phase mutation outside BeforeToolExecute should fail");

        assert!(
            matches!(
                err,
                AgentLoopError::StateError(ref message)
                if message.contains("mutated tool gate outside BeforeToolExecute")
            ),
            "unexpected error: {err:?}"
        );
    });
}

struct ReminderPhasePlugin;

#[async_trait]
impl AgentPlugin for ReminderPhasePlugin {
    fn id(&self) -> &str {
        "reminder"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::AfterToolExecute {
            step.reminder("Tool execution completed");
        }
    }
}

#[test]
fn test_execute_tools_with_reminder_phase_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "With reminder".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ReminderPhasePlugin)];

        let thread = execute_tools_with_plugins(thread, &result, &tools, true, &plugins)
            .await
            .unwrap();

        // Should have tool response + reminder message
        assert_eq!(thread.message_count(), 2);
        assert!(thread.messages[1].content.contains("system-reminder"));
        assert!(thread.messages[1]
            .content
            .contains("Tool execution completed"));
    });
}

#[test]
fn test_build_messages_with_context() {
    let thread = Thread::new("test").with_message(Message::user("Hello"));
    let tool_descriptors = vec![ToolDescriptor::new("test", "Test", "Test tool")];
    let mut fixture = TestFixture::new();
    fixture.messages = thread.messages.clone();
    let mut step = fixture.step(tool_descriptors);

    step.system("System context 1");
    step.system("System context 2");
    step.thread("Thread context");

    let messages = build_messages(&step, "Base system prompt");

    assert_eq!(messages.len(), 3);
    assert!(messages[0].content.contains("Base system prompt"));
    assert!(messages[0].content.contains("System context 1"));
    assert!(messages[0].content.contains("System context 2"));
    assert_eq!(messages[1].content, "Thread context");
    assert_eq!(messages[2].content, "Hello");
}

#[test]
fn test_build_messages_empty_system() {
    let thread = Thread::new("test").with_message(Message::user("Hello"));
    let mut fixture = TestFixture::new();
    fixture.messages = thread.messages.clone();
    let step = fixture.step(vec![]);

    let messages = build_messages(&step, "");

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "Hello");
}

struct ToolFilterPlugin;

#[async_trait]
impl AgentPlugin for ToolFilterPlugin {
    fn id(&self) -> &str {
        "filter"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeInference {
            step.exclude("dangerous_tool");
        }
    }
}

#[test]
fn test_tool_filtering_via_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let tool_descriptors = vec![
            ToolDescriptor::new("safe_tool", "Safe", "Safe tool"),
            ToolDescriptor::new("dangerous_tool", "Dangerous", "Dangerous tool"),
        ];
        let fixture = TestFixture::new();
        let mut step = fixture.step(tool_descriptors);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ToolFilterPlugin)];

        emit_phase_checked(Phase::BeforeInference, &mut step, &plugins)
            .await
            .expect("BeforeInference should not fail");

        assert_eq!(step.tools.len(), 1);
        assert_eq!(step.tools[0].id, "safe_tool");
    });
}

#[tokio::test]
async fn test_plugin_state_channel_available_in_before_tool_execute() {
    struct GuardedPlugin;

    #[async_trait]
    impl AgentPlugin for GuardedPlugin {
        fn id(&self) -> &str {
            "guarded"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::BeforeToolExecute {
                return;
            }

            let state = step.snapshot();
            let allow_exec = state
                .get("plugin")
                .and_then(|p| p.get("allow_exec"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if !allow_exec {
                step.block("missing plugin.allow_exec in state");
            }
        }
    }

    let tool = EchoTool;
    let call =
        crate::contracts::state::ToolCall::new("call_1", "echo", json!({ "message": "hello" }));
    let state = json!({ "plugin": { "allow_exec": true } });
    let tool_descriptors = vec![tool.descriptor()];
    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(GuardedPlugin)];

    let result = execute_single_tool_with_phases(
        Some(&tool),
        &call,
        &state,
        &tool_descriptors,
        &plugins,
        None,
        None,
        "test",
        &[],
        0,
        Arc::new(Mutex::new(Vec::new())),
    )
    .await
    .expect("tool execution should succeed");

    assert!(result.execution.result.is_success());
}

#[tokio::test]
async fn test_plugin_sees_real_session_id_and_scope_in_tool_phase() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static VERIFIED: AtomicBool = AtomicBool::new(false);

    struct SessionCheckPlugin;

    #[async_trait]
    impl AgentPlugin for SessionCheckPlugin {
        fn id(&self) -> &str {
            "session_check"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::BeforeToolExecute {
                assert_eq!(step.thread_id(), "real-thread-42");
                assert_eq!(step.run_config().value("user_id"), Some(&json!("u-abc")),);
                VERIFIED.store(true, Ordering::SeqCst);
            }
        }
    }

    VERIFIED.store(false, Ordering::SeqCst);

    let tool = EchoTool;
    let call = crate::contracts::state::ToolCall::new("call_1", "echo", json!({ "message": "hi" }));
    let state = json!({});
    let tool_descriptors = vec![tool.descriptor()];
    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(SessionCheckPlugin)];

    let mut rt = carve_agent_contract::RunConfig::new();
    rt.set("user_id", "u-abc").unwrap();

    let result = execute_single_tool_with_phases(
        Some(&tool),
        &call,
        &state,
        &tool_descriptors,
        &plugins,
        None,
        Some(&rt),
        "real-thread-42",
        &[],
        0,
        Arc::new(Mutex::new(Vec::new())),
    )
    .await
    .expect("tool execution should succeed");

    assert!(result.execution.result.is_success());
    assert!(VERIFIED.load(Ordering::SeqCst), "plugin did not run");
}

#[tokio::test]
async fn test_plugin_state_patch_visible_in_next_step_before_inference() {
    struct StateChannelPlugin;

    #[async_trait]
    impl AgentPlugin for StateChannelPlugin {
        fn id(&self) -> &str {
            "state_channel"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            match phase {
                Phase::BeforeToolExecute => {
                    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("debug", "seen_tool_execute"),
                        json!(true),
                    )))
                    .with_source("test:state_channel");
                    step.pending_patches.push(patch);
                }
                Phase::BeforeInference => {
                    let state = step.snapshot();
                    let seen_tool_execute = state
                        .get("debug")
                        .and_then(|d| d.get("seen_tool_execute"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if seen_tool_execute {
                        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                            carve_state::path!("debug", "before_inference_observed"),
                            json!(true),
                        )))
                        .with_source("test:state_channel");
                        step.pending_patches.push(patch);
                    }
                }
                _ => {}
            }
        }
    }

    let responses = vec![
        MockResponse::text("run tools").with_tool_call("call_1", "echo", json!({"message": "a"})),
        MockResponse::text("done"),
    ];
    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(StateChannelPlugin) as Arc<dyn AgentPlugin>)
        .with_parallel_tools(true);
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let (_events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(responses),
        config,
        thread,
        tools,
    )
    .await;

    let state = final_thread.rebuild_state().expect("state rebuild");
    assert_eq!(state["debug"]["seen_tool_execute"], true);
    assert_eq!(state["debug"]["before_inference_observed"], true);
}

#[tokio::test]
async fn test_run_phase_block_executes_phases_extracts_output_and_commits_pending_patches() {
    struct PhaseBlockPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    #[async_trait]
    impl AgentPlugin for PhaseBlockPlugin {
        fn id(&self) -> &str {
            "phase_block"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            self.phases.lock().unwrap().push(phase);
            match phase {
                Phase::StepStart => {
                    step.system("from_step_start");
                }
                Phase::BeforeInference => {
                    step.skip_inference = true;
                    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("debug", "phase_block"),
                        json!(true),
                    )))
                    .with_source("test:phase_block");
                    step.pending_patches.push(patch);
                }
                _ => {}
            }
        }
    }

    let thread = Thread::with_initial_state("test", json!({}));
    let tool_descriptors = vec![ToolDescriptor::new("echo", "Echo", "Echo")];
    let phases = Arc::new(Mutex::new(Vec::new()));
    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PhaseBlockPlugin {
        phases: phases.clone(),
    })];
    let (extracted, pending) = run_phase_block(
        &thread,
        &tool_descriptors,
        &plugins,
        &[Phase::StepStart, Phase::BeforeInference],
        |_| {},
        |step| (step.system_context.clone(), step.skip_inference),
    )
    .await
    .expect("phase block should succeed");

    assert_eq!(
        phases.lock().unwrap().as_slice(),
        &[Phase::StepStart, Phase::BeforeInference]
    );
    assert_eq!(extracted.0, vec!["from_step_start".to_string()]);
    assert!(extracted.1);
    assert_eq!(pending.len(), 1);

    let updated = apply_pending_patches(thread, pending);
    let state = updated
        .rebuild_state()
        .expect("state rebuild should succeed");
    assert_eq!(state["debug"]["phase_block"], true);
}

#[tokio::test]
async fn test_emit_cleanup_phases_and_apply_runs_after_inference_and_step_end() {
    struct CleanupPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    #[async_trait]
    impl AgentPlugin for CleanupPlugin {
        fn id(&self) -> &str {
            "cleanup_plugin"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            self.phases.lock().unwrap().push(phase);
            match phase {
                Phase::AfterInference => {
                    let agent = step.state_of::<crate::runtime::control::LoopControlState>();
                    let err = agent
                        .inference_error()
                        .ok()
                        .flatten()
                        .map(|v| json!({"type": v.error_type, "message": v.message}));
                    assert_eq!(
                        err.as_ref()
                            .and_then(|v| v.get("type"))
                            .and_then(|v| v.as_str()),
                        Some("llm_stream_start_error")
                    );
                }
                Phase::StepEnd => {
                    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("debug", "cleanup_ran"),
                        json!(true),
                    )))
                    .with_source("test:cleanup");
                    step.pending_patches.push(patch);
                }
                _ => {}
            }
        }
    }

    let thread = Thread::with_initial_state("test", json!({}));
    let tool_descriptors = vec![ToolDescriptor::new("echo", "Echo", "Echo")];
    let phases = Arc::new(Mutex::new(Vec::new()));
    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(CleanupPlugin {
        phases: phases.clone(),
    })];
    let updated = emit_cleanup_phases_and_apply(
        thread,
        &tool_descriptors,
        &plugins,
        "llm_stream_start_error",
        "boom".to_string(),
    )
    .await
    .expect("cleanup phases should succeed");

    assert_eq!(
        phases.lock().unwrap().as_slice(),
        &[Phase::AfterInference, Phase::StepEnd]
    );
    let state = updated
        .rebuild_state()
        .expect("state rebuild should succeed");
    assert_eq!(state["debug"]["cleanup_ran"], true);
}

#[tokio::test]
async fn test_plugin_can_model_run_scoped_data_via_state_and_cleanup() {
    struct RunScopedStatePlugin;

    #[async_trait]
    impl AgentPlugin for RunScopedStatePlugin {
        fn id(&self) -> &str {
            "run_scoped_state"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            match phase {
                Phase::RunStart => {
                    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("debug", "temp_counter"),
                        json!(1),
                    )))
                    .with_source("test:run_scoped_state");
                    step.pending_patches.push(patch);
                }
                Phase::StepStart => {
                    let state = step.snapshot();
                    let current = state
                        .get("debug")
                        .and_then(|a| a.get("temp_counter"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("debug", "temp_counter"),
                        json!(current + 1),
                    )))
                    .with_source("test:run_scoped_state");
                    step.pending_patches.push(patch);
                }
                Phase::RunEnd => {
                    let state = step.snapshot();
                    let run_count = state
                        .get("debug")
                        .and_then(|d| d.get("run_count"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        + 1;
                    let counter = state
                        .get("debug")
                        .and_then(|a| a.get("temp_counter"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(-1);

                    let patch = Patch::new()
                        .with_op(Op::set(
                            carve_state::path!("debug", "run_count"),
                            json!(run_count),
                        ))
                        .with_op(Op::set(
                            carve_state::path!("debug", "last_temp_counter"),
                            json!(counter),
                        ))
                        .with_op(Op::set(
                            carve_state::path!("debug", "temp_counter"),
                            Value::Null,
                        ));

                    step.pending_patches
                        .push(TrackedPatch::new(patch).with_source("test:run_scoped_state"));
                }
                _ => {}
            }
        }
    }

    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(RunScopedStatePlugin) as Arc<dyn AgentPlugin>);
    let tools = HashMap::new();
    let thread = Thread::with_initial_state("test", json!({}));

    let (_, first_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("done")]),
        config.clone(),
        thread,
        tools.clone(),
    )
    .await;
    let first_state = first_thread.rebuild_state().unwrap();
    assert_eq!(first_state["debug"]["run_count"], 1);
    assert_eq!(first_state["debug"]["last_temp_counter"], 2);
    assert_eq!(first_state["debug"]["temp_counter"], Value::Null);

    let (_, second_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("done")]),
        config,
        first_thread,
        tools,
    )
    .await;
    let second_state = second_thread.rebuild_state().unwrap();
    assert_eq!(second_state["debug"]["run_count"], 2);
    assert_eq!(
        second_state["debug"]["last_temp_counter"], 2,
        "run-local state should be recreated each run and cleaned on RunEnd"
    );
    assert_eq!(second_state["debug"]["temp_counter"], Value::Null);
}

// ============================================================================
// Additional Coverage Tests
// ============================================================================

#[test]
fn test_agent_config_debug() {
    let config = AgentConfig::new("gpt-4").with_system_prompt("You are helpful.");

    let debug_str = format!("{:?}", config);
    assert!(debug_str.contains("AgentConfig"));
    assert!(debug_str.contains("gpt-4"));
    // Check that system_prompt is shown as length indicator
    assert!(debug_str.contains("chars]"));
}

#[test]
fn test_agent_config_with_chat_options() {
    let chat_options = ChatOptions::default();
    let config = AgentConfig::new("gpt-4").with_chat_options(chat_options);
    assert!(config.chat_options.is_some());
}

#[test]
fn test_agent_config_with_plugins() {
    struct DummyPlugin;

    #[async_trait]
    impl AgentPlugin for DummyPlugin {
        fn id(&self) -> &str {
            "dummy"
        }
        async fn on_phase(
            &self,
            _phase: Phase,
            _step: &mut StepContext<'_>,
        ) {
        }
    }

    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(DummyPlugin), Arc::new(DummyPlugin)];
    let config = AgentConfig::new("gpt-4").with_plugins(plugins);
    assert_eq!(config.plugins.len(), 2);
}

struct PendingPhasePlugin;

#[async_trait]
impl AgentPlugin for PendingPhasePlugin {
    fn id(&self) -> &str {
        "pending"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeToolExecute && step.tool_name() == Some("echo") {
            use crate::contracts::runtime::Interaction;
            step.pending(Interaction::new("confirm_1", "confirm").with_message("Execute echo?"));
        }
    }
}

#[test]
fn test_execute_tools_with_pending_phase_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Pending".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PendingPhasePlugin)];

        let err = execute_tools_with_plugins(thread, &result, &tools, true, &plugins)
            .await
            .unwrap_err();

        let (thread, interaction) = match err {
            AgentLoopError::PendingInteraction {
                state: thread,
                interaction,
            } => (thread, interaction),
            other => panic!("Expected PendingInteraction error, got: {:?}", other),
        };

        assert_eq!(interaction.id, "confirm_1");
        assert_eq!(interaction.action, "confirm");

        // Pending tool gets a placeholder tool result to keep message sequence valid.
        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert_eq!(msg.role, crate::contracts::state::Role::Tool);
        assert!(msg.content.contains("awaiting approval"));

        let state = thread.rebuild_state().unwrap();
        assert_eq!(state["loop_control"]["pending_interaction"]["id"], "confirm_1");
    });
}

#[test]
fn test_apply_tool_results_rejects_multiple_pending_interactions() {
    let thread = Thread::new("test");

    let mut first = tool_execution_result("call_1", None);
    first.pending_interaction =
        Some(Interaction::new("confirm_1", "confirm").with_message("approve first tool"));

    let mut second = tool_execution_result("call_2", None);
    second.pending_interaction =
        Some(Interaction::new("confirm_2", "confirm").with_message("approve second tool"));

    let result = apply_tool_results_to_session(thread, &[first, second], None, false);
    assert!(
        matches!(result, Err(AgentLoopError::StateError(_))),
        "expected StateError when multiple pending interactions exist"
    );
}

#[test]
fn test_apply_tool_results_appends_skill_instruction_as_user_message() {
    let thread = Thread::with_initial_state("test", json!({}));
    let result = skill_activation_result("call_1", "docx", Some("## DOCX\nUse docx-js."));

    let applied = apply_tool_results_to_session(thread, &[result], None, false)
        .expect("apply_tool_results_to_session should succeed");

    assert_eq!(applied.thread.message_count(), 2);
    assert_eq!(
        applied.thread.messages[0].role,
        crate::contracts::state::Role::Tool
    );
    assert_eq!(
        applied.thread.messages[1].role,
        crate::contracts::state::Role::User
    );
    assert_eq!(applied.thread.messages[1].content, "## DOCX\nUse docx-js.");
}

#[test]
fn test_apply_tool_results_skill_instruction_user_message_attaches_metadata() {
    let thread = Thread::with_initial_state("test", json!({}));
    let result = skill_activation_result("call_1", "docx", Some("Use docx-js."));
    let meta = MessageMetadata {
        run_id: Some("run-1".to_string()),
        step_index: Some(3),
    };

    let applied = apply_tool_results_to_session(thread, &[result], Some(meta.clone()), false)
        .expect("apply_tool_results_to_session should succeed");

    assert_eq!(applied.thread.message_count(), 2);
    let user_msg = &applied.thread.messages[1];
    assert_eq!(user_msg.role, crate::contracts::state::Role::User);
    assert_eq!(user_msg.metadata.as_ref(), Some(&meta));
}

#[test]
fn test_apply_tool_results_skill_without_instruction_does_not_append_user_message() {
    let thread = Thread::with_initial_state("test", json!({}));
    let result = skill_activation_result("call_1", "docx", None);

    let applied = apply_tool_results_to_session(thread, &[result], None, false)
        .expect("apply_tool_results_to_session should succeed");

    assert_eq!(applied.thread.message_count(), 1);
    assert_eq!(
        applied.thread.messages[0].role,
        crate::contracts::state::Role::Tool
    );
}

#[test]
fn test_apply_tool_results_appends_user_messages_from_agent_state_outbox() {
    let thread = Thread::with_initial_state("test", json!({}));
    let fix = TestFixture::new();
    let ctx = fix.ctx_with("call_1", "test");
    let skill_state = ctx.state_of::<carve_agent_extension_skills::SkillState>();
    skill_state.append_user_messages_insert(
        "call_1".to_string(),
        vec!["first".to_string(), "second".to_string()],
    );
    let outbox_patch = ctx.take_patch();
    let result = ToolExecutionResult {
        execution: crate::engine::tool_execution::ToolExecution {
            call: crate::contracts::state::ToolCall::new("call_1", "any_tool", json!({})),
            result: ToolResult::success("any_tool", json!({"ok": true})),
            patch: Some(outbox_patch),
        },
        reminders: Vec::new(),
        pending_interaction: None,
        pending_patches: Vec::new(),
    };

    let applied = apply_tool_results_to_session(thread, &[result], None, false)
        .expect("apply should succeed");

    assert_eq!(applied.thread.message_count(), 3);
    assert_eq!(
        applied.thread.messages[0].role,
        crate::contracts::state::Role::Tool
    );
    assert_eq!(
        applied.thread.messages[1].role,
        crate::contracts::state::Role::User
    );
    assert_eq!(applied.thread.messages[1].content, "first");
    assert_eq!(
        applied.thread.messages[2].role,
        crate::contracts::state::Role::User
    );
    assert_eq!(applied.thread.messages[2].content, "second");
}

#[test]
fn test_apply_tool_results_ignores_blank_agent_state_outbox_messages() {
    let thread = Thread::with_initial_state("test", json!({}));
    let fix = TestFixture::new();
    let ctx = fix.ctx_with("call_1", "test");
    let skill_state = ctx.state_of::<carve_agent_extension_skills::SkillState>();
    skill_state.append_user_messages_insert(
        "call_1".to_string(),
        vec!["".to_string(), "   ".to_string()],
    );
    let outbox_patch = ctx.take_patch();
    let result = ToolExecutionResult {
        execution: crate::engine::tool_execution::ToolExecution {
            call: crate::contracts::state::ToolCall::new("call_1", "any_tool", json!({})),
            result: ToolResult::success("any_tool", json!({"ok": true})),
            patch: Some(outbox_patch),
        },
        reminders: Vec::new(),
        pending_interaction: None,
        pending_patches: Vec::new(),
    };

    let applied = apply_tool_results_to_session(thread, &[result], None, false)
        .expect("apply should succeed");

    assert_eq!(applied.thread.message_count(), 1);
    assert_eq!(
        applied.thread.messages[0].role,
        crate::contracts::state::Role::Tool
    );
}

#[test]
fn test_apply_tool_results_keeps_tool_and_appended_user_message_order_stable() {
    let thread = Thread::with_initial_state("test", json!({}));
    let first = skill_activation_result("call_2", "beta", Some("Instruction B"));
    let second = skill_activation_result("call_1", "alpha", Some("Instruction A"));

    let applied =
        apply_tool_results_to_session(thread, &[first, second], None, true).expect("apply");
    let messages = &applied.thread.messages;

    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, crate::contracts::state::Role::Tool);
    assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_2"));
    assert_eq!(messages[1].role, crate::contracts::state::Role::Tool);
    assert_eq!(messages[1].tool_call_id.as_deref(), Some("call_1"));
    assert_eq!(messages[2].role, crate::contracts::state::Role::User);
    assert_eq!(messages[2].content, "Instruction B");
    assert_eq!(messages[3].role, crate::contracts::state::Role::User);
    assert_eq!(messages[3].content, "Instruction A");
}

#[test]
fn test_agent_loop_error_state_error() {
    let err = AgentLoopError::StateError("invalid state".to_string());
    assert!(err.to_string().contains("State error"));
    assert!(err.to_string().contains("invalid state"));
}

#[test]
fn test_execute_tools_missing_tool() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Calling unknown tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "unknown_tool",
                json!({}),
            )],
            usage: None,
        };
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new(); // Empty tools

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert!(
            msg.content.contains("not found") || msg.content.contains("Error"),
            "Expected 'not found' error in message, got: {}",
            msg.content
        );
    });
}

#[test]
fn test_execute_tools_with_config_empty_calls() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "No tools".to_string(),
            tool_calls: vec![],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4");

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        // No messages should be added when there are no tool calls
        assert_eq!(thread.message_count(), 0);
    });
}

#[test]
fn test_execute_tools_with_config_basic() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4");

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        assert_eq!(thread.message_count(), 1);
        assert_eq!(thread.messages[0].role, crate::contracts::state::Role::Tool);
    });
}

#[test]
fn test_execute_tools_with_config_enforces_scope_tool_policy_at_execution() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut thread = Thread::new("test");
        thread
            .run_config
            .set(
                crate::engine::tool_filter::SCOPE_ALLOWED_TOOLS_KEY,
                vec!["other"],
            )
            .unwrap();
        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4");

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        let result: ToolResult = serde_json::from_str(&msg.content).expect("tool result");
        assert!(result.is_error());
        assert!(result
            .message
            .unwrap_or_default()
            .contains("not allowed by current policy"));
    });
}

#[test]
fn test_execute_tools_with_config_attaches_scope_run_metadata() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut thread = Thread::new("test").with_message(
            Message::assistant_with_tool_calls(
                "calling tool",
                vec![crate::contracts::state::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
            )
            .with_metadata(crate::contracts::state::MessageMetadata {
                run_id: Some("run-meta-1".to_string()),
                step_index: Some(7),
            }),
        );
        thread.run_config.set("run_id", "run-meta-1").unwrap();

        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4");

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        assert_eq!(thread.message_count(), 2);
        let tool_msg = thread.messages.last().expect("tool message should exist");
        assert_eq!(tool_msg.role, crate::contracts::state::Role::Tool);
        let meta = tool_msg
            .metadata
            .as_ref()
            .expect("tool message metadata should be attached");
        assert_eq!(meta.run_id.as_deref(), Some("run-meta-1"));
        assert_eq!(meta.step_index, Some(7));
    });
}

#[test]
fn test_execute_tools_with_config_with_blocking_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Blocked".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4")
            .with_plugin(Arc::new(BlockingPhasePlugin) as Arc<dyn AgentPlugin>);

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert!(
            msg.content.contains("blocked") || msg.content.contains("Error"),
            "Expected blocked error in message, got: {}",
            msg.content
        );
    });
}

#[test]
fn test_execute_tools_with_config_denied_permission_is_visible_as_tool_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::with_initial_state(
            "test",
            json!({
                "loop_control": {
                    "pending_interaction": {
                        "id": "permission_echo",
                        "action": "tool:AskUserQuestion"
                    }
                }
            }),
        );
        let result = StreamResult {
            text: "Trying tool after denial".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let interaction = carve_agent_extension_interaction::InteractionPlugin::with_responses(
            Vec::new(),
            vec!["permission_echo".to_string()],
        );
        let config =
            AgentConfig::new("gpt-4").with_plugin(Arc::new(interaction) as Arc<dyn AgentPlugin>);

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .expect("denied permission should block tool and return thread");

        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert_eq!(msg.role, crate::contracts::state::Role::Tool);
        assert!(
            msg.content.contains("User denied the action")
                || msg.content.to_lowercase().contains("denied"),
            "Denied permission should be visible in tool error message, got: {}",
            msg.content
        );

        let final_state = thread.rebuild_state().expect("state should rebuild");
        let pending = final_state
            .get("loop_control")
            .and_then(|a| a.get("pending_interaction"));
        assert!(pending.is_none() || pending == Some(&Value::Null));
    });
}

#[test]
fn test_execute_tools_with_config_with_pending_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Pending".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4")
            .with_plugin(Arc::new(PendingPhasePlugin) as Arc<dyn AgentPlugin>);

        let err = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap_err();

        let (thread, interaction) = match err {
            AgentLoopError::PendingInteraction {
                state: thread,
                interaction,
            } => (thread, interaction),
            other => panic!("Expected PendingInteraction error, got: {:?}", other),
        };

        assert_eq!(interaction.id, "confirm_1");
        assert_eq!(interaction.action, "confirm");

        // Pending tool gets a placeholder tool result to keep message sequence valid.
        assert_eq!(thread.message_count(), 1);
        let msg = &thread.messages[0];
        assert_eq!(msg.role, crate::contracts::state::Role::Tool);
        assert!(msg.content.contains("awaiting approval"));

        // Pending interaction should be persisted via Thread.
        let state = thread.rebuild_state().unwrap();
        assert_eq!(state["loop_control"]["pending_interaction"]["id"], "confirm_1");
    });
}

#[test]
fn test_execute_tools_with_config_with_reminder_plugin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "With reminder".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4")
            .with_plugin(Arc::new(ReminderPhasePlugin) as Arc<dyn AgentPlugin>);

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        // Should have tool response + reminder message
        assert_eq!(thread.message_count(), 2);
        assert!(thread.messages[1].content.contains("system-reminder"));
    });
}

#[test]
fn test_execute_tools_with_config_clears_persisted_pending_interaction_on_success() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Seed a session with a previously persisted pending interaction.
        let base_state = json!({});
        let pending_patch = set_agent_pending_interaction(
            &base_state,
            Interaction::new("confirm_1", "confirm").with_message("ok"),
        );
        let thread = Thread::with_initial_state("test", base_state).with_patch(pending_patch);

        let result = StreamResult {
            text: "Calling tool".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let config = AgentConfig::new("gpt-4");

        let thread = execute_tools_with_config(thread, &result, &tools, &config)
            .await
            .unwrap();

        let state = thread.rebuild_state().unwrap();
        let pending = state
            .get("loop_control")
            .and_then(|a| a.get("pending_interaction"));
        assert!(
            pending.is_none() || pending.is_some_and(|v| v.is_null()),
            "expected pending_interaction to be cleared, got: {pending:?}"
        );
    });
}

#[test]
fn test_execute_tools_sequential_propagates_intermediate_state_apply_errors() {
    struct FirstCallIntermediatePatchPlugin;

    #[async_trait]
    impl AgentPlugin for FirstCallIntermediatePatchPlugin {
        fn id(&self) -> &str {
            "first_call_intermediate_patch"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::AfterToolExecute || step.tool_call_id() != Some("call_1") {
                return;
            }

            // This increment fails when applied between call_1 and call_2 because
            // `counter` doesn't exist yet. Swallowing that failure hides a broken
            // intermediate state transition.
            let patch = TrackedPatch::new(
                Patch::new().with_op(Op::increment(carve_state::path!("counter"), 1_i64)),
            )
            .with_source("test:intermediate_apply_error");
            step.pending_patches.push(patch);
        }
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Call tools".to_string(),
            tool_calls: vec![
                crate::contracts::state::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "hello"}),
                ),
                crate::contracts::state::ToolCall::new("call_2", "counter", json!({"amount": 5})),
            ],
            usage: None,
        };

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));
        tools.insert("counter".to_string(), Arc::new(CounterTool));
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(FirstCallIntermediatePatchPlugin)];

        let err = execute_tools_with_plugins(thread, &result, &tools, false, &plugins)
            .await
            .expect_err("sequential apply errors should surface");
        assert!(matches!(err, AgentLoopError::StateError(_)));
    });
}

// ========================================================================
// Phase lifecycle helpers & tests for run_loop_stream
// ========================================================================

/// Plugin that records phases AND skips inference.
struct RecordAndSkipPlugin {
    phases: Arc<Mutex<Vec<Phase>>>,
}

impl RecordAndSkipPlugin {
    fn new() -> (Self, Arc<Mutex<Vec<Phase>>>) {
        let phases = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                phases: phases.clone(),
            },
            phases,
        )
    }
}

#[async_trait]
impl AgentPlugin for RecordAndSkipPlugin {
    fn id(&self) -> &str {
        "record_and_skip"
    }
    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        self.phases.lock().unwrap().push(phase);
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

/// Collect all events from a stream.
async fn collect_stream_events(
    stream: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
) -> Vec<AgentEvent> {
    use futures::StreamExt;
    let mut events = Vec::new();
    let mut stream = stream;
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn test_stream_skip_inference_emits_run_end_phase() {
    let (recorder, phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();

    let stream = run_loop_stream(
        Client::default(),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    let events = collect_stream_events(stream).await;

    // Verify events include RunStart and RunFinish
    assert!(
        matches!(events.first(), Some(AgentEvent::RunStart { .. })),
        "Expected RunStart as first event, got: {:?}",
        events.first()
    );
    assert!(
        matches!(events.last(), Some(AgentEvent::RunFinish { .. })),
        "Expected RunFinish as last event, got: {:?}",
        events.last()
    );

    // Verify phase lifecycle: RunStart → StepStart → BeforeInference → RunEnd
    let recorded = phases.lock().unwrap().clone();
    assert!(
        recorded.contains(&Phase::RunStart),
        "Missing RunStart phase"
    );
    assert!(recorded.contains(&Phase::RunEnd), "Missing RunEnd phase");

    // RunEnd must be last
    assert_eq!(
        recorded.last(),
        Some(&Phase::RunEnd),
        "RunEnd should be last phase, got: {:?}",
        recorded
    );
    let run_end_count = recorded.iter().filter(|p| **p == Phase::RunEnd).count();
    assert_eq!(run_end_count, 1, "RunEnd should be emitted exactly once");
}

#[tokio::test]
async fn test_stream_skip_inference_emits_run_start_and_finish() {
    // Verify the complete event sequence on skip_inference path
    let (recorder, _phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();

    let stream = run_loop_stream(
        Client::default(),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    let events = collect_stream_events(stream).await;

    let event_names: Vec<&str> = events
        .iter()
        .map(|e| match e {
            AgentEvent::RunStart { .. } => "RunStart",
            AgentEvent::Pending { .. } => "Pending",
            AgentEvent::RunFinish { .. } => "RunFinish",
            AgentEvent::Error { .. } => "Error",
            _ => "Other",
        })
        .collect();
    assert_eq!(event_names, vec!["RunStart", "RunFinish"]);
}

#[tokio::test]
async fn test_stream_skip_inference_with_pending_state_emits_pending_and_pauses() {
    struct PendingSkipPlugin;

    #[async_trait]
    impl AgentPlugin for PendingSkipPlugin {
        fn id(&self) -> &str {
            "pending_skip"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::BeforeInference {
                return;
            }
            let state = step.snapshot();
            let patch = set_agent_pending_interaction(
                &state,
                Interaction::new("agent_recovery_run-1", "recover_agent_run")
                    .with_message("resume?"),
            );
            step.pending_patches.push(patch);
            step.skip_inference = true;
        }
    }

    let config = AgentConfig::new("gpt-4o-mini")
        .with_plugin(Arc::new(PendingSkipPlugin) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();

    let events = collect_stream_events(run_loop_stream(
        Client::default(),
        config,
        thread,
        tools,
        RunContext::default(),
    ))
    .await;

    assert!(matches!(events.get(0), Some(AgentEvent::RunStart { .. })));
    assert!(matches!(
        events.get(1),
        Some(AgentEvent::InteractionRequested { interaction })
            if interaction.action == "recover_agent_run"
    ));
    assert!(matches!(
        events.get(2),
        Some(AgentEvent::Pending { interaction })
            if interaction.action == "recover_agent_run"
    ));
    assert!(matches!(
        events.get(3),
        Some(AgentEvent::RunFinish {
            termination: TerminationReason::PendingInteraction,
            ..
        })
    ));
    assert_eq!(events.len(), 4, "unexpected extra events: {events:?}");
}

#[tokio::test]
async fn test_stream_emits_interaction_resolved_on_denied_response() {
    struct SkipInferencePlugin;

    #[async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let interaction = carve_agent_extension_interaction::InteractionPlugin::with_responses(
        Vec::new(),
        vec!["permission_write_file".to_string()],
    );
    let config = AgentConfig::new("gpt-4o-mini")
        .with_plugin(Arc::new(interaction))
        .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::with_initial_state(
        "test",
        serde_json::json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "confirm",
                    "parameters": { "tool_id": "write_file" }
                }
            }
        }),
    )
    .with_message(crate::contracts::state::Message::user("continue"));
    let tools = HashMap::new();

    let events = collect_stream_events(run_loop_stream(
        Client::default(),
        config,
        thread,
        tools,
        RunContext::default(),
    ))
    .await;

    assert!(matches!(events.first(), Some(AgentEvent::RunStart { .. })));
    assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::InteractionResolved {
                    interaction_id,
                    result
                } if interaction_id == "permission_write_file" && result == &serde_json::Value::Bool(false)
            )),
            "missing denied InteractionResolved event: {events:?}"
        );
}

#[tokio::test]
async fn test_stream_permission_approval_replays_tool_and_appends_tool_result() {
    struct SkipInferencePlugin;

    #[async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference_for_permission_approval"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let interaction = carve_agent_extension_interaction::InteractionPlugin::with_responses(
        vec!["permission_echo".to_string()],
        Vec::new(),
    );
    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(interaction))
        .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::with_initial_state(
        "test",
        json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "permission_echo",
                    "action": "tool:AskUserQuestion",
                    "parameters": {
                        "origin_tool_call": {
                            "id": "call_1",
                            "name": "echo",
                            "arguments": { "message": "approved-run" }
                        }
                    }
                }
            }
        }),
    )
    .with_message(Message::assistant_with_tool_calls(
        "need permission",
        vec![crate::contracts::state::ToolCall::new(
            "call_1",
            "echo",
            json!({"message": "approved-run"}),
        )],
    ))
    .with_message(Message::tool(
        "call_1",
        "Tool 'echo' is awaiting approval. Execution paused.",
    ));

    let tools = tool_map([EchoTool]);
    let (events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        config,
        thread,
        tools,
    )
    .await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::InteractionResolved {
                interaction_id,
                result
            } if interaction_id == "permission_echo" && result == &serde_json::Value::Bool(true)
        )),
        "missing approval InteractionResolved event: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCallDone { id, result, .. }
                if id == "call_1" && result.status == crate::contracts::tool::ToolStatus::Success
        )),
        "approved flow must replay and execute original tool call: {events:?}"
    );

    let tool_msgs: Vec<&Arc<Message>> = final_thread
        .messages
        .iter()
        .filter(|m| {
            m.role == crate::contracts::state::Role::Tool
                && m.tool_call_id.as_deref() == Some("call_1")
        })
        .collect();
    assert!(!tool_msgs.is_empty(), "expected tool messages for call_1");
    let placeholder_index = tool_msgs
        .iter()
        .position(|m| m.content.contains("awaiting approval"))
        .expect("placeholder must remain immutable in append-only stream");
    let replay_index = tool_msgs
        .iter()
        .position(|m| m.content.contains("\"echoed\":\"approved-run\""))
        .expect("missing replayed tool result content");
    assert!(
        replay_index > placeholder_index,
        "replayed tool output must be appended after placeholder"
    );

    let final_state = final_thread.rebuild_state().expect("state should rebuild");
    let pending = final_state
        .get("loop_control")
        .and_then(|a| a.get("pending_interaction"));
    assert!(pending.is_none() || pending == Some(&Value::Null));
}

#[tokio::test]
async fn test_stream_permission_denied_does_not_replay_tool_call() {
    struct SkipInferencePlugin;

    #[async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference_for_permission_denial"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    let interaction = carve_agent_extension_interaction::InteractionPlugin::with_responses(
        Vec::new(),
        vec!["permission_echo".to_string()],
    );
    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(interaction))
        .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::with_initial_state(
        "test",
        json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "permission_echo",
                    "action": "tool:AskUserQuestion",
                    "parameters": {
                        "origin_tool_call": {
                            "id": "call_1",
                            "name": "echo",
                            "arguments": { "message": "denied-run" }
                        }
                    }
                }
            }
        }),
    )
    .with_message(Message::assistant_with_tool_calls(
        "need permission",
        vec![crate::contracts::state::ToolCall::new(
            "call_1",
            "echo",
            json!({"message": "denied-run"}),
        )],
    ))
    .with_message(Message::tool(
        "call_1",
        "Tool 'echo' is awaiting approval. Execution paused.",
    ));

    let tools = tool_map([EchoTool]);
    let (events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        config,
        thread,
        tools,
    )
    .await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::InteractionResolved {
                interaction_id,
                result
            } if interaction_id == "permission_echo" && result == &serde_json::Value::Bool(false)
        )),
        "missing denied InteractionResolved event: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDone { id, .. } if id == "call_1")),
        "denied flow must not replay or execute original tool call: {events:?}"
    );

    let tool_msg = final_thread
        .messages
        .iter()
        .find(|m| {
            m.role == crate::contracts::state::Role::Tool
                && m.tool_call_id.as_deref() == Some("call_1")
        })
        .expect("placeholder tool message should remain when denied");
    assert!(
        tool_msg.content.contains("awaiting approval"),
        "denied flow should not replace placeholder with successful tool output"
    );
    assert!(
        !tool_msg.content.contains("User denied the action"),
        "denied session-start flow currently does not synthesize tool-denied message"
    );

    let final_state = final_thread.rebuild_state().expect("state should rebuild");
    let pending = final_state
        .get("loop_control")
        .and_then(|a| a.get("pending_interaction"));
    assert!(pending.is_none() || pending == Some(&Value::Null));
}

#[tokio::test]
async fn test_run_loop_skip_inference_emits_run_end_phase() {
    let (recorder, phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();

    let result = run_loop(&client, &config, thread, &tools).await;
    assert!(result.is_ok());

    let recorded = phases.lock().unwrap().clone();
    assert!(
        recorded.contains(&Phase::RunStart),
        "Missing RunStart phase"
    );
    assert!(recorded.contains(&Phase::RunEnd), "Missing RunEnd phase");
    assert_eq!(
        recorded.last(),
        Some(&Phase::RunEnd),
        "RunEnd should be last phase, got: {:?}",
        recorded
    );
    let run_end_count = recorded.iter().filter(|p| **p == Phase::RunEnd).count();
    assert_eq!(run_end_count, 1, "RunEnd should be emitted exactly once");
}

#[tokio::test]
async fn test_run_loop_skip_inference_with_pending_state_returns_pending_interaction() {
    struct PendingSkipPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    #[async_trait]
    impl AgentPlugin for PendingSkipPlugin {
        fn id(&self) -> &str {
            "pending_skip_non_stream"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            self.phases.lock().unwrap().push(phase);
            if phase != Phase::BeforeInference {
                return;
            }
            let state = step.snapshot();
            let patch = set_agent_pending_interaction(
                &state,
                Interaction::new("agent_recovery_run-1", "recover_agent_run")
                    .with_message("resume?"),
            );
            step.pending_patches.push(patch);
            step.skip_inference = true;
        }
    }

    let phases = Arc::new(Mutex::new(Vec::new()));
    let config = AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(PendingSkipPlugin {
        phases: phases.clone(),
    }) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();

    let result = run_loop(&client, &config, thread, &tools).await;
    let (thread, interaction) = match result {
        Err(AgentLoopError::PendingInteraction {
            state: thread,
            interaction,
        }) => (thread, interaction),
        other => panic!("expected PendingInteraction on skip_inference, got: {other:?}"),
    };
    assert_eq!(interaction.action, "recover_agent_run");
    assert_eq!(interaction.message, "resume?");

    let state = thread.rebuild_state().expect("state should rebuild");
    assert_eq!(
        state["loop_control"]["pending_interaction"]["action"],
        Value::String("recover_agent_run".to_string())
    );

    let recorded = phases.lock().unwrap().clone();
    assert_eq!(
        recorded.last(),
        Some(&Phase::RunEnd),
        "RunEnd should be last phase, got: {:?}",
        recorded
    );
    let run_end_count = recorded.iter().filter(|p| **p == Phase::RunEnd).count();
    assert_eq!(run_end_count, 1, "RunEnd should be emitted exactly once");
}

#[tokio::test]
async fn test_run_loop_auto_generated_run_id_is_rfc4122_uuid_v7() {
    let (recorder, _phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();

    let (final_thread, _response) = run_loop(&client, &config, thread, &tools)
        .await
        .expect("run_loop should succeed");
    let run_id = final_thread
        .run_config
        .value("run_id")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("run_loop must populate scope run_id"));

    let parsed = uuid::Uuid::parse_str(run_id)
        .unwrap_or_else(|_| panic!("run_id must be parseable UUID, got: {run_id}"));
    assert_eq!(
        parsed.get_variant(),
        uuid::Variant::RFC4122,
        "run_id must be RFC4122 UUID, got: {run_id}"
    );
    assert_eq!(
        parsed.get_version_num(),
        7,
        "run_id must be version 7 UUID, got: {run_id}"
    );
}

#[tokio::test]
async fn test_run_loop_phase_sequence_on_skip_inference() {
    // Verify the full phase sequence: RunStart → StepStart → BeforeInference → RunEnd
    let (recorder, phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();

    let result = run_loop(&client, &config, thread, &tools).await;
    assert!(result.is_ok());

    let recorded = phases.lock().unwrap().clone();
    assert_eq!(
        recorded,
        vec![
            Phase::RunStart,
            Phase::StepStart,
            Phase::BeforeInference,
            Phase::RunEnd,
        ],
        "Unexpected phase sequence: {:?}",
        recorded
    );
}

#[tokio::test]
async fn test_run_loop_rejects_skip_inference_mutation_outside_before_inference() {
    struct InvalidStepStartSkipPlugin;

    #[async_trait]
    impl AgentPlugin for InvalidStepStartSkipPlugin {
        fn id(&self) -> &str {
            "invalid_step_start_skip"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::StepStart {
                step.skip_inference = true;
            }
        }
    }

    let config = AgentConfig::new("gpt-4o-mini")
        .with_plugin(Arc::new(InvalidStepStartSkipPlugin) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();

    let result = run_loop(&client, &config, thread, &tools).await;
    assert!(
        matches!(
            result,
            Err(AgentLoopError::StateError(ref message))
            if message.contains("mutated skip_inference outside BeforeInference")
        ),
        "expected phase mutation state error, got: {result:?}"
    );
}

#[tokio::test]
async fn test_stream_rejects_skip_inference_mutation_outside_before_inference() {
    struct InvalidStepStartSkipPlugin;

    #[async_trait]
    impl AgentPlugin for InvalidStepStartSkipPlugin {
        fn id(&self) -> &str {
            "invalid_step_start_skip"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::StepStart {
                step.skip_inference = true;
            }
        }
    }

    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(InvalidStepStartSkipPlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::new("test").with_message(Message::user("hi"));
    let tools = HashMap::new();

    let events = run_mock_stream(MockStreamProvider::new(vec![]), config, thread, tools).await;

    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::Error { message }
            if message.contains("mutated skip_inference outside BeforeInference")
        )),
        "expected mutation error event, got: {events:?}"
    );
    assert!(
        matches!(events.last(), Some(AgentEvent::RunFinish { .. })),
        "expected stream termination after mutation error, got: {events:?}"
    );
}

#[tokio::test]
async fn test_stream_run_finish_has_matching_thread_id() {
    let (recorder, _phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

    let thread =
        Thread::new("my-thread").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();

    let stream = run_loop_stream(
        Client::default(),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    let events = collect_stream_events(stream).await;

    // Extract thread_id from RunStart and RunFinish
    let start_tid = events.iter().find_map(|e| match e {
        AgentEvent::RunStart { thread_id, .. } => Some(thread_id.clone()),
        _ => None,
    });
    let finish_tid = events.iter().find_map(|e| match e {
        AgentEvent::RunFinish { thread_id, .. } => Some(thread_id.clone()),
        _ => None,
    });

    assert_eq!(
        start_tid, finish_tid,
        "RunStart and RunFinish thread_ids must match"
    );
    assert_eq!(start_tid.as_deref(), Some("my-thread"));
}

// ========================================================================
// RunContext tests
// ========================================================================

#[test]
fn test_run_context_default() {
    let ctx = RunContext::default();
    assert!(ctx.cancellation_token.is_none());
    assert!(ctx.state_committer.is_none());
}

#[test]
fn test_run_context_with_cancellation() {
    let ctx = RunContext {
        cancellation_token: Some(CancellationToken::new()),
        ..RunContext::default()
    };
    assert!(ctx.cancellation_token.is_some());
}

#[test]
fn test_run_context_run_cancellation_token_accessor() {
    let token = RunCancellationToken::new();
    let ctx = RunContext {
        cancellation_token: Some(token),
        ..RunContext::default()
    };
    assert!(ctx.run_cancellation_token().is_some());
}

#[test]
fn test_run_context_clone() {
    let ctx = RunContext {
        cancellation_token: None,
        ..RunContext::default()
    };
    let cloned = ctx.clone();
    assert!(cloned.cancellation_token.is_none());
}

#[test]
fn test_scope_run_id_in_session() {
    let mut thread = Thread::new("test");
    thread.run_config.set("run_id", "my-run").unwrap();
    thread.run_config.set("parent_run_id", "parent-run").unwrap();
    assert_eq!(
        thread.run_config.value("run_id").and_then(|v| v.as_str()),
        Some("my-run")
    );
    assert_eq!(
        thread.run_config.value("parent_run_id").and_then(|v| v.as_str()),
        Some("parent-run")
    );
}

// ========================================================================
// Mock ChatProvider for non-stream stop condition/retry tests
// ========================================================================

struct MockChatProvider {
    responses: Mutex<Vec<genai::Result<genai::chat::ChatResponse>>>,
    models_seen: Mutex<Vec<String>>,
}

impl MockChatProvider {
    fn new(responses: Vec<genai::Result<genai::chat::ChatResponse>>) -> Self {
        Self {
            responses: Mutex::new(responses),
            models_seen: Mutex::new(Vec::new()),
        }
    }

    fn seen_models(&self) -> Vec<String> {
        self.models_seen.lock().expect("lock poisoned").clone()
    }
}

fn text_chat_response(text: &str) -> genai::chat::ChatResponse {
    let model_iden = genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "mock");
    genai::chat::ChatResponse {
        content: MessageContent::from_text(text.to_string()),
        reasoning_content: None,
        model_iden: model_iden.clone(),
        provider_model_iden: model_iden,
        usage: Usage::default(),
        captured_raw_body: None,
    }
}

fn text_chat_response_with_usage(
    text: &str,
    prompt_tokens: i32,
    completion_tokens: i32,
) -> genai::chat::ChatResponse {
    let model_iden = genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "mock");
    genai::chat::ChatResponse {
        content: MessageContent::from_text(text.to_string()),
        reasoning_content: None,
        model_iden: model_iden.clone(),
        provider_model_iden: model_iden,
        usage: Usage {
            prompt_tokens: Some(prompt_tokens),
            prompt_tokens_details: None,
            completion_tokens: Some(completion_tokens),
            completion_tokens_details: None,
            total_tokens: Some(prompt_tokens + completion_tokens),
        },
        captured_raw_body: None,
    }
}

fn tool_call_chat_response(call_id: &str, name: &str, args: Value) -> genai::chat::ChatResponse {
    let model_iden = genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "mock");
    genai::chat::ChatResponse {
        content: MessageContent::from_tool_calls(vec![genai::chat::ToolCall {
            call_id: call_id.to_string(),
            fn_name: name.to_string(),
            fn_arguments: Value::String(args.to_string()),
            thought_signatures: None,
        }]),
        reasoning_content: None,
        model_iden: model_iden.clone(),
        provider_model_iden: model_iden,
        usage: Usage::default(),
        captured_raw_body: None,
    }
}

fn tool_call_chat_response_object_args(
    call_id: &str,
    name: &str,
    args: Value,
) -> genai::chat::ChatResponse {
    let model_iden = genai::ModelIden::new(genai::adapter::AdapterKind::OpenAI, "mock");
    genai::chat::ChatResponse {
        content: MessageContent::from_tool_calls(vec![genai::chat::ToolCall {
            call_id: call_id.to_string(),
            fn_name: name.to_string(),
            fn_arguments: args,
            thought_signatures: None,
        }]),
        reasoning_content: None,
        model_iden: model_iden.clone(),
        provider_model_iden: model_iden,
        usage: Usage::default(),
        captured_raw_body: None,
    }
}

#[async_trait]
impl ChatProvider for MockChatProvider {
    async fn exec_chat_response(
        &self,
        model: &str,
        _chat_req: genai::chat::ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse> {
        self.models_seen
            .lock()
            .expect("lock poisoned")
            .push(model.to_string());
        let mut responses = self.responses.lock().expect("lock poisoned");
        if responses.is_empty() {
            Ok(text_chat_response("done"))
        } else {
            responses.remove(0)
        }
    }
}

struct HangingChatProvider {
    ready: Arc<Notify>,
    proceed: Arc<Notify>,
    response: genai::chat::ChatResponse,
}

#[async_trait]
impl ChatProvider for HangingChatProvider {
    async fn exec_chat_response(
        &self,
        _model: &str,
        _chat_req: genai::chat::ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse> {
        self.ready.notify_one();
        self.proceed.notified().await;
        Ok(self.response.clone())
    }
}

#[tokio::test]
async fn test_nonstream_uses_fallback_model_after_primary_failures() {
    let provider = MockChatProvider::new(vec![
        Err(genai::Error::Internal("429 rate limit".to_string())),
        Err(genai::Error::Internal("429 rate limit".to_string())),
        Ok(text_chat_response("ok")),
    ]);
    let config = AgentConfig::new("primary")
        .with_fallback_model("fallback")
        .with_llm_retry_policy(LlmRetryPolicy {
            max_attempts_per_model: 2,
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
            retry_stream_start: true,
        });
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let (final_thread, last_text) =
        run_loop_with_context_provider(&provider, &config, thread, &tools, RunContext::default())
            .await
            .expect("non-stream run should succeed with fallback model");

    assert_eq!(last_text, "ok");
    assert_eq!(
        provider.seen_models(),
        vec![
            "primary".to_string(),
            "primary".to_string(),
            "fallback".to_string()
        ]
    );
    assert!(
        final_thread
            .messages
            .iter()
            .any(|m| m.role == crate::contracts::state::Role::Assistant && m.content == "ok"),
        "assistant response should be stored in thread"
    );
}

#[tokio::test]
async fn test_nonstream_llm_error_runs_cleanup_and_run_end_phases() {
    struct CleanupOnLlmErrorPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    #[async_trait]
    impl AgentPlugin for CleanupOnLlmErrorPlugin {
        fn id(&self) -> &str {
            "cleanup_on_llm_error_nonstream"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            self.phases.lock().expect("lock poisoned").push(phase);
            if phase != Phase::AfterInference {
                return;
            }

            let agent = step.state_of::<crate::runtime::control::LoopControlState>();
            let err_type = agent.inference_error().ok().flatten().map(|e| e.error_type);
            assert_eq!(err_type.as_deref(), Some("llm_exec_error"));
        }
    }

    let phases = Arc::new(Mutex::new(Vec::new()));
    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(CleanupOnLlmErrorPlugin {
            phases: phases.clone(),
        }) as Arc<dyn AgentPlugin>)
        .with_llm_retry_policy(LlmRetryPolicy {
            max_attempts_per_model: 1,
            initial_backoff_ms: 1,
            max_backoff_ms: 1,
            retry_stream_start: true,
        });
    let provider = MockChatProvider::new(vec![Err(genai::Error::Internal(
        "429 rate limit".to_string(),
    ))]);
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let err =
        run_loop_with_context_provider(&provider, &config, thread, &tools, RunContext::default())
            .await
            .expect_err("non-stream run should fail when provider always errors");
    assert!(
        matches!(err, AgentLoopError::LlmError(ref message) if message.contains("429")),
        "expected llm error with source message, got: {err:?}"
    );

    let recorded = phases.lock().expect("lock poisoned").clone();
    assert!(
        recorded.contains(&Phase::AfterInference),
        "cleanup should run AfterInference on llm error, got: {recorded:?}"
    );
    assert!(
        recorded.contains(&Phase::StepEnd),
        "cleanup should run StepEnd on llm error, got: {recorded:?}"
    );
    assert!(
        recorded.contains(&Phase::RunEnd),
        "run should still emit RunEnd on llm error, got: {recorded:?}"
    );
}

#[tokio::test]
async fn test_nonstream_stop_timeout_condition_triggers_on_natural_end_path() {
    let provider = MockChatProvider::new(vec![Ok(text_chat_response("done now"))]);
    let config = AgentConfig::new("mock").with_stop_condition(
        crate::engine::stop_conditions::Timeout(std::time::Duration::from_secs(0)),
    );
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let err =
        run_loop_with_context_provider(&provider, &config, thread, &tools, RunContext::default())
            .await
            .expect_err("timeout stop condition should stop non-stream run");

    match err {
        AgentLoopError::Stopped {
            state: thread,
            reason,
        } => {
            assert_eq!(reason, StopReason::TimeoutReached);
            assert!(
                thread
                    .messages
                    .iter()
                    .any(|m| m.role == crate::contracts::state::Role::Assistant),
                "assistant turn should still be committed before stop check"
            );
        }
        other => panic!("expected Stopped(TimeoutReached), got: {other:?}"),
    }
}

#[tokio::test]
async fn test_nonstream_cancellation_token_during_inference() {
    let ready = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let provider = HangingChatProvider {
        ready: ready.clone(),
        proceed: proceed.clone(),
        response: text_chat_response("never"),
    };
    let token = CancellationToken::new();
    let token_for_run = token.clone();

    let handle = tokio::spawn(async move {
        run_loop_with_context_provider(
            &provider,
            &AgentConfig::new("mock"),
            Thread::new("test").with_message(Message::user("go")),
            &HashMap::new(),
            RunContext {
                cancellation_token: Some(token_for_run),
                ..RunContext::default()
            },
        )
        .await
    });

    ready.notified().await;
    token.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_millis(300), handle)
        .await
        .expect("non-stream run should stop shortly after cancellation during inference")
        .expect("run task should not panic");
    proceed.notify_waiters();

    assert!(
        matches!(result, Err(AgentLoopError::Cancelled { .. })),
        "expected cancellation during inference, got: {result:?}"
    );
}

#[test]
fn test_loop_outcome_run_finish_projection_natural_end_has_result_payload() {
    let outcome = LoopOutcome {
        state: Thread::new("thread-1"),
        termination: TerminationReason::NaturalEnd,
        response: Some("final text".to_string()),
        usage: LoopUsage::default(),
        stats: LoopStats::default(),
        failure: None,
    };

    let event = outcome.to_run_finish_event("run-1".to_string());
    match event {
        AgentEvent::RunFinish {
            thread_id,
            run_id,
            result,
            termination,
        } => {
            assert_eq!(thread_id, "thread-1");
            assert_eq!(run_id, "run-1");
            assert_eq!(termination, TerminationReason::NaturalEnd);
            assert_eq!(result, Some(json!({ "response": "final text" })));
        }
        other => panic!("expected run finish event, got: {other:?}"),
    }
}

#[test]
fn test_loop_outcome_run_finish_projection_non_natural_has_no_result_payload() {
    let outcome = LoopOutcome {
        state: Thread::new("thread-2"),
        termination: TerminationReason::Cancelled,
        response: Some("ignored".to_string()),
        usage: LoopUsage::default(),
        stats: LoopStats::default(),
        failure: None,
    };

    let event = outcome.to_run_finish_event("run-2".to_string());
    match event {
        AgentEvent::RunFinish {
            result,
            termination,
            ..
        } => {
            assert_eq!(termination, TerminationReason::Cancelled);
            assert_eq!(result, None);
        }
        other => panic!("expected run finish event, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_nonstream_loop_outcome_collects_usage_and_stats() {
    let provider = MockChatProvider::new(vec![Ok(text_chat_response_with_usage("done", 7, 3))]);
    let config = AgentConfig::new("mock");
    let thread = Thread::new("usage-stats").with_message(Message::user("go"));
    let tools = HashMap::new();

    let outcome = run_loop_outcome_with_context_provider(
        &provider,
        &config,
        thread,
        &tools,
        RunContext::default(),
    )
    .await;

    assert_eq!(outcome.termination, TerminationReason::NaturalEnd);
    assert_eq!(outcome.response.as_deref(), Some("done"));
    assert_eq!(outcome.usage.prompt_tokens, 7);
    assert_eq!(outcome.usage.completion_tokens, 3);
    assert_eq!(outcome.usage.total_tokens, 10);
    assert_eq!(outcome.stats.steps, 1);
    assert_eq!(outcome.stats.llm_calls, 1);
    assert_eq!(outcome.stats.llm_retries, 0);
    assert_eq!(outcome.stats.tool_calls, 0);
    assert_eq!(outcome.stats.tool_errors, 0);
    assert!(outcome
        .state
        .messages
        .iter()
        .any(|m| m.role == crate::contracts::state::Role::Assistant && m.content == "done"));
}

#[tokio::test]
async fn test_nonstream_loop_outcome_llm_error_tracks_attempts_and_failure_kind() {
    let provider = MockChatProvider::new(vec![
        Err(genai::Error::Internal("429 rate limit".to_string())),
        Err(genai::Error::Internal("still failing".to_string())),
    ]);
    let config = AgentConfig::new("primary").with_llm_retry_policy(LlmRetryPolicy {
        max_attempts_per_model: 2,
        initial_backoff_ms: 1,
        max_backoff_ms: 1,
        retry_stream_start: true,
    });
    let thread = Thread::new("error-stats").with_message(Message::user("go"));
    let tools = HashMap::new();

    let outcome = run_loop_outcome_with_context_provider(
        &provider,
        &config,
        thread,
        &tools,
        RunContext::default(),
    )
    .await;

    assert_eq!(outcome.termination, TerminationReason::Error);
    assert_eq!(outcome.stats.llm_calls, 2);
    assert_eq!(outcome.stats.llm_retries, 1);
    assert_eq!(outcome.stats.steps, 0);
    assert!(matches!(
        outcome.failure,
        Some(outcome::LoopFailure::Llm(message)) if message.contains("model='primary' attempt=2/2")
    ));
}

#[tokio::test]
async fn test_nonstream_cancellation_token_during_tool_execution() {
    let ready = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let tool = ActivityGateTool {
        id: "activity_gate".to_string(),
        stream_id: "nonstream_cancel".to_string(),
        ready: ready.clone(),
        proceed,
    };
    let provider = MockChatProvider::new(vec![
        Ok(tool_call_chat_response(
            "call_1",
            "activity_gate",
            json!({}),
        )),
        Ok(text_chat_response("done")),
    ]);
    let token = CancellationToken::new();
    let token_for_run = token.clone();

    let handle = tokio::spawn(async move {
        run_loop_with_context_provider(
            &provider,
            &AgentConfig::new("mock"),
            Thread::new("test").with_message(Message::user("go")),
            &tool_map([tool]),
            RunContext {
                cancellation_token: Some(token_for_run),
                ..RunContext::default()
            },
        )
        .await
    });

    ready.notified().await;
    token.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_millis(300), handle)
        .await
        .expect("non-stream run should stop shortly after cancellation during tool execution")
        .expect("run task should not panic");

    let thread = match result {
        Err(AgentLoopError::Cancelled { state: thread }) => thread,
        other => panic!("expected Cancelled during tool execution, got: {other:?}"),
    };
    assert!(
        thread
            .messages
            .iter()
            .any(|m| m.role == crate::contracts::state::Role::Assistant),
        "assistant tool_call turn should be committed before cancellation"
    );
    assert!(
        !thread
            .messages
            .iter()
            .any(|m| m.role == crate::contracts::state::Role::Tool),
        "tool results should not be committed after cancellation"
    );
}

#[tokio::test]
async fn test_golden_run_loop_and_stream_natural_end_alignment() {
    let thread = Thread::new("golden-natural").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);
    let nonstream_provider = MockChatProvider::new(vec![
        Ok(tool_call_chat_response_object_args(
            "call_1",
            "echo",
            json!({"message": "aligned"}),
        )),
        Ok(text_chat_response("done")),
    ]);

    let (nonstream_thread, nonstream_response) = run_loop_with_context_provider(
        &nonstream_provider,
        &AgentConfig::new("mock"),
        thread.clone(),
        &tools,
        RunContext::default(),
    )
    .await
    .expect("non-stream run should succeed");

    let (events, stream_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![
            MockResponse::text("").with_tool_call("call_1", "echo", json!({"message": "aligned"})),
            MockResponse::text("done"),
        ]),
        AgentConfig::new("mock"),
        thread,
        tools.clone(),
    )
    .await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::NaturalEnd)
    );
    assert_eq!(
        extract_run_finish_response(&events),
        Some(nonstream_response.clone())
    );
    assert_eq!(
        compact_canonical_messages(&nonstream_thread),
        compact_canonical_messages(&stream_thread),
        "stream/non-stream should produce equivalent persisted message sequences"
    );
}

#[tokio::test]
async fn test_golden_run_loop_and_stream_cancelled_alignment() {
    let thread = Thread::new("golden-cancel").with_message(Message::user("go"));
    let tools = HashMap::new();
    let nonstream_provider = MockChatProvider::new(vec![Ok(text_chat_response("unused"))]);
    let nonstream_token = CancellationToken::new();
    nonstream_token.cancel();

    let nonstream_result = run_loop_with_context_provider(
        &nonstream_provider,
        &AgentConfig::new("mock"),
        thread.clone(),
        &tools,
        RunContext {
            cancellation_token: Some(nonstream_token),
            ..RunContext::default()
        },
    )
    .await;
    let nonstream_thread = match nonstream_result {
        Err(AgentLoopError::Cancelled { state: thread }) => *thread,
        other => panic!("expected non-stream cancellation, got: {other:?}"),
    };

    let stream_token = CancellationToken::new();
    stream_token.cancel();
    let (events, stream_thread) = run_mock_stream_with_final_thread_with_context(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        AgentConfig::new("mock"),
        thread,
        tools,
        RunContext {
            cancellation_token: Some(stream_token),
            ..RunContext::default()
        },
    )
    .await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Cancelled)
    );
    assert_eq!(extract_run_finish_response(&events), None);
    assert_eq!(
        compact_canonical_messages(&nonstream_thread),
        compact_canonical_messages(&stream_thread),
        "stream/non-stream cancellation should leave equivalent persisted messages"
    );
}

#[tokio::test]
async fn test_golden_run_loop_and_stream_pending_resume_alignment() {
    struct GoldenPendingPlugin;

    #[async_trait]
    impl AgentPlugin for GoldenPendingPlugin {
        fn id(&self) -> &str {
            "golden_pending_plugin"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::BeforeInference {
                return;
            }
            let state = step.snapshot();
            let patch = set_agent_pending_interaction(
                &state,
                Interaction::new("golden_resume_1", "recover_agent_run").with_message("resume me"),
            );
            step.pending_patches.push(patch);
            step.skip_inference = true;
        }
    }

    let thread = Thread::new("golden-resume").with_message(Message::user("continue"));
    let config =
        AgentConfig::new("mock").with_plugin(Arc::new(GoldenPendingPlugin) as Arc<dyn AgentPlugin>);
    let tools = HashMap::new();
    let nonstream_provider = MockChatProvider::new(vec![Ok(text_chat_response("unused"))]);

    let nonstream_result = run_loop_with_context_provider(
        &nonstream_provider,
        &config,
        thread.clone(),
        &tools,
        RunContext::default(),
    )
    .await;
    let (nonstream_thread, nonstream_interaction) = match nonstream_result {
        Err(AgentLoopError::PendingInteraction {
            state: thread,
            interaction,
        }) => (*thread, *interaction),
        other => panic!("expected non-stream pending interaction, got: {other:?}"),
    };

    let (events, stream_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        config,
        thread,
        tools,
    )
    .await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::PendingInteraction)
    );
    let stream_interaction =
        extract_requested_interaction(&events).expect("stream should emit requested interaction");
    assert_eq!(stream_interaction.id, nonstream_interaction.id);
    assert_eq!(stream_interaction.action, nonstream_interaction.action);
    assert_eq!(stream_interaction.message, nonstream_interaction.message);

    assert_eq!(
        compact_canonical_messages(&nonstream_thread),
        compact_canonical_messages(&stream_thread),
        "stream/non-stream pending path should preserve equivalent persisted messages"
    );

    let nonstream_state = nonstream_thread
        .rebuild_state()
        .expect("non-stream state should rebuild");
    let stream_state = stream_thread
        .rebuild_state()
        .expect("stream state should rebuild");
    assert_eq!(
        nonstream_state["loop_control"]["pending_interaction"],
        stream_state["loop_control"]["pending_interaction"]
    );
}

#[tokio::test]
async fn test_stream_replay_is_idempotent_across_reruns() {
    struct SkipInferencePlugin;

    #[async_trait]
    impl AgentPlugin for SkipInferencePlugin {
        fn id(&self) -> &str {
            "skip_inference_replay_idempotent"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    fn replay_config() -> AgentConfig {
        let interaction = carve_agent_extension_interaction::InteractionPlugin::with_responses(
            vec!["permission_counting_echo".to_string()],
            Vec::new(),
        );
        AgentConfig::new("mock")
            .with_plugin(Arc::new(interaction))
            .with_plugin(Arc::new(SkipInferencePlugin) as Arc<dyn AgentPlugin>)
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let counting_tool: Arc<dyn Tool> = Arc::new(CountingEchoTool {
        calls: calls.clone(),
    });
    let tools = tool_map_from_arc([counting_tool]);

    let thread = Thread::with_initial_state(
        "idempotent-replay",
        json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "permission_counting_echo",
                    "action": "tool:AskUserQuestion",
                    "parameters": {
                        "origin_tool_call": {
                            "id": "call_1",
                            "name": "counting_echo",
                            "arguments": { "message": "approved-run" }
                        }
                    }
                }
            }
        }),
    )
    .with_message(Message::assistant_with_tool_calls(
        "need permission",
        vec![crate::contracts::state::ToolCall::new(
            "call_1",
            "counting_echo",
            json!({"message": "approved-run"}),
        )],
    ))
    .with_message(Message::tool(
        "call_1",
        "Tool 'counting_echo' is awaiting approval. Execution paused.",
    ));

    let (first_events, first_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        replay_config(),
        thread,
        tools.clone(),
    )
    .await;
    assert!(
        first_events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCallDone { id, result, .. }
            if id == "call_1" && result.status == crate::contracts::tool::ToolStatus::Success
        )),
        "first run should replay and execute the pending tool call"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "replayed tool should execute exactly once in first run"
    );

    let (second_events, second_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        replay_config(),
        first_thread,
        tools,
    )
    .await;
    assert!(
        !second_events.iter().any(|e| matches!(
            e,
            AgentEvent::ToolCallDone { id, .. } if id == "call_1"
        )),
        "second run must not replay already-applied tool call"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "tool execution count must remain stable across reruns"
    );

    let final_state = second_thread.rebuild_state().expect("state should rebuild");
    let pending = final_state
        .get("loop_control")
        .and_then(|a| a.get("pending_interaction"));
    assert!(pending.is_none() || pending == Some(&Value::Null));
}

// ========================================================================
// Mock ChatStreamProvider for stop condition integration tests
// ========================================================================

/// A single mock LLM response: text and optional tool calls.
#[derive(Clone)]
struct MockResponse {
    text: String,
    tool_calls: Vec<genai::chat::ToolCall>,
    usage: Option<Usage>,
}

impl MockResponse {
    fn text(s: &str) -> Self {
        Self {
            text: s.to_string(),
            tool_calls: Vec::new(),
            usage: None,
        }
    }

    fn with_tool_call(mut self, call_id: &str, name: &str, args: Value) -> Self {
        self.tool_calls.push(genai::chat::ToolCall {
            call_id: call_id.to_string(),
            fn_name: name.to_string(),
            fn_arguments: Value::String(args.to_string()),
            thought_signatures: None,
        });
        self
    }

    fn with_usage(mut self, input: i32, output: i32) -> Self {
        self.usage = Some(Usage {
            prompt_tokens: Some(input),
            prompt_tokens_details: None,
            completion_tokens: Some(output),
            completion_tokens_details: None,
            total_tokens: Some(input + output),
        });
        self
    }
}

/// Mock provider that returns pre-configured responses in order.
/// After all responses are consumed, returns text-only (triggering NaturalEnd).
struct MockStreamProvider {
    responses: Mutex<Vec<MockResponse>>,
}

/// Provider that fails stream startup for a fixed number of calls, then succeeds.
struct FailingStartProvider {
    failures_left: Mutex<usize>,
    models_seen: Mutex<Vec<String>>,
}

impl FailingStartProvider {
    fn new(failures: usize) -> Self {
        Self {
            failures_left: Mutex::new(failures),
            models_seen: Mutex::new(Vec::new()),
        }
    }

    fn seen_models(&self) -> Vec<String> {
        self.models_seen.lock().expect("lock poisoned").clone()
    }
}

#[async_trait]
impl ChatStreamProvider for FailingStartProvider {
    async fn exec_chat_stream_events(
        &self,
        model: &str,
        _chat_req: genai::chat::ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<Pin<Box<dyn Stream<Item = genai::Result<ChatStreamEvent>> + Send>>> {
        self.models_seen
            .lock()
            .expect("lock poisoned")
            .push(model.to_string());
        let mut remaining = self.failures_left.lock().expect("lock poisoned");
        if *remaining > 0 {
            *remaining -= 1;
            return Err(genai::Error::Internal("429 rate limit".to_string()));
        }

        let events = vec![
            Ok(ChatStreamEvent::Start),
            Ok(ChatStreamEvent::Chunk(StreamChunk {
                content: "ok".to_string(),
            })),
            Ok(ChatStreamEvent::End(StreamEnd::default())),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

impl MockStreamProvider {
    fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl ChatStreamProvider for MockStreamProvider {
    async fn exec_chat_stream_events(
        &self,
        _model: &str,
        _chat_req: genai::chat::ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<Pin<Box<dyn Stream<Item = genai::Result<ChatStreamEvent>> + Send>>> {
        let resp = {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                MockResponse::text("done")
            } else {
                responses.remove(0)
            }
        };

        let mut events: Vec<genai::Result<ChatStreamEvent>> = Vec::new();
        events.push(Ok(ChatStreamEvent::Start));

        if !resp.text.is_empty() {
            events.push(Ok(ChatStreamEvent::Chunk(StreamChunk {
                content: resp.text.clone(),
            })));
        }

        for tc in &resp.tool_calls {
            events.push(Ok(ChatStreamEvent::ToolCallChunk(ToolChunk {
                tool_call: tc.clone(),
            })));
        }

        let end = StreamEnd {
            captured_content: if resp.tool_calls.is_empty() {
                None
            } else {
                Some(MessageContent::from_tool_calls(resp.tool_calls))
            },
            captured_usage: resp.usage,
            ..Default::default()
        };
        events.push(Ok(ChatStreamEvent::End(end)));

        Ok(Box::pin(futures::stream::iter(events)))
    }
}

/// Helper: run a mock stream and collect events.
async fn run_mock_stream(
    provider: MockStreamProvider,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
) -> Vec<AgentEvent> {
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(provider),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    collect_stream_events(stream).await
}

#[tokio::test]
async fn test_stream_serialization_emits_seq_timestamp_and_step_id() {
    let events = run_mock_stream(
        MockStreamProvider::new(vec![MockResponse::text("hello")]),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        HashMap::new(),
    )
    .await;

    let serialized: Vec<Value> = events
        .iter()
        .map(|event| serde_json::to_value(event).expect("serialize event"))
        .collect();
    assert!(!serialized.is_empty());

    for (idx, event) in serialized.iter().enumerate() {
        assert_eq!(
            event.get("seq").and_then(Value::as_u64),
            Some(idx as u64),
            "seq mismatch at index {idx}: {event:?}"
        );
        assert!(
            event.get("timestamp_ms").and_then(Value::as_u64).is_some(),
            "timestamp_ms missing at index {idx}: {event:?}"
        );
    }

    let step_start = serialized
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("step_start"))
        .expect("step_start event");
    assert_eq!(
        step_start.get("step_id").and_then(Value::as_str),
        Some("step:0")
    );

    let text_delta = serialized
        .iter()
        .find(|event| event.get("type").and_then(Value::as_str) == Some("text_delta"))
        .expect("text_delta event");
    assert_eq!(
        text_delta.get("step_id").and_then(Value::as_str),
        Some("step:0")
    );
    assert!(text_delta.get("run_id").and_then(Value::as_str).is_some());
    assert!(text_delta
        .get("thread_id")
        .and_then(Value::as_str)
        .is_some());
}

#[tokio::test]
async fn test_stream_retries_startup_error_then_succeeds() {
    let provider = Arc::new(FailingStartProvider::new(1));
    let config = AgentConfig::new("mock").with_llm_retry_policy(LlmRetryPolicy {
        max_attempts_per_model: 2,
        initial_backoff_ms: 1,
        max_backoff_ms: 10,
        retry_stream_start: true,
    });
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let stream = run_loop_stream_impl_with_provider(
        provider.clone(),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    let events = collect_stream_events(stream).await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::NaturalEnd)
    );
    let seen = provider.seen_models();
    assert_eq!(seen, vec!["mock".to_string(), "mock".to_string()]);
}

#[tokio::test]
async fn test_stream_uses_fallback_model_after_primary_failures() {
    let provider = Arc::new(FailingStartProvider::new(2));
    let config = AgentConfig::new("primary")
        .with_fallback_model("fallback")
        .with_llm_retry_policy(LlmRetryPolicy {
            max_attempts_per_model: 2,
            initial_backoff_ms: 1,
            max_backoff_ms: 10,
            retry_stream_start: true,
        });
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let stream = run_loop_stream_impl_with_provider(
        provider.clone(),
        config,
        thread,
        tools,
        RunContext::default(),
    );
    let events = collect_stream_events(stream).await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::NaturalEnd)
    );
    let seen = provider.seen_models();
    assert_eq!(
        seen,
        vec![
            "primary".to_string(),
            "primary".to_string(),
            "fallback".to_string()
        ]
    );
    assert_eq!(
        extract_inference_model(&events),
        Some("fallback".to_string())
    );
}

/// Helper: run a mock stream and collect events plus final session.
async fn run_mock_stream_with_final_thread(
    provider: MockStreamProvider,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
) -> (Vec<AgentEvent>, Thread) {
    run_mock_stream_with_final_thread_with_context(
        provider,
        config,
        thread,
        tools,
        RunContext::default(),
    )
    .await
}

/// Helper: run a mock stream and collect events plus final session with explicit run context.
async fn run_mock_stream_with_final_thread_with_context(
    provider: MockStreamProvider,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> (Vec<AgentEvent>, Thread) {
    let mut final_thread = thread.clone();
    let (checkpoint_tx, mut checkpoint_rx) = tokio::sync::mpsc::unbounded_channel();
    let run_ctx = run_ctx.with_state_committer(Arc::new(ChannelStateCommitter::new(checkpoint_tx)));
    let stream =
        run_loop_stream_impl_with_provider(Arc::new(provider), config, thread, tools, run_ctx);
    let events = collect_stream_events(stream).await;
    while let Some(changeset) = checkpoint_rx.recv().await {
        changeset.apply_to(&mut final_thread);
    }
    (events, final_thread)
}

#[derive(Clone)]
struct RecordingStateCommitter {
    reasons: Arc<Mutex<Vec<CheckpointReason>>>,
    fail_on: Option<CheckpointReason>,
}

impl RecordingStateCommitter {
    fn new(fail_on: Option<CheckpointReason>) -> Self {
        Self {
            reasons: Arc::new(Mutex::new(Vec::new())),
            fail_on,
        }
    }

    fn reasons(&self) -> Vec<CheckpointReason> {
        self.reasons.lock().expect("lock poisoned").clone()
    }
}

#[async_trait]
impl StateCommitter for RecordingStateCommitter {
    async fn commit(
        &self,
        _thread_id: &str,
        changeset: crate::contracts::AgentChangeSet,
        precondition: VersionPrecondition,
    ) -> Result<u64, StateCommitError> {
        self.reasons
            .lock()
            .expect("lock poisoned")
            .push(changeset.reason.clone());

        if self
            .fail_on
            .as_ref()
            .is_some_and(|reason| *reason == changeset.reason)
        {
            return Err(StateCommitError::new(format!(
                "forced commit failure at {:?}",
                changeset.reason
            )));
        }
        let version = match precondition {
            VersionPrecondition::Any => 1,
            VersionPrecondition::Exact(version) => version.saturating_add(1),
        };
        Ok(version)
    }
}

/// Extract the termination from the RunFinish event.
fn extract_termination(events: &[AgentEvent]) -> Option<TerminationReason> {
    events.iter().find_map(|e| match e {
        AgentEvent::RunFinish { termination, .. } => Some(termination.clone()),
        _ => None,
    })
}

fn extract_run_finish_response(events: &[AgentEvent]) -> Option<String> {
    events.iter().find_map(|e| match e {
        AgentEvent::RunFinish { result, .. } => result
            .as_ref()
            .map(|_| AgentEvent::extract_response(result)),
        _ => None,
    })
}

fn extract_requested_interaction(events: &[AgentEvent]) -> Option<Interaction> {
    events.iter().find_map(|e| match e {
        AgentEvent::InteractionRequested { interaction } => Some(interaction.clone()),
        AgentEvent::Pending { interaction } => Some(interaction.clone()),
        _ => None,
    })
}

fn extract_inference_model(events: &[AgentEvent]) -> Option<String> {
    events.iter().find_map(|e| match e {
        AgentEvent::InferenceComplete { model, .. } => Some(model.clone()),
        _ => None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalMessage {
    role: crate::contracts::state::Role,
    content: String,
    tool_call_id: Option<String>,
    visibility: crate::contracts::state::Visibility,
    tool_calls: Vec<CanonicalToolCall>,
}

fn canonical_messages(thread: &Thread) -> Vec<CanonicalMessage> {
    thread
        .messages
        .iter()
        .map(|msg| {
            let mut tool_calls = msg
                .tool_calls
                .as_ref()
                .map(|calls| {
                    calls
                        .iter()
                        .map(|call| CanonicalToolCall {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.to_string(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            tool_calls.sort_by(|a, b| {
                a.id.cmp(&b.id)
                    .then_with(|| a.name.cmp(&b.name))
                    .then_with(|| a.arguments.cmp(&b.arguments))
            });

            CanonicalMessage {
                role: msg.role,
                content: msg.content.clone(),
                tool_call_id: msg.tool_call_id.clone(),
                visibility: msg.visibility,
                tool_calls,
            }
        })
        .collect()
}

fn compact_canonical_messages(thread: &Thread) -> Vec<CanonicalMessage> {
    let mut compacted = Vec::new();
    for msg in canonical_messages(thread) {
        if compacted.last() == Some(&msg) {
            continue;
        }
        compacted.push(msg);
    }
    compacted
}

#[tokio::test]
async fn test_stream_state_commit_failure_on_assistant_turn_emits_error_and_run_finish() {
    let committer = Arc::new(RecordingStateCommitter::new(Some(
        CheckpointReason::AssistantTurnCommitted,
    )));
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(MockStreamProvider::new(vec![MockResponse::text("done")])),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        HashMap::new(),
        RunContext::default().with_state_committer(committer.clone() as Arc<dyn StateCommitter>),
    );
    let events = collect_stream_events(stream).await;

    assert_eq!(extract_termination(&events), Some(TerminationReason::Error));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("state commit failed"))),
        "expected state commit error event, got: {events:?}"
    );
    assert_eq!(
        committer.reasons(),
        vec![
            CheckpointReason::AssistantTurnCommitted,
            CheckpointReason::RunFinished
        ]
    );
}

#[tokio::test]
async fn test_stream_state_commit_failure_on_tool_results_emits_error_before_tool_done() {
    let committer = Arc::new(RecordingStateCommitter::new(Some(
        CheckpointReason::ToolResultsCommitted,
    )));
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(MockStreamProvider::new(vec![
            MockResponse::text("tool").with_tool_call("call_1", "echo", json!({"message":"hi"}))
        ])),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        tool_map([EchoTool]),
        RunContext::default().with_state_committer(committer.clone() as Arc<dyn StateCommitter>),
    );
    let events = collect_stream_events(stream).await;

    assert_eq!(extract_termination(&events), Some(TerminationReason::Error));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCallReady { id, .. } if id == "call_1")),
        "tool round should begin before commit failure"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDone { .. })),
        "tool result events must not be emitted after tool commit failure"
    );
    assert_eq!(
        committer.reasons(),
        vec![
            CheckpointReason::AssistantTurnCommitted,
            CheckpointReason::ToolResultsCommitted,
            CheckpointReason::RunFinished
        ]
    );
}

#[tokio::test]
async fn test_stream_run_finished_commit_failure_emits_error_without_run_finish_event() {
    let committer = Arc::new(RecordingStateCommitter::new(Some(
        CheckpointReason::RunFinished,
    )));
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(MockStreamProvider::new(vec![MockResponse::text("done")])),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        HashMap::new(),
        RunContext::default().with_state_committer(committer.clone() as Arc<dyn StateCommitter>),
    );
    let events = collect_stream_events(stream).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("state commit failed"))),
        "expected run-finished commit error event, got: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::RunFinish { .. })),
        "run finish event should be suppressed when final force-commit fails"
    );
    assert_eq!(
        committer.reasons(),
        vec![
            CheckpointReason::AssistantTurnCommitted,
            CheckpointReason::RunFinished
        ]
    );
}

#[tokio::test]
async fn test_stream_skip_inference_force_commits_run_finished_delta() {
    let (recorder, _phases) = RecordAndSkipPlugin::new();
    let committer = Arc::new(RecordingStateCommitter::new(None));
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(MockStreamProvider::new(vec![])),
        AgentConfig::new("mock").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>),
        Thread::new("test").with_message(Message::user("go")),
        HashMap::new(),
        RunContext::default().with_state_committer(committer.clone() as Arc<dyn StateCommitter>),
    );
    let events = collect_stream_events(stream).await;

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::PluginRequested)
    );
    assert_eq!(committer.reasons(), vec![CheckpointReason::RunFinished]);
}

#[tokio::test]
async fn test_stream_replay_invalid_payload_emits_error_and_finish() {
    struct InvalidReplayPayloadPlugin;

    #[async_trait]
    impl AgentPlugin for InvalidReplayPayloadPlugin {
        fn id(&self) -> &str {
            "invalid_replay_payload"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::RunStart {
                step.pending_patches.push(
                    carve_state::TrackedPatch::new(Patch::new().with_op(Op::set(
                        carve_state::path!("interaction_outbox", "replay_tool_calls"),
                        json!({"bad": "payload"}),
                    )))
                    .with_source("test:invalid_replay_payload"),
                );
            }
        }
    }

    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(InvalidReplayPayloadPlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::new("test").with_message(Message::user("resume"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(
        MockStreamProvider::new(vec![MockResponse::text("should not run")]),
        config,
        thread,
        tools,
    )
    .await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::Error { message }
            if message.contains("interaction_outbox.replay_tool_calls")
        )),
        "expected replay payload parse error, got events: {events:?}"
    );
    assert!(
        matches!(
            events.last(),
            Some(AgentEvent::RunFinish {
                termination: TerminationReason::Error,
                ..
            })
        ),
        "expected terminal RunFinish after replay parse error, got: {:?}",
        events.last()
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::TextDelta { .. })),
        "stream should terminate before inference when replay payload is invalid"
    );
}

#[tokio::test]
async fn test_stream_replay_rebuild_state_failure_emits_error() {
    struct ReplayPlugin;

    #[async_trait]
    impl AgentPlugin for ReplayPlugin {
        fn id(&self) -> &str {
            "replay_state_failure"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::RunStart {
                let outbox = step.state_of::<InteractionOutbox>();
                outbox.replay_tool_calls_push(crate::contracts::state::ToolCall::new(
                    "replay_call_1",
                    "echo",
                    json!({"message": "resume"}),
                ));
            }
        }
    }

    let broken_patch = carve_state::TrackedPatch::new(
        Patch::new().with_op(Op::increment(carve_state::path!("missing_counter"), 1_i64)),
    )
    .with_source("test:broken_state");
    let thread = Thread::with_initial_state("test", json!({}))
        .with_message(Message::user("resume"))
        .with_patch(broken_patch);

    let config =
        AgentConfig::new("mock").with_plugin(Arc::new(ReplayPlugin) as Arc<dyn AgentPlugin>);
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(
        MockStreamProvider::new(vec![MockResponse::text("should not run")]),
        config,
        thread,
        tools,
    )
    .await;

    assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("State error") || message.contains("replay"))),
            "expected state rebuild error, got events: {events:?}"
        );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDone { .. })),
        "replay tool must not execute when state rebuild fails"
    );
}

#[tokio::test]
async fn test_stream_replay_tool_exec_respects_tool_phases() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static BEFORE_TOOL_EXECUTED: AtomicBool = AtomicBool::new(false);

    struct ReplayBlockingPlugin;

    #[async_trait]
    impl AgentPlugin for ReplayBlockingPlugin {
        fn id(&self) -> &str {
            "replay_blocking"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            match phase {
                Phase::RunStart => {
                    let outbox = step.state_of::<InteractionOutbox>();
                    outbox.replay_tool_calls_push(crate::contracts::state::ToolCall::new(
                        "replay_call_1",
                        "echo",
                        json!({"message": "resume"}),
                    ));
                }
                Phase::BeforeToolExecute if step.tool_call_id() == Some("replay_call_1") => {
                    BEFORE_TOOL_EXECUTED.store(true, Ordering::SeqCst);
                    step.block("blocked in replay");
                }
                _ => {}
            }
        }
    }

    BEFORE_TOOL_EXECUTED.store(false, Ordering::SeqCst);

    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(ReplayBlockingPlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::new("test").with_message(Message::user("resume"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(
        MockStreamProvider::new(vec![MockResponse::text("done")]),
        config,
        thread,
        tools,
    )
    .await;

    let replay_done = events.iter().find(|e| {
        matches!(
            e,
            AgentEvent::ToolCallDone { id, .. } if id == "replay_call_1"
        )
    });

    let replay_result = match replay_done {
        Some(AgentEvent::ToolCallDone { result, .. }) => result,
        _ => panic!("expected replay ToolCallDone event, got: {events:?}"),
    };

    assert!(
        BEFORE_TOOL_EXECUTED.load(Ordering::SeqCst),
        "BeforeToolExecute should run for replayed tool calls"
    );
    assert!(
        replay_result.is_error(),
        "blocked replay should produce an error tool result"
    );
}

#[tokio::test]
async fn test_stream_replay_without_placeholder_appends_tool_result_message() {
    struct ReplayPlugin;

    #[async_trait]
    impl AgentPlugin for ReplayPlugin {
        fn id(&self) -> &str {
            "replay_without_placeholder"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase == Phase::RunStart {
                let outbox = step.state_of::<InteractionOutbox>();
                outbox.replay_tool_calls_push(crate::contracts::state::ToolCall::new(
                    "replay_call_1",
                    "echo",
                    json!({"message": "resume"}),
                ));
            }
        }
    }

    let config =
        AgentConfig::new("mock").with_plugin(Arc::new(ReplayPlugin) as Arc<dyn AgentPlugin>);
    let thread = Thread::new("test").with_message(Message::user("resume"));
    let tools = tool_map([EchoTool]);

    let (_events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(vec![MockResponse::text("unused")]),
        config,
        thread,
        tools,
    )
    .await;

    let msg = final_thread
        .messages
        .iter()
        .find(|m| {
            m.role == crate::contracts::state::Role::Tool
                && m.tool_call_id.as_deref() == Some("replay_call_1")
        })
        .expect("replay should append a real tool message when no placeholder exists");
    assert!(
        !msg.content.contains("awaiting approval"),
        "replayed message must not remain placeholder"
    );
    assert!(
        msg.content.contains("\"echoed\":\"resume\""),
        "unexpected replay tool message: {}",
        msg.content
    );
}

#[tokio::test]
async fn test_stream_apply_error_still_runs_run_end_phase() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SESSION_END_RAN: AtomicBool = AtomicBool::new(false);

    struct PendingAndRunEndPlugin;

    #[async_trait]
    impl AgentPlugin for PendingAndRunEndPlugin {
        fn id(&self) -> &str {
            "pending_and_run_end"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            match phase {
                Phase::BeforeToolExecute => {
                    if let Some(call_id) = step.tool_call_id() {
                        step.pending(
                            Interaction::new(format!("confirm_{call_id}"), "confirm")
                                .with_message("needs confirmation"),
                        );
                    }
                }
                Phase::RunEnd => {
                    SESSION_END_RAN.store(true, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }

    SESSION_END_RAN.store(false, Ordering::SeqCst);

    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(PendingAndRunEndPlugin) as Arc<dyn AgentPlugin>)
        .with_parallel_tools(true);
    let thread = Thread::new("test").with_message(Message::user("run tools"));
    let responses = vec![MockResponse::text("run both")
        .with_tool_call("call_1", "echo", json!({"message": "a"}))
        .with_tool_call("call_2", "echo", json!({"message": "b"}))];
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;

    assert!(
        events.iter().any(|e| matches!(
            e,
            AgentEvent::Error { message }
            if message.contains("multiple pending interactions")
        )),
        "expected apply error when multiple pending interactions exist: {events:?}"
    );
    assert!(
        SESSION_END_RAN.load(Ordering::SeqCst),
        "RunEnd phase must run on apply_tool_results failure"
    );
}

// ========================================================================
// Stop condition integration tests
// ========================================================================

#[tokio::test]
async fn test_stop_max_rounds_via_stop_condition() {
    // Configure MaxRounds(2) as explicit stop condition.
    // Provider returns tool calls forever → should stop after 2 rounds.
    let responses: Vec<MockResponse> = (0..10)
        .map(|i| {
            MockResponse::text("calling echo").with_tool_call(
                &format!("c{i}"),
                "echo",
                json!({"message": "hi"}),
            )
        })
        .collect();

    let config =
        AgentConfig::new("mock").with_stop_condition(crate::engine::stop_conditions::MaxRounds(2));
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::MaxRoundsReached))
    );
}

#[tokio::test]
async fn test_stop_natural_end_no_tools() {
    // LLM returns text only → NaturalEnd.
    let provider = MockStreamProvider::new(vec![MockResponse::text("Hello!")]);
    let config = AgentConfig::new("mock");
    let thread = Thread::new("test").with_message(Message::user("hi"));
    let tools = HashMap::new();

    let events = run_mock_stream(provider, config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::NaturalEnd)
    );
}

#[test]
fn test_apply_tool_results_rejects_conflicting_parallel_state_patches() {
    let thread = Thread::with_initial_state("test", json!({}));
    let left = tool_execution_result(
        "call_a",
        Some(TrackedPatch::new(Patch::new().with_op(Op::set(
            carve_state::path!("debug", "shared"),
            json!(1),
        )))),
    );
    let right = tool_execution_result(
        "call_b",
        Some(TrackedPatch::new(Patch::new().with_op(Op::set(
            carve_state::path!("debug", "shared"),
            json!(2),
        )))),
    );

    let err = match apply_tool_results_to_session(thread, &[left, right], None, true) {
        Ok(_) => panic!("parallel conflicting patches should be rejected"),
        Err(err) => err,
    };
    match err {
        AgentLoopError::StateError(message) => {
            assert!(
                message.contains("conflicting parallel state patches"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected state error, got: {other:?}"),
    }
}

#[test]
fn test_apply_tool_results_accepts_disjoint_parallel_state_patches() {
    let thread = Thread::with_initial_state("test", json!({}));
    let left = tool_execution_result(
        "call_a",
        Some(TrackedPatch::new(Patch::new().with_op(Op::set(
            carve_state::path!("debug", "alpha"),
            json!(1),
        )))),
    );
    let right = tool_execution_result(
        "call_b",
        Some(TrackedPatch::new(Patch::new().with_op(Op::set(
            carve_state::path!("debug", "beta"),
            json!(2),
        )))),
    );

    let applied = apply_tool_results_to_session(thread, &[left, right], None, true)
        .expect("parallel disjoint patches should succeed");
    let state = applied.thread.rebuild_state().expect("state rebuild");
    assert_eq!(state["debug"]["alpha"], 1);
    assert_eq!(state["debug"]["beta"], 2);
}

#[tokio::test]
async fn test_stop_plugin_requested() {
    // SkipInferencePlugin → PluginRequested.
    let (recorder, _) = RecordAndSkipPlugin::new();
    let config = AgentConfig::new("mock").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
    let thread = Thread::new("test").with_message(Message::user("hi"));
    let tools = HashMap::new();

    let provider = MockStreamProvider::new(vec![]);
    let events = run_mock_stream(provider, config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::PluginRequested)
    );
}

#[tokio::test]
async fn test_stop_on_tool_condition() {
    // StopOnTool("finish") → first round calls echo, second calls finish.
    let responses = vec![
        MockResponse::text("step 1").with_tool_call("c1", "echo", json!({"message": "a"})),
        MockResponse::text("step 2").with_tool_call("c2", "finish_tool", json!({})),
    ];

    struct FinishTool;
    #[async_trait]
    impl Tool for FinishTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("finish_tool", "Finish", "Finishes the run")
        }
        async fn execute(
            &self,
            _args: Value,
            _ctx: &ToolCallContext<'_>,
        ) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::success("finish_tool", json!({"done": true})))
        }
    }

    let config = AgentConfig::new("mock").with_stop_condition(
        crate::engine::stop_conditions::StopOnTool("finish_tool".to_string()),
    );
    let thread = Thread::new("test").with_message(Message::user("go"));

    let mut tools = tool_map([EchoTool]);
    let ft: Arc<dyn Tool> = Arc::new(FinishTool);
    tools.insert("finish_tool".to_string(), ft);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::ToolCalled(
            "finish_tool".to_string(),
        )))
    );
}

#[tokio::test]
async fn test_stop_content_match_condition() {
    // ContentMatch("FINAL_ANSWER") → second response has it in the text.
    let responses = vec![
        MockResponse::text("thinking...").with_tool_call("c1", "echo", json!({"message": "a"})),
        MockResponse::text("here is the FINAL_ANSWER: 42").with_tool_call(
            "c2",
            "echo",
            json!({"message": "b"}),
        ),
    ];

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::ContentMatch(
            "FINAL_ANSWER".to_string(),
        ))
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(10));
    let thread = Thread::new("test").with_message(Message::user("solve"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::ContentMatched(
            "FINAL_ANSWER".to_string(),
        )))
    );
}

#[tokio::test]
async fn test_stop_token_budget_condition() {
    // TokenBudget with max_total=500 → second round pushes over budget.
    let responses = vec![
        MockResponse::text("step 1")
            .with_tool_call("c1", "echo", json!({"message": "a"}))
            .with_usage(200, 100),
        MockResponse::text("step 2")
            .with_tool_call("c2", "echo", json!({"message": "b"}))
            .with_usage(200, 100),
    ];

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::TokenBudget { max_total: 500 })
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(10));
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::TokenBudgetExceeded))
    );
}

#[tokio::test]
async fn test_stop_consecutive_errors_condition() {
    // ConsecutiveErrors(2) → all tool calls fail each round.
    let responses: Vec<MockResponse> = (0..5)
        .map(|i| {
            MockResponse::text(&format!("round {i}")).with_tool_call(
                &format!("c{i}"),
                "failing",
                json!({}),
            )
        })
        .collect();

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::ConsecutiveErrors(2))
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(10));
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([FailingTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(
            StopReason::ConsecutiveErrorsExceeded,
        ))
    );
}

#[tokio::test]
async fn test_stop_loop_detection_condition() {
    // LoopDetection(window=3) → same tool called repeatedly.
    let responses: Vec<MockResponse> = (0..5)
        .map(|i| {
            MockResponse::text(&format!("round {i}")).with_tool_call(
                &format!("c{i}"),
                "echo",
                json!({"message": "same"}),
            )
        })
        .collect();

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::LoopDetection { window: 3 })
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(10));
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::LoopDetected))
    );
}

#[tokio::test]
async fn test_stop_cancellation_token() {
    // Cancel before first inference.
    let token = CancellationToken::new();
    token.cancel();

    let provider = MockStreamProvider::new(vec![MockResponse::text("never")]);
    let config = AgentConfig::new("mock");
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let stream = run_loop_stream_impl_with_provider(
        Arc::new(provider),
        config,
        thread,
        tools,
        RunContext {
            cancellation_token: Some(token),
            ..RunContext::default()
        },
    );
    let events = collect_stream_events(stream).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Cancelled)
    );
}

#[tokio::test]
async fn test_stop_cancellation_token_during_inference_stream() {
    struct HangingStreamProvider;

    #[async_trait]
    impl ChatStreamProvider for HangingStreamProvider {
        async fn exec_chat_stream_events(
            &self,
            _model: &str,
            _chat_req: genai::chat::ChatRequest,
            _options: Option<&ChatOptions>,
        ) -> genai::Result<
            Pin<Box<dyn Stream<Item = genai::Result<genai::chat::ChatStreamEvent>> + Send>>,
        > {
            let stream = async_stream::stream! {
                yield Ok(ChatStreamEvent::Start);
                yield Ok(ChatStreamEvent::Chunk(StreamChunk {
                    content: "partial".to_string(),
                }));
                // Simulate a provider stream that hangs after emitting a partial response.
                let _: () = futures::future::pending().await;
            };
            Ok(Box::pin(stream))
        }
    }

    let token = CancellationToken::new();
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(HangingStreamProvider),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        HashMap::new(),
        RunContext {
            cancellation_token: Some(token.clone()),
            ..RunContext::default()
        },
    );

    let collect_task = tokio::spawn(async move { collect_stream_events(stream).await });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    token.cancel();

    let events = tokio::time::timeout(std::time::Duration::from_millis(250), collect_task)
        .await
        .expect("stream should stop shortly after cancellation")
        .expect("collector task should not panic");

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Cancelled)
    );
}

#[tokio::test]
async fn test_stop_condition_applies_on_natural_end_without_tools() {
    let responses = vec![MockResponse::text("done now")];
    let config = AgentConfig::new("mock").with_stop_condition(
        crate::engine::stop_conditions::ContentMatch("done".to_string()),
    );
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::ContentMatched(
            "done".to_string()
        )))
    );
}

#[tokio::test]
async fn test_run_loop_with_context_cancellation_token() {
    let (recorder, _phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools = HashMap::new();
    let client = Client::default();
    let token = CancellationToken::new();
    token.cancel();

    let result = run_loop_with_context(
        &client,
        &config,
        thread,
        &tools,
        RunContext {
            cancellation_token: Some(token),
            ..RunContext::default()
        },
    )
    .await;

    assert!(
        matches!(result, Err(AgentLoopError::Cancelled { .. })),
        "expected cancellation, got: {result:?}"
    );
}

#[tokio::test]
async fn test_stop_first_condition_wins() {
    // Both MaxRounds(1) and TokenBudget(50) should trigger after round 1.
    // MaxRounds is first in the list → it wins.
    let responses = vec![MockResponse::text("r1")
        .with_tool_call("c1", "echo", json!({"message": "a"}))
        .with_usage(100, 100)];

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(1))
        .with_stop_condition(crate::engine::stop_conditions::TokenBudget { max_total: 50 });
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    // MaxRounds listed first → wins
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::MaxRoundsReached))
    );
}

#[tokio::test]
async fn test_stop_default_max_rounds_from_config() {
    // No explicit stop_conditions → auto-creates MaxRounds from config.max_rounds.
    let responses: Vec<MockResponse> = (0..5)
        .map(|i| {
            MockResponse::text(&format!("r{i}")).with_tool_call(
                &format!("c{i}"),
                "echo",
                json!({"message": "a"}),
            )
        })
        .collect();

    let config = AgentConfig::new("mock").with_max_rounds(2);
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::MaxRoundsReached))
    );
}

#[tokio::test]
async fn test_stop_max_rounds_counts_no_tool_step() {
    // Single no-tool step should still count toward MaxRounds.
    let responses = vec![MockResponse::text("done")];
    let config =
        AgentConfig::new("mock").with_stop_condition(crate::engine::stop_conditions::MaxRounds(1));
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::MaxRoundsReached))
    );
}

#[tokio::test]
async fn test_termination_in_run_finish_event() {
    // Verify RunFinish event structure when stop condition triggers.
    let responses =
        vec![MockResponse::text("r1").with_tool_call("c1", "echo", json!({"message": "a"}))];

    let config =
        AgentConfig::new("mock").with_stop_condition(crate::engine::stop_conditions::MaxRounds(1));
    let thread = Thread::new("test-thread").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;

    let finish = events
        .iter()
        .find(|e| matches!(e, AgentEvent::RunFinish { .. }));
    assert!(finish.is_some());
    if let Some(AgentEvent::RunFinish {
        thread_id,
        termination,
        ..
    }) = finish
    {
        assert_eq!(thread_id, "test-thread");
        assert_eq!(
            *termination,
            TerminationReason::Stopped(StopReason::MaxRoundsReached)
        );
    }
}

#[tokio::test]
async fn test_consecutive_errors_resets_on_success() {
    // Round 1: failing tool (consecutive_errors=1)
    // Round 2: echo succeeds (consecutive_errors=0)
    // Round 3: failing tool (consecutive_errors=1)
    // ConsecutiveErrors(2) should NOT trigger — never reaches 2.
    let responses = vec![
        MockResponse::text("r1").with_tool_call("c1", "failing", json!({})),
        MockResponse::text("r2").with_tool_call("c2", "echo", json!({"message": "ok"})),
        MockResponse::text("r3").with_tool_call("c3", "failing", json!({})),
    ];

    let mut tools = tool_map([EchoTool]);
    let ft: Arc<dyn Tool> = Arc::new(FailingTool);
    tools.insert("failing".to_string(), ft);

    let config = AgentConfig::new("mock")
        .with_stop_condition(crate::engine::stop_conditions::ConsecutiveErrors(2))
        .with_stop_condition(crate::engine::stop_conditions::MaxRounds(3));
    let thread = Thread::new("test").with_message(Message::user("go"));

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    // Should hit MaxRounds(3), not ConsecutiveErrors
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::MaxRoundsReached))
    );
}

#[tokio::test]
async fn test_run_state_tracks_completed_steps() {
    let mut state = RunState::new();
    assert_eq!(state.completed_steps, 0);

    let tool_calls = vec![crate::contracts::state::ToolCall::new(
        "c1",
        "echo",
        json!({}),
    )];
    state.record_tool_step(&tool_calls, 0);
    mark_step_completed(&mut state);
    assert_eq!(state.completed_steps, 1);
    assert_eq!(state.consecutive_errors, 0);
    assert_eq!(state.tool_call_history.len(), 1);
}

#[tokio::test]
async fn test_run_state_tracks_token_usage() {
    let mut state = RunState::new();
    let result = StreamResult {
        text: "hello".to_string(),
        tool_calls: vec![],
        usage: Some(Usage {
            prompt_tokens: Some(100),
            prompt_tokens_details: None,
            completion_tokens: Some(50),
            completion_tokens_details: None,
            total_tokens: Some(150),
        }),
    };
    state.update_from_response(&result);
    assert_eq!(state.total_input_tokens, 100);
    assert_eq!(state.total_output_tokens, 50);

    state.update_from_response(&result);
    assert_eq!(state.total_input_tokens, 200);
    assert_eq!(state.total_output_tokens, 100);
}

#[tokio::test]
async fn test_run_state_caps_history_at_20() {
    let mut state = RunState::new();
    for i in 0..25 {
        let tool_calls = vec![crate::contracts::state::ToolCall::new(
            &format!("c{i}"),
            &format!("tool_{i}"),
            json!({}),
        )];
        state.record_tool_step(&tool_calls, 0);
    }
    assert_eq!(state.tool_call_history.len(), 20);
}

#[tokio::test]
async fn test_effective_stop_conditions_empty_uses_max_rounds() {
    let config = AgentConfig::new("mock").with_max_rounds(5);
    let conditions = effective_stop_conditions(&config);
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].id(), "max_rounds");
}

#[tokio::test]
async fn test_effective_stop_conditions_explicit_overrides() {
    let config = AgentConfig::new("mock")
        .with_max_rounds(5) // ignored when explicit conditions set
        .with_stop_condition(crate::engine::stop_conditions::Timeout(
            std::time::Duration::from_secs(30),
        ));
    let conditions = effective_stop_conditions(&config);
    assert_eq!(conditions.len(), 1);
    assert_eq!(conditions[0].id(), "timeout");
}

// ========================================================================
// Parallel Tool Execution: Partial Failure Tests
// ========================================================================

#[test]
fn test_parallel_tools_partial_failure() {
    // When running tools in parallel, a failing tool should produce an error
    // message, while the successful tool should still complete.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Call both".to_string(),
            tool_calls: vec![
                crate::contracts::state::ToolCall::new(
                    "call_echo",
                    "echo",
                    json!({"message": "ok"}),
                ),
                crate::contracts::state::ToolCall::new("call_fail", "failing", json!({})),
            ],
            usage: None,
        };

        let mut tools = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>);
        tools.insert(
            "failing".to_string(),
            Arc::new(FailingTool) as Arc<dyn Tool>,
        );

        let thread = execute_tools(thread, &result, &tools, true).await.unwrap();

        // Both tools produce messages.
        assert_eq!(
            thread.message_count(),
            2,
            "Both tools should produce a message"
        );

        // One should be success, one should be error.
        let contents: Vec<&str> = thread.messages.iter().map(|m| m.content.as_str()).collect();
        let has_success = contents.iter().any(|c| c.contains("echoed"));
        let has_error = contents
            .iter()
            .any(|c| c.to_lowercase().contains("error") || c.to_lowercase().contains("fail"));
        assert!(has_success, "Echo tool should succeed: {:?}", contents);
        assert!(
            has_error,
            "Failing tool should produce error: {:?}",
            contents
        );
    });
}

#[test]
fn test_parallel_tools_conflicting_state_patches_return_error() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::with_initial_state("test", json!({"counter": 0}));
        let result = StreamResult {
            text: "conflicting calls".to_string(),
            tool_calls: vec![
                crate::contracts::state::ToolCall::new("call_1", "counter", json!({"amount": 1})),
                crate::contracts::state::ToolCall::new("call_2", "counter", json!({"amount": 2})),
            ],
            usage: None,
        };
        let tools = tool_map([CounterTool]);

        let err = execute_tools(thread, &result, &tools, true)
            .await
            .expect_err("parallel conflicting patches should fail");
        assert!(
            matches!(err, AgentLoopError::StateError(ref msg) if msg.contains("conflict")),
            "expected conflict state error, got: {err:?}"
        );
    });
}

#[test]
fn test_sequential_tools_partial_failure() {
    // Same test but with sequential execution.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Call both".to_string(),
            tool_calls: vec![
                crate::contracts::state::ToolCall::new(
                    "call_echo",
                    "echo",
                    json!({"message": "ok"}),
                ),
                crate::contracts::state::ToolCall::new("call_fail", "failing", json!({})),
            ],
            usage: None,
        };

        let mut tools = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>);
        tools.insert(
            "failing".to_string(),
            Arc::new(FailingTool) as Arc<dyn Tool>,
        );

        let thread = execute_tools(thread, &result, &tools, false).await.unwrap();

        assert_eq!(
            thread.message_count(),
            2,
            "Both tools should produce a message"
        );
        let contents: Vec<&str> = thread.messages.iter().map(|m| m.content.as_str()).collect();
        let has_success = contents.iter().any(|c| c.contains("echoed"));
        let has_error = contents
            .iter()
            .any(|c| c.to_lowercase().contains("error") || c.to_lowercase().contains("fail"));
        assert!(has_success, "Echo tool should succeed: {:?}", contents);
        assert!(
            has_error,
            "Failing tool should produce error: {:?}",
            contents
        );
    });
}

#[tokio::test]
async fn test_sequential_tools_stop_after_first_pending_interaction() {
    struct PendingEveryToolPlugin {
        seen_calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AgentPlugin for PendingEveryToolPlugin {
        fn id(&self) -> &str {
            "pending_every_tool"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::BeforeToolExecute {
                return;
            }
            if let Some(call_id) = step.tool_call_id() {
                self.seen_calls
                    .lock()
                    .expect("lock poisoned")
                    .push(call_id.to_string());
                step.pending(
                    Interaction::new(format!("confirm_{call_id}"), "confirm")
                        .with_message("needs confirmation"),
                );
            }
        }
    }

    let seen_calls = Arc::new(Mutex::new(Vec::new()));
    let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PendingEveryToolPlugin {
        seen_calls: seen_calls.clone(),
    }) as Arc<dyn AgentPlugin>];

    let thread = Thread::new("test");
    let result = StreamResult {
        text: "Call both".to_string(),
        tool_calls: vec![
            crate::contracts::state::ToolCall::new("call_1", "echo", json!({"message":"a"})),
            crate::contracts::state::ToolCall::new("call_2", "echo", json!({"message":"b"})),
        ],
        usage: None,
    };
    let tools = tool_map([EchoTool]);

    let err = execute_tools_with_plugins(thread, &result, &tools, false, &plugins)
        .await
        .expect_err("sequential mode should pause on first pending interaction");
    let (thread, interaction) = match err {
        AgentLoopError::PendingInteraction {
            state: thread,
            interaction,
        } => (thread, interaction),
        other => panic!("expected PendingInteraction, got: {other:?}"),
    };
    assert_eq!(interaction.id, "confirm_call_1");
    assert_eq!(
        seen_calls.lock().expect("lock poisoned").clone(),
        vec!["call_1".to_string()],
        "second tool must not execute after first pending interaction in sequential mode"
    );
    assert_eq!(thread.message_count(), 1);
    assert_eq!(thread.messages[0].tool_call_id.as_deref(), Some("call_1"));
}

// ========================================================================
// Plugin Execution Order Tests
// ========================================================================

/// Plugin that records when it runs (appends to a shared Vec).
struct OrderTrackingPlugin {
    id: &'static str,
    order_log: Arc<std::sync::Mutex<Vec<String>>>,
}

#[async_trait]
impl AgentPlugin for OrderTrackingPlugin {
    fn id(&self) -> &str {
        self.id
    }

    async fn on_phase(&self, phase: Phase, _step: &mut StepContext<'_>) {
        self.order_log
            .lock()
            .unwrap()
            .push(format!("{}:{:?}", self.id, phase));
    }
}

#[test]
fn test_plugin_execution_order_preserved() {
    // Plugins should execute in the order they are provided.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));

        let plugin_a = OrderTrackingPlugin {
            id: "plugin_a",
            order_log: Arc::clone(&log),
        };
        let plugin_b = OrderTrackingPlugin {
            id: "plugin_b",
            order_log: Arc::clone(&log),
        };

        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Test".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(plugin_a), Arc::new(plugin_b)];

        let _ = execute_tools_with_plugins(thread, &result, &tools, false, &plugins).await;

        let entries = log.lock().unwrap().clone();

        // For each phase, plugin_a should appear before plugin_b.
        let before_a = entries
            .iter()
            .position(|e| e.starts_with("plugin_a:BeforeToolExecute"));
        let before_b = entries
            .iter()
            .position(|e| e.starts_with("plugin_b:BeforeToolExecute"));
        if let (Some(a), Some(b)) = (before_a, before_b) {
            assert!(
                a < b,
                "plugin_a should run before plugin_b in BeforeToolExecute phase"
            );
        }

        let after_a = entries
            .iter()
            .position(|e| e.starts_with("plugin_a:AfterToolExecute"));
        let after_b = entries
            .iter()
            .position(|e| e.starts_with("plugin_b:AfterToolExecute"));
        if let (Some(a), Some(b)) = (after_a, after_b) {
            assert!(
                a < b,
                "plugin_a should run before plugin_b in AfterToolExecute phase"
            );
        }
    });
}

/// Plugin that blocks if it runs after another plugin has already set pending.
struct ConditionalBlockPlugin;

#[async_trait]
impl AgentPlugin for ConditionalBlockPlugin {
    fn id(&self) -> &str {
        "conditional_block"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeToolExecute && step.tool_pending() {
            step.block("Blocked because tool was pending".to_string());
        }
    }
}

#[test]
fn test_plugin_order_affects_outcome() {
    // When PendingPhasePlugin runs FIRST, it sets pending. Then
    // ConditionalBlockPlugin sees pending and blocks. Net result: blocked.
    // When reversed, ConditionalBlockPlugin sees no pending (does nothing),
    // then PendingPhasePlugin sets pending. Net result: pending (not blocked).
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let thread = Thread::new("test");
        let result = StreamResult {
            text: "Test".to_string(),
            tool_calls: vec![crate::contracts::state::ToolCall::new(
                "call_1",
                "echo",
                json!({"message": "test"}),
            )],
            usage: None,
        };
        let tools = tool_map([EchoTool]);

        // Order 1: PendingPhasePlugin first → ConditionalBlockPlugin blocks.
        let plugins_order1: Vec<Arc<dyn AgentPlugin>> = vec![
            Arc::new(PendingPhasePlugin),
            Arc::new(ConditionalBlockPlugin),
        ];
        let r1 =
            execute_tools_with_plugins(thread.clone(), &result, &tools, false, &plugins_order1)
                .await;
        // When pending+blocked, the blocked result takes priority.
        let s1 = r1.unwrap();
        assert_eq!(s1.message_count(), 1);
        assert!(
            s1.messages[0].content.to_lowercase().contains("blocked")
                || s1.messages[0].content.to_lowercase().contains("pending"),
            "Order 1 should block or produce error: {}",
            s1.messages[0].content
        );

        // Order 2: ConditionalBlockPlugin first → sees no pending → PendingPhasePlugin sets pending.
        let plugins_order2: Vec<Arc<dyn AgentPlugin>> = vec![
            Arc::new(ConditionalBlockPlugin),
            Arc::new(PendingPhasePlugin),
        ];
        let r2 = execute_tools_with_plugins(thread, &result, &tools, false, &plugins_order2).await;
        // Should be PendingInteraction (not blocked).
        assert!(r2.is_err(), "Order 2 should result in PendingInteraction");
        match r2.unwrap_err() {
            AgentLoopError::PendingInteraction { .. } => {}
            other => panic!("Expected PendingInteraction, got: {:?}", other),
        }
    });
}

// ========================================================================
// Message ID alignment integration tests
// ========================================================================
//
// These tests verify that pre-generated message IDs flow correctly through
// the entire pipeline: streaming AgentEvents → stored Thread messages →
// AG-UI protocol events → AI SDK protocol events.

/// Verify that `StepStart.message_id` matches the stored assistant `Message.id`.
#[tokio::test]
async fn test_message_id_stepstart_matches_stored_assistant_message() {
    let responses = vec![MockResponse::text("Hello world")];
    let config = AgentConfig::new("mock");
    let thread = Thread::new("test").with_message(Message::user("hi"));

    let (events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(responses),
        config,
        thread,
        HashMap::new(),
    )
    .await;

    // Extract message_id from StepStart event.
    let step_msg_id = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::StepStart { message_id } => Some(message_id.clone()),
            _ => None,
        })
        .expect("stream must contain a StepStart event");

    // The pre-generated ID must be a valid UUID v7.
    assert_eq!(step_msg_id.len(), 36, "message_id should be a UUID");
    assert_eq!(&step_msg_id[14..15], "7", "message_id should be UUID v7");

    // Find the assistant message stored in the final thread.
    let assistant_msg = final_thread
        .messages
        .iter()
        .find(|m| m.role == crate::contracts::state::Role::Assistant)
        .expect("final thread must contain an assistant message");

    assert_eq!(
        assistant_msg.id.as_deref(),
        Some(step_msg_id.as_str()),
        "StepStart.message_id must equal stored assistant Message.id"
    );
}

/// Verify that `ToolCallDone.message_id` matches the stored tool `Message.id`.
#[tokio::test]
async fn test_message_id_toolcalldone_matches_stored_tool_message() {
    // Two responses: first triggers a tool call, second is the final answer.
    let responses = vec![
        MockResponse::text("let me search").with_tool_call(
            "call_1",
            "echo",
            json!({"message": "test"}),
        ),
        MockResponse::text("found it"),
    ];
    let config = AgentConfig::new("mock");
    let thread = Thread::new("test").with_message(Message::user("search"));
    let tools = tool_map([EchoTool]);

    let (events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(responses),
        config,
        thread,
        tools,
    )
    .await;

    // Extract message_id from the ToolCallDone event.
    let tool_done_msg_id = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolCallDone { message_id, .. } => Some(message_id.clone()),
            _ => None,
        })
        .expect("stream must contain a ToolCallDone event");

    assert_eq!(
        tool_done_msg_id.len(),
        36,
        "tool message_id should be a UUID"
    );

    // Find the tool result message in the final thread.
    let tool_msg = final_thread
        .messages
        .iter()
        .find(|m| m.role == crate::contracts::state::Role::Tool)
        .expect("final thread must contain a tool message");

    assert_eq!(
        tool_msg.id.as_deref(),
        Some(tool_done_msg_id.as_str()),
        "ToolCallDone.message_id must equal stored tool Message.id"
    );
}

/// End-to-end: run a multi-step stream with tool calls and verify all message IDs
/// are consistent across runtime events and stored messages.
#[tokio::test]
async fn test_message_id_end_to_end_multi_step() {
    // Step 1: tool call round. Step 2: final text answer.
    let responses = vec![
        MockResponse::text("searching").with_tool_call("c1", "echo", json!({"message": "query"})),
        MockResponse::text("final answer"),
    ];
    let config = AgentConfig::new("mock");
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = tool_map([EchoTool]);

    let (events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(responses),
        config,
        thread,
        tools,
    )
    .await;

    // Collect all StepStart message_ids and ToolCallDone message_ids.
    let step_ids: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::StepStart { message_id } => Some(message_id.clone()),
            _ => None,
        })
        .collect();
    let tool_ids: Vec<(String, String)> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolCallDone { id, message_id, .. } => {
                Some((id.clone(), message_id.clone()))
            }
            _ => None,
        })
        .collect();

    assert_eq!(step_ids.len(), 2, "two steps expected (tool round + final)");
    assert_eq!(tool_ids.len(), 1, "one tool call done expected");

    // All IDs must be distinct.
    let all_ids: Vec<&str> = step_ids
        .iter()
        .map(|s| s.as_str())
        .chain(tool_ids.iter().map(|(_, mid)| mid.as_str()))
        .collect();
    let unique: std::collections::HashSet<&str> = all_ids.iter().copied().collect();
    assert_eq!(
        all_ids.len(),
        unique.len(),
        "all pre-generated IDs must be unique"
    );

    // Verify stored assistant messages match step IDs.
    let assistant_msgs: Vec<&Arc<Message>> = final_thread
        .messages
        .iter()
        .filter(|m| m.role == crate::contracts::state::Role::Assistant)
        .collect();
    assert_eq!(assistant_msgs.len(), 2);
    assert_eq!(assistant_msgs[0].id.as_deref(), Some(step_ids[0].as_str()));
    assert_eq!(assistant_msgs[1].id.as_deref(), Some(step_ids[1].as_str()));

    // Verify stored tool message matches ToolCallDone ID.
    let tool_msgs: Vec<&Arc<Message>> = final_thread
        .messages
        .iter()
        .filter(|m| m.role == crate::contracts::state::Role::Tool)
        .collect();
    assert_eq!(tool_msgs.len(), 1);
    assert_eq!(
        tool_msgs[0].id.as_deref(),
        Some(tool_ids[0].1.as_str()),
        "stored tool Message.id must match ToolCallDone.message_id"
    );
}

#[tokio::test]
async fn test_run_step_skip_inference_returns_empty_result_without_assistant_message() {
    let (recorder, phases) = RecordAndSkipPlugin::new();
    let config =
        AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let (next_thread, result) = run_step(&Client::default(), &config, thread, &tools)
        .await
        .expect("run_step should skip inference and return empty result");

    assert_eq!(result.text, "");
    assert!(result.tool_calls.is_empty());
    assert!(result.usage.is_none());
    assert_eq!(next_thread.message_count(), 1);

    let recorded = phases.lock().expect("lock poisoned").clone();
    assert_eq!(recorded, vec![Phase::StepStart, Phase::BeforeInference]);
}

#[tokio::test]
async fn test_run_step_skip_inference_with_pending_state_returns_pending_interaction() {
    struct PendingSkipStepPlugin;

    #[async_trait]
    impl AgentPlugin for PendingSkipStepPlugin {
        fn id(&self) -> &str {
            "pending_skip_step"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            if phase != Phase::BeforeInference {
                return;
            }
            let state = step.snapshot();
            let patch = set_agent_pending_interaction(
                &state,
                Interaction::new("agent_recovery_step-1", "recover_agent_run")
                    .with_message("resume step?"),
            );
            step.pending_patches.push(patch);
            step.skip_inference = true;
        }
    }

    let config = AgentConfig::new("gpt-4o-mini")
        .with_plugin(Arc::new(PendingSkipStepPlugin) as Arc<dyn AgentPlugin>);
    let thread =
        Thread::new("test").with_message(crate::contracts::state::Message::user("hello"));
    let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

    let result = run_step(&Client::default(), &config, thread, &tools).await;
    let (thread, interaction) = match result {
        Err(AgentLoopError::PendingInteraction {
            state: thread,
            interaction,
        }) => (thread, interaction),
        other => panic!("expected PendingInteraction on skip_inference, got: {other:?}"),
    };
    assert_eq!(interaction.action, "recover_agent_run");
    assert_eq!(interaction.message, "resume step?");

    let state = thread.rebuild_state().expect("state should rebuild");
    assert_eq!(
        state["loop_control"]["pending_interaction"]["action"],
        Value::String("recover_agent_run".to_string())
    );
}

#[tokio::test]
async fn test_stream_tool_execution_injects_scope_context_for_tools() {
    let responses = vec![
        MockResponse::text("call scope").with_tool_call("call_1", "scope_snapshot", json!({})),
        MockResponse::text("done"),
    ];
    let config = AgentConfig::new("mock");
    let thread = Thread::with_initial_state("stream-caller", json!({"k":"v"}))
        .with_message(Message::user("hello"));
    let tools = tool_map([ScopeSnapshotTool]);

    let (_events, final_thread) = run_mock_stream_with_final_thread(
        MockStreamProvider::new(responses),
        config,
        thread,
        tools,
    )
    .await;

    let tool_msg = final_thread
        .messages
        .iter()
        .find(|m| {
            m.role == crate::contracts::state::Role::Tool
                && m.tool_call_id.as_deref() == Some("call_1")
        })
        .expect("scope snapshot tool result should exist");
    let tool_result: ToolResult =
        serde_json::from_str(&tool_msg.content).expect("tool result json");
    assert_eq!(
        tool_result.status,
        crate::contracts::tool::ToolStatus::Success
    );
    assert_eq!(tool_result.data["thread_id"], json!("stream-caller"));
    assert_eq!(tool_result.data["state"]["k"], json!("v"));
    assert_eq!(tool_result.data["messages_len"], json!(2));
}

#[tokio::test]
async fn test_stream_startup_error_runs_cleanup_phases_and_persists_cleanup_patch() {
    struct CleanupOnStartErrorPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    #[async_trait]
    impl AgentPlugin for CleanupOnStartErrorPlugin {
        fn id(&self) -> &str {
            "cleanup_on_start_error"
        }

        async fn on_phase(
            &self,
            phase: Phase,
            step: &mut StepContext<'_>,
        ) {
            self.phases.lock().expect("lock poisoned").push(phase);
            match phase {
                Phase::AfterInference => {
                    let agent = step.state_of::<crate::runtime::control::LoopControlState>();
                    let err_type = agent.inference_error().ok().flatten().map(|e| e.error_type);
                    assert_eq!(err_type.as_deref(), Some("llm_stream_start_error"));
                }
                Phase::StepEnd => {
                    step.pending_patches.push(
                        TrackedPatch::new(Patch::new().with_op(Op::set(
                            carve_state::path!("debug", "cleanup_ran"),
                            json!(true),
                        )))
                        .with_source("test:cleanup_on_start_error"),
                    );
                }
                _ => {}
            }
        }
    }

    let phases = Arc::new(Mutex::new(Vec::new()));
    let config = AgentConfig::new("mock")
        .with_plugin(Arc::new(CleanupOnStartErrorPlugin {
            phases: phases.clone(),
        }) as Arc<dyn AgentPlugin>)
        .with_llm_retry_policy(LlmRetryPolicy {
            max_attempts_per_model: 1,
            initial_backoff_ms: 1,
            max_backoff_ms: 1,
            retry_stream_start: true,
        });

    let initial_thread =
        Thread::with_initial_state("test", json!({})).with_message(Message::user("go"));
    let mut final_thread = initial_thread.clone();
    let (checkpoint_tx, mut checkpoint_rx) = tokio::sync::mpsc::unbounded_channel();
    let run_ctx = RunContext::default()
        .with_state_committer(Arc::new(ChannelStateCommitter::new(checkpoint_tx)));

    let events = collect_stream_events(run_loop_stream_impl_with_provider(
        Arc::new(FailingStartProvider::new(10)),
        config,
        initial_thread,
        HashMap::new(),
        run_ctx,
    ))
    .await;

    while let Some(changeset) = checkpoint_rx.recv().await {
        changeset.apply_to(&mut final_thread);
    }

    assert_eq!(extract_termination(&events), Some(TerminationReason::Error));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("429"))),
        "expected stream error event from startup failure, got: {events:?}"
    );

    let recorded = phases.lock().expect("lock poisoned").clone();
    assert!(
        recorded.contains(&Phase::AfterInference),
        "cleanup should run AfterInference on startup failure, got: {recorded:?}"
    );
    assert!(
        recorded.contains(&Phase::StepEnd),
        "cleanup should run StepEnd on startup failure, got: {recorded:?}"
    );
    assert!(
        recorded.contains(&Phase::RunEnd),
        "run should still emit RunEnd on startup failure, got: {recorded:?}"
    );

    let state = final_thread.rebuild_state().expect("state should rebuild");
    assert_eq!(state["debug"]["cleanup_ran"], true);
}

#[tokio::test]
async fn test_stop_timeout_condition_triggers_on_natural_end_path() {
    let responses = vec![MockResponse::text("done now")];
    let config = AgentConfig::new("mock").with_stop_condition(
        crate::engine::stop_conditions::Timeout(std::time::Duration::from_secs(0)),
    );
    let thread = Thread::new("test").with_message(Message::user("go"));
    let tools = HashMap::new();

    let events = run_mock_stream(MockStreamProvider::new(responses), config, thread, tools).await;
    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Stopped(StopReason::TimeoutReached))
    );
}

#[tokio::test]
async fn test_stop_cancellation_token_during_tool_execution_stream() {
    let ready = Arc::new(Notify::new());
    let proceed = Arc::new(Notify::new());
    let tool = ActivityGateTool {
        id: "activity_gate".to_string(),
        stream_id: "stream_cancel".to_string(),
        ready: ready.clone(),
        proceed,
    };

    let responses = vec![MockResponse::text("running tool").with_tool_call(
        "call_1",
        "activity_gate",
        json!({}),
    )];
    let token = CancellationToken::new();
    let stream = run_loop_stream_impl_with_provider(
        Arc::new(MockStreamProvider::new(responses)),
        AgentConfig::new("mock"),
        Thread::new("test").with_message(Message::user("go")),
        tool_map([tool]),
        RunContext {
            cancellation_token: Some(token.clone()),
            ..RunContext::default()
        },
    );

    let collector = tokio::spawn(async move { collect_stream_events(stream).await });
    ready.notified().await;
    token.cancel();

    let events = tokio::time::timeout(std::time::Duration::from_millis(300), collector)
        .await
        .expect("stream should stop shortly after cancellation during tool execution")
        .expect("collector task should not panic");

    assert_eq!(
        extract_termination(&events),
        Some(TerminationReason::Cancelled)
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::ToolCallDone { .. })),
        "tool should not report completion after cancellation"
    );
}
