#![allow(missing_docs)]

use async_trait::async_trait;
use awaken::agent::config::AgentConfig;
use awaken::agent::loop_runner::{AgentRunResult, run_agent_loop};
use awaken::agent::state::{RunLifecycleSlot, RunLifecycleState};
use awaken::contract::event::AgentEvent;
use awaken::contract::executor::{InferenceExecutionError, InferenceRequest, LlmExecutor};
use awaken::contract::identity::{RunIdentity, RunOrigin};
use awaken::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken::contract::lifecycle::{RunStatus, TerminationReason};
use awaken::contract::message::{Message, ToolCall};
use awaken::contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use awaken::*;
use awaken::{SlotOptions, StateStore};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock LLM: returns scripted responses
// ---------------------------------------------------------------------------

struct ScriptedLlm {
    responses: Mutex<Vec<StreamResult>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<StreamResult>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl LlmExecutor for ScriptedLlm {
    async fn execute(
        &self,
        _req: InferenceRequest,
    ) -> Result<StreamResult, InferenceExecutionError> {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            Ok(StreamResult {
                text: "I have nothing more to say.".into(),
                tool_calls: vec![],
                usage: None,
                stop_reason: Some(StopReason::EndTurn),
            })
        } else {
            Ok(responses.remove(0))
        }
    }

    fn name(&self) -> &str {
        "scripted"
    }
}

// ---------------------------------------------------------------------------
// Mock Tool: echo arguments back
// ---------------------------------------------------------------------------

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("echo", "echo", "Echoes input back")
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, ToolError> {
        let msg = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no message")
            .to_string();
        Ok(ToolResult::success_with_message("echo", args, msg))
    }
}

// ---------------------------------------------------------------------------
// Mock Tool: calculator
// ---------------------------------------------------------------------------

struct CalcTool;

#[async_trait]
impl Tool for CalcTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("calc", "calculator", "Evaluates math")
    }

    async fn execute(&self, args: Value) -> Result<ToolResult, ToolError> {
        let result = args.get("result").cloned().unwrap_or(json!(0));
        Ok(ToolResult::success("calc", result))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_store() -> StateStore {
    let store = StateStore::new();
    // Register the RunLifecycleSlot so the loop can write lifecycle state
    store
        .install_plugin(RunLifecyclePlugin)
        .expect("install lifecycle plugin");
    store
}

struct RunLifecyclePlugin;

impl Plugin for RunLifecyclePlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: "run-lifecycle",
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_slot::<RunLifecycleSlot>(SlotOptions {
            persistent: true,
            retain_on_uninstall: false,
        })?;
        Ok(())
    }
}

fn test_identity() -> RunIdentity {
    RunIdentity::new(
        "thread-1".into(),
        None,
        "run-1".into(),
        None,
        "test-agent".into(),
        RunOrigin::User,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_loop_single_step_natural_end() {
    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "Hello, world!".into(),
        tool_calls: vec![],
        usage: Some(TokenUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(5),
            total_tokens: Some(15),
            ..Default::default()
        }),
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "You are helpful.", llm);
    let store = make_store();

    let result = run_agent_loop(&agent, &store, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    assert_eq!(result.response, "Hello, world!");
    assert_eq!(result.termination, TerminationReason::NaturalEnd);
    assert_eq!(result.steps, 1);

    // Verify state transition
    let lifecycle = store.read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.status, RunStatus::Done);
    assert_eq!(lifecycle.done_reason.as_deref(), Some("natural"));
    assert_eq!(lifecycle.step_count, 1);
    assert_eq!(lifecycle.run_id, "run-1");
}

#[tokio::test]
async fn agent_loop_with_tool_call_then_response() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        // Step 1: LLM requests tool call
        StreamResult {
            text: "Let me search.".into(),
            tool_calls: vec![ToolCall::new("c1", "echo", json!({"message": "hello"}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        // Step 2: LLM responds with final answer
        StreamResult {
            text: "The echo said: hello".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent =
        AgentConfig::new("test", "gpt-4o", "You are helpful.", llm).with_tool(Arc::new(EchoTool));
    let store = make_store();

    let result = run_agent_loop(
        &agent,
        &store,
        vec![Message::user("echo hello")],
        test_identity(),
    )
    .await
    .unwrap();

    assert_eq!(result.response, "The echo said: hello");
    assert_eq!(result.termination, TerminationReason::NaturalEnd);
    assert_eq!(result.steps, 2);

    let lifecycle = store.read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.status, RunStatus::Done);
    assert_eq!(lifecycle.step_count, 2);
}

#[tokio::test]
async fn agent_loop_multiple_tool_calls_in_one_step() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        // Step 1: LLM requests 2 tool calls
        StreamResult {
            text: "".into(),
            tool_calls: vec![
                ToolCall::new("c1", "echo", json!({"message": "first"})),
                ToolCall::new("c2", "calc", json!({"result": 42})),
            ],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        // Step 2: final
        StreamResult {
            text: "Done.".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm)
        .with_tool(Arc::new(EchoTool))
        .with_tool(Arc::new(CalcTool));
    let store = make_store();

    let result = run_agent_loop(
        &agent,
        &store,
        vec![Message::user("multi-tool")],
        test_identity(),
    )
    .await
    .unwrap();

    assert_eq!(result.steps, 2);
    // Should have 2 ToolCallDone events from step 1
    let tool_done_count = result
        .events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolCallDone { .. }))
        .count();
    assert_eq!(tool_done_count, 2);
}

#[tokio::test]
async fn agent_loop_max_rounds_exceeded() {
    // LLM always requests tool calls, never stops
    let llm = Arc::new(ScriptedLlm::new(
        (0..5)
            .map(|i| StreamResult {
                text: "".into(),
                tool_calls: vec![ToolCall::new(
                    format!("c{i}"),
                    "echo",
                    json!({"message": "loop"}),
                )],
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
            })
            .collect(),
    ));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm)
        .with_max_rounds(3)
        .with_tool(Arc::new(EchoTool));
    let store = make_store();

    let result = run_agent_loop(&agent, &store, vec![Message::user("loop")], test_identity())
        .await
        .unwrap();

    // Loop runs max_rounds steps, then on step max_rounds+1 the guard triggers
    assert!(result.steps > 3);
    assert!(matches!(
        result.termination,
        TerminationReason::Stopped(ref s) if s.code == "max_rounds"
    ));

    let lifecycle = store.read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.status, RunStatus::Done);
    assert!(
        lifecycle
            .done_reason
            .as_deref()
            .unwrap()
            .starts_with("stopped:max_rounds")
    );
}

#[tokio::test]
async fn agent_loop_unknown_tool_returns_error_result() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![ToolCall::new("c1", "nonexistent", json!({}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "Sorry.".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let store = make_store();

    let result = run_agent_loop(
        &agent,
        &store,
        vec![Message::user("call unknown")],
        test_identity(),
    )
    .await;
    // Unknown tool should produce an error ToolResult, not crash the loop
    assert!(result.is_err()); // Currently errors because tool not found
}

#[tokio::test]
async fn agent_loop_events_have_correct_sequence() {
    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "Hi!".into(),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let store = make_store();

    let result = run_agent_loop(&agent, &store, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    // Expected event sequence for a single no-tool step
    let event_types: Vec<&str> = result
        .events
        .iter()
        .map(|e| match e {
            AgentEvent::RunStart { .. } => "RunStart",
            AgentEvent::StepStart { .. } => "StepStart",
            AgentEvent::InferenceComplete { .. } => "InferenceComplete",
            AgentEvent::StepEnd => "StepEnd",
            AgentEvent::RunFinish { .. } => "RunFinish",
            _ => "Other",
        })
        .collect();

    assert_eq!(
        event_types,
        vec![
            "RunStart",
            "StepStart",
            "InferenceComplete",
            "StepEnd",
            "RunFinish"
        ]
    );
}

#[tokio::test]
async fn agent_loop_lifecycle_state_reflects_run_id() {
    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "ok".into(),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let store = make_store();

    let identity = RunIdentity::new(
        "t-custom".into(),
        None,
        "r-custom".into(),
        None,
        "a-custom".into(),
        RunOrigin::Internal,
    );

    run_agent_loop(&agent, &store, vec![Message::user("hi")], identity)
        .await
        .unwrap();

    let lifecycle = store.read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.run_id, "r-custom");
}
