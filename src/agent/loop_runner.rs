//! Minimal sequential agent loop driven by state machines.
//!
//! Run lifecycle: RunLifecycleSlot (Running → StepCompleted → Done/Waiting)
//! Tool call lifecycle: ToolCallStatesSlot (New → Running → Succeeded/Failed/Suspended)

use std::sync::Arc;

use crate::contract::event::AgentEvent;
use crate::contract::executor::InferenceRequest;
use crate::contract::identity::RunIdentity;
use crate::contract::lifecycle::TerminationReason;
use crate::contract::message::{Message, Role, ToolCall, gen_message_id};
use crate::contract::suspension::{ToolCallOutcome, ToolCallStatus};
use crate::contract::tool::ToolResult;
use crate::model::Phase;
use crate::runtime::PhaseRuntime;
use crate::state::MutationBatch;

use super::config::AgentConfig;
use super::state::{
    RunLifecycleSlot, RunLifecycleUpdate, ToolCallStatesSlot, ToolCallStatesUpdate,
};

/// Errors from the agent loop.
#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("phase error: {0}")]
    PhaseError(#[from] crate::error::StateError),
}

/// Result of running the agent loop.
#[derive(Debug)]
pub struct AgentRunResult {
    pub response: String,
    pub termination: TerminationReason,
    pub events: Vec<AgentEvent>,
    pub steps: usize,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Run the agent loop to completion.
///
/// State-machine driven: every lifecycle transition is committed via StateStore.
/// Phase hooks are dispatched through PhaseRuntime.
pub async fn run_agent_loop(
    agent: &AgentConfig,
    runtime: &PhaseRuntime,
    initial_messages: Vec<Message>,
    run_identity: RunIdentity,
) -> Result<AgentRunResult, AgentLoopError> {
    let store = runtime.store();
    let mut events = Vec::new();
    let mut messages: Vec<Arc<Message>> = initial_messages.into_iter().map(Arc::new).collect();
    let mut steps: usize = 0;

    // --- Run lifecycle: Start ---
    commit_update::<RunLifecycleSlot>(
        store,
        RunLifecycleUpdate::Start {
            run_id: run_identity.run_id.clone(),
            updated_at: now_ms(),
        },
    )?;

    events.push(AgentEvent::RunStart {
        thread_id: run_identity.thread_id.clone(),
        run_id: run_identity.run_id.clone(),
        parent_run_id: run_identity.parent_run_id.clone(),
    });

    runtime.run_phase(Phase::RunStart)?;

    let termination = loop {
        steps += 1;
        if steps > agent.max_rounds {
            break TerminationReason::stopped_with_detail(
                "max_rounds",
                format!("exceeded {} rounds", agent.max_rounds),
            );
        }

        events.push(AgentEvent::StepStart {
            message_id: gen_message_id(),
        });

        // Clear tool call states from previous step
        commit_update::<ToolCallStatesSlot>(store, ToolCallStatesUpdate::Clear)?;

        runtime.run_phase(Phase::StepStart)?;
        runtime.run_phase(Phase::BeforeInference)?;

        // LLM inference
        let start = std::time::Instant::now();
        let request = InferenceRequest {
            model: agent.model.clone(),
            messages: messages.iter().map(|m| (**m).clone()).collect(),
            tools: agent.tool_descriptors(),
            system_prompt: Some(agent.system_prompt.clone()),
            overrides: None,
        };

        let stream_result = agent
            .llm_executor
            .execute(request)
            .await
            .map_err(|e| AgentLoopError::InferenceFailed(e.to_string()))?;

        let duration_ms = start.elapsed().as_millis() as u64;
        events.push(AgentEvent::InferenceComplete {
            model: agent.model.clone(),
            usage: stream_result.usage.clone(),
            duration_ms,
        });

        runtime.run_phase(Phase::AfterInference)?;

        if !stream_result.needs_tools() {
            messages.push(Arc::new(Message::assistant(&stream_result.text)));
            complete_step(store, runtime, &mut events)?;
            break TerminationReason::NaturalEnd;
        }

        // Add assistant message with tool calls
        messages.push(Arc::new(Message::assistant_with_tool_calls(
            &stream_result.text,
            stream_result.tool_calls.clone(),
        )));

        // Execute each tool call sequentially with state machine transitions
        for call in &stream_result.tool_calls {
            events.push(AgentEvent::ToolCallStart {
                id: call.id.clone(),
                name: call.name.clone(),
            });

            // Tool call lifecycle: New → Running
            commit_tool_call_transition(store, call, ToolCallStatus::Running)?;

            runtime.run_phase(Phase::BeforeToolExecute)?;

            let tool_result = execute_tool(agent, call).await;

            // Tool call lifecycle: Running → Succeeded/Failed
            let terminal_status = if tool_result.is_success() {
                ToolCallStatus::Succeeded
            } else {
                ToolCallStatus::Failed
            };
            commit_tool_call_transition(store, call, terminal_status)?;

            events.push(AgentEvent::ToolCallDone {
                id: call.id.clone(),
                result: tool_result.clone(),
                outcome: ToolCallOutcome::from_tool_result(&tool_result),
            });

            runtime.run_phase(Phase::AfterToolExecute)?;

            let tool_content = tool_result_to_content(&tool_result);
            messages.push(Arc::new(Message::tool(&call.id, tool_content)));
        }

        complete_step(store, runtime, &mut events)?;
    };

    // --- Run lifecycle: Done ---
    let (_, done_reason) = termination.to_run_status();
    commit_update::<RunLifecycleSlot>(
        store,
        RunLifecycleUpdate::Done {
            done_reason: done_reason.unwrap_or_else(|| "unknown".into()),
            updated_at: now_ms(),
        },
    )?;

    runtime.run_phase(Phase::RunEnd)?;

    let response = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.content.clone())
        .unwrap_or_default();

    events.push(AgentEvent::RunFinish {
        thread_id: run_identity.thread_id.clone(),
        run_id: run_identity.run_id.clone(),
        result: Some(serde_json::json!({"response": response})),
        termination: termination.clone(),
    });

    Ok(AgentRunResult {
        response,
        termination,
        events,
        steps,
    })
}

// -- Helpers --

fn complete_step(
    store: &crate::state::StateStore,
    runtime: &PhaseRuntime,
    events: &mut Vec<AgentEvent>,
) -> Result<(), AgentLoopError> {
    commit_update::<RunLifecycleSlot>(
        store,
        RunLifecycleUpdate::StepCompleted {
            updated_at: now_ms(),
        },
    )?;
    runtime.run_phase(Phase::StepEnd)?;
    events.push(AgentEvent::StepEnd);
    Ok(())
}

fn commit_update<S: crate::state::StateSlot>(
    store: &crate::state::StateStore,
    update: S::Update,
) -> Result<(), crate::error::StateError> {
    let mut patch = MutationBatch::new();
    patch.update::<S>(update);
    store.commit(patch)?;
    Ok(())
}

fn commit_tool_call_transition(
    store: &crate::state::StateStore,
    call: &ToolCall,
    status: ToolCallStatus,
) -> Result<(), crate::error::StateError> {
    commit_update::<ToolCallStatesSlot>(
        store,
        ToolCallStatesUpdate::Upsert {
            call_id: call.id.clone(),
            tool_name: call.name.clone(),
            arguments: call.arguments.clone(),
            status,
            updated_at: now_ms(),
        },
    )
}

/// Execute a tool, returning ToolResult (never crashes the loop).
async fn execute_tool(agent: &AgentConfig, call: &ToolCall) -> ToolResult {
    let Some(tool) = agent.tools.get(&call.name) else {
        return ToolResult::error(&call.name, format!("tool '{}' not found", call.name));
    };

    if let Err(e) = tool.validate_args(&call.arguments) {
        return ToolResult::error(&call.name, e.to_string());
    }

    match tool.execute(call.arguments.clone()).await {
        Ok(result) => result,
        Err(e) => ToolResult::error(&call.name, e.to_string()),
    }
}

fn tool_result_to_content(result: &ToolResult) -> String {
    match &result.message {
        Some(msg) => msg.clone(),
        None => serde_json::to_string(&result.data).unwrap_or_default(),
    }
}
