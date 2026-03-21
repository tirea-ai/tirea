#![allow(missing_docs)]

use async_trait::async_trait;
use awaken::agent::config::AgentConfig;
use awaken::agent::loop_runner::{AgentRunResult, run_agent_loop};
use awaken::agent::state::{
    RunLifecycleSlot, RunLifecycleState, ToolCallStates, ToolCallStatesSlot,
};
use awaken::contract::event::AgentEvent;
use awaken::contract::executor::{InferenceExecutionError, InferenceRequest, LlmExecutor};
use awaken::contract::identity::{RunIdentity, RunOrigin};
use awaken::contract::inference::{StopReason, StreamResult, TokenUsage};
use awaken::contract::lifecycle::{RunStatus, TerminationReason};
use awaken::contract::message::{Message, ToolCall};
use awaken::contract::suspension::ToolCallStatus;
use awaken::contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use awaken::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock LLM
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
// Mock Tools
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

struct FailingTool;

#[async_trait]
impl Tool for FailingTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new("fail", "fail", "Always fails")
    }

    async fn execute(&self, _args: Value) -> Result<ToolResult, ToolError> {
        Err(ToolError::ExecutionFailed("intentional failure".into()))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct LoopStatePlugin;

impl Plugin for LoopStatePlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor { name: "loop-state" }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_slot::<RunLifecycleSlot>(SlotOptions::default())?;
        registrar.register_slot::<ToolCallStatesSlot>(SlotOptions::default())?;
        Ok(())
    }
}

fn make_runtime() -> PhaseRuntime {
    let store = StateStore::new();
    let runtime = PhaseRuntime::new(store).unwrap();
    runtime.install_plugin(LoopStatePlugin).unwrap();
    runtime
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
async fn single_step_natural_end() {
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
    let runtime = make_runtime();

    let result = run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    assert_eq!(result.response, "Hello, world!");
    assert_eq!(result.termination, TerminationReason::NaturalEnd);
    assert_eq!(result.steps, 1);

    // Verify run lifecycle state
    let lifecycle = runtime.store().read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.status, RunStatus::Done);
    assert_eq!(lifecycle.done_reason.as_deref(), Some("natural"));
    assert_eq!(lifecycle.step_count, 1);
    assert_eq!(lifecycle.run_id, "run-1");
}

#[tokio::test]
async fn tool_call_then_response() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "Let me search.".into(),
            tool_calls: vec![ToolCall::new("c1", "echo", json!({"message": "hello"}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "The echo said: hello".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm).with_tool(Arc::new(EchoTool));
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("echo hello")],
        test_identity(),
    )
    .await
    .unwrap();

    assert_eq!(result.response, "The echo said: hello");
    assert_eq!(result.steps, 2);

    let lifecycle = runtime.store().read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.step_count, 2);
}

#[tokio::test]
async fn tool_call_state_machine_transitions() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![ToolCall::new("c1", "echo", json!({"message": "hi"}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "Done.".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm).with_tool(Arc::new(EchoTool));
    let runtime = make_runtime();

    run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("test")],
        test_identity(),
    )
    .await
    .unwrap();

    // After step 1 (with tool call), tool call state should show Succeeded
    // But step 2 clears it, so final state is empty (cleared at step start)
    let tool_states = runtime
        .store()
        .read_slot::<ToolCallStatesSlot>()
        .unwrap_or_default();
    // Step 2 had no tool calls, so after Clear at step 2 start, it's empty
    assert!(tool_states.calls.is_empty());
}

#[tokio::test]
async fn multiple_tool_calls_in_one_step() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![
                ToolCall::new("c1", "echo", json!({"message": "first"})),
                ToolCall::new("c2", "calc", json!({"result": 42})),
            ],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
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
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("multi-tool")],
        test_identity(),
    )
    .await
    .unwrap();

    assert_eq!(result.steps, 2);
    let tool_done_count = result
        .events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ToolCallDone { .. }))
        .count();
    assert_eq!(tool_done_count, 2);
}

#[tokio::test]
async fn max_rounds_exceeded() {
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
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("loop")],
        test_identity(),
    )
    .await
    .unwrap();

    assert!(matches!(
        result.termination,
        TerminationReason::Stopped(ref s) if s.code == "max_rounds"
    ));

    let lifecycle = runtime.store().read_slot::<RunLifecycleSlot>().unwrap();
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
async fn unknown_tool_returns_error_result_not_crash() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![ToolCall::new("c1", "nonexistent", json!({}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "Sorry, that tool doesn't exist.".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("call unknown")],
        test_identity(),
    )
    .await
    .unwrap(); // Should NOT error — unknown tool produces ToolResult::error

    assert_eq!(result.termination, TerminationReason::NaturalEnd);
    assert_eq!(result.steps, 2);

    // The tool call should have Failed status
    // (cleared by step 2, but the event shows it)
    let tool_fail_events: Vec<_> = result
        .events
        .iter()
        .filter(|e| {
            matches!(e, AgentEvent::ToolCallDone { outcome, .. }
                if *outcome == awaken::contract::suspension::ToolCallOutcome::Failed)
        })
        .collect();
    assert_eq!(tool_fail_events.len(), 1);
}

#[tokio::test]
async fn failing_tool_produces_error_result_continues_loop() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![ToolCall::new("c1", "fail", json!({}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "Tool failed, sorry.".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm).with_tool(Arc::new(FailingTool));
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("use fail tool")],
        test_identity(),
    )
    .await
    .unwrap();

    assert_eq!(result.termination, TerminationReason::NaturalEnd);
    assert_eq!(result.steps, 2);
}

#[tokio::test]
async fn events_have_correct_sequence_for_single_step() {
    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "Hi!".into(),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let runtime = make_runtime();

    let result = run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

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
async fn events_have_correct_sequence_with_tool_call() {
    let llm = Arc::new(ScriptedLlm::new(vec![
        StreamResult {
            text: "".into(),
            tool_calls: vec![ToolCall::new("c1", "echo", json!({"message": "x"}))],
            usage: None,
            stop_reason: Some(StopReason::ToolUse),
        },
        StreamResult {
            text: "Done".into(),
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::EndTurn),
        },
    ]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm).with_tool(Arc::new(EchoTool));
    let runtime = make_runtime();

    let result = run_agent_loop(
        &agent,
        &runtime,
        vec![Message::user("echo")],
        test_identity(),
    )
    .await
    .unwrap();

    let event_types: Vec<&str> = result
        .events
        .iter()
        .map(|e| match e {
            AgentEvent::RunStart { .. } => "RunStart",
            AgentEvent::StepStart { .. } => "StepStart",
            AgentEvent::InferenceComplete { .. } => "InferenceComplete",
            AgentEvent::ToolCallStart { .. } => "ToolCallStart",
            AgentEvent::ToolCallDone { .. } => "ToolCallDone",
            AgentEvent::StepEnd => "StepEnd",
            AgentEvent::RunFinish { .. } => "RunFinish",
            _ => "Other",
        })
        .collect();

    assert_eq!(
        event_types,
        vec![
            "RunStart",
            // Step 1: tool call
            "StepStart",
            "InferenceComplete",
            "ToolCallStart",
            "ToolCallDone",
            "StepEnd",
            // Step 2: final response
            "StepStart",
            "InferenceComplete",
            "StepEnd",
            "RunFinish",
        ]
    );
}

#[tokio::test]
async fn lifecycle_state_reflects_custom_run_id() {
    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "ok".into(),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let runtime = make_runtime();

    let identity = RunIdentity::new(
        "t-custom".into(),
        None,
        "r-custom".into(),
        None,
        "a-custom".into(),
        RunOrigin::Internal,
    );

    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], identity)
        .await
        .unwrap();

    let lifecycle = runtime.store().read_slot::<RunLifecycleSlot>().unwrap();
    assert_eq!(lifecycle.run_id, "r-custom");
}

#[tokio::test]
async fn phase_hooks_fire_during_loop() {
    let hook_phases = Arc::new(Mutex::new(Vec::<Phase>::new()));

    struct PhaseTracker(Arc<Mutex<Vec<Phase>>>);
    #[async_trait]
    impl PhaseHook for PhaseTracker {
        async fn run(&self, ctx: &PhaseContext) -> Result<StateCommand, StateError> {
            self.0.lock().unwrap().push(ctx.phase);
            Ok(StateCommand::new())
        }
    }

    struct TrackerPlugin(Arc<Mutex<Vec<Phase>>>);
    impl Plugin for TrackerPlugin {
        fn descriptor(&self) -> PluginDescriptor {
            PluginDescriptor { name: "tracker" }
        }
        fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
            for phase in Phase::ALL {
                registrar.register_phase_hook(
                    "tracker",
                    phase,
                    PhaseTracker(Arc::clone(&self.0)),
                )?;
            }
            Ok(())
        }
    }

    let llm = Arc::new(ScriptedLlm::new(vec![StreamResult {
        text: "done".into(),
        tool_calls: vec![],
        usage: None,
        stop_reason: Some(StopReason::EndTurn),
    }]));

    let agent = AgentConfig::new("test", "gpt-4o", "helpful", llm);
    let runtime = make_runtime();
    runtime
        .install_plugin(TrackerPlugin(Arc::clone(&hook_phases)))
        .unwrap();

    run_agent_loop(&agent, &runtime, vec![Message::user("hi")], test_identity())
        .await
        .unwrap();

    let phases = hook_phases.lock().unwrap();
    assert_eq!(
        *phases,
        vec![
            Phase::RunStart,
            Phase::StepStart,
            Phase::BeforeInference,
            Phase::AfterInference,
            Phase::StepEnd,
            Phase::RunEnd,
        ]
    );
}
