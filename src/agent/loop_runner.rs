//! Minimal sequential agent loop.
//!
//! Implements: RunStart → [StepStart → BeforeInference → LLM → AfterInference
//! → (for each tool call) BeforeToolExecute → execute → AfterToolExecute
//! → StepEnd] × N → RunEnd
//!
//! State transitions tracked via `RunLifecycleSlot`.

use std::sync::Arc;

use crate::contract::event::AgentEvent;
use crate::contract::executor::InferenceRequest;
use crate::contract::identity::RunIdentity;
use crate::contract::lifecycle::TerminationReason;
use crate::contract::message::{Message, ToolCall, gen_message_id};
use crate::contract::tool::ToolResult;
use crate::model::Phase;
use crate::state::{MutationBatch, StateStore};

use super::config::AgentConfig;
use super::state::{RunLifecycleSlot, RunLifecycleUpdate};

/// Errors from the agent loop.
#[derive(Debug, thiserror::Error)]
pub enum AgentLoopError {
    #[error("inference failed: {0}")]
    InferenceFailed(String),
    #[error("tool execution failed: {tool_name}: {message}")]
    ToolExecutionFailed { tool_name: String, message: String },
    #[error("state error: {0}")]
    StateError(#[from] crate::error::StateError),
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
pub async fn run_agent_loop(
    agent: &AgentConfig,
    store: &StateStore,
    initial_messages: Vec<Message>,
    run_identity: RunIdentity,
) -> Result<AgentRunResult, AgentLoopError> {
    let mut events = Vec::new();
    let mut messages: Vec<Arc<Message>> = initial_messages.into_iter().map(Arc::new).collect();
    let mut steps: usize = 0;

    // --- State transition: Start ---
    commit_lifecycle(
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

    // RunStart phase hooks (placeholder — will be wired through PhaseRuntime)
    run_phase_hooks(store, Phase::RunStart)?;

    let termination = loop {
        steps += 1;
        if steps > agent.max_rounds {
            break TerminationReason::stopped_with_detail(
                "max_rounds",
                format!("exceeded {} rounds", agent.max_rounds),
            );
        }

        let step_message_id = gen_message_id();
        events.push(AgentEvent::StepStart {
            message_id: step_message_id,
        });

        run_phase_hooks(store, Phase::StepStart)?;
        run_phase_hooks(store, Phase::BeforeInference)?;

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

        // AfterInference phase
        run_phase_hooks(store, Phase::AfterInference)?;

        if !stream_result.needs_tools() {
            messages.push(Arc::new(Message::assistant(&stream_result.text)));
            // --- State transition: StepCompleted ---
            commit_lifecycle(
                store,
                RunLifecycleUpdate::StepCompleted {
                    updated_at: now_ms(),
                },
            )?;
            run_phase_hooks(store, Phase::StepEnd)?;
            events.push(AgentEvent::StepEnd);
            break TerminationReason::NaturalEnd;
        }

        // Add assistant message with tool calls
        messages.push(Arc::new(Message::assistant_with_tool_calls(
            &stream_result.text,
            stream_result.tool_calls.clone(),
        )));

        // Execute each tool call sequentially
        for call in &stream_result.tool_calls {
            events.push(AgentEvent::ToolCallStart {
                id: call.id.clone(),
                name: call.name.clone(),
            });

            run_phase_hooks(store, Phase::BeforeToolExecute)?;

            let tool_result = execute_tool(agent, call).await?;

            events.push(AgentEvent::ToolCallDone {
                id: call.id.clone(),
                result: tool_result.clone(),
                outcome: crate::contract::suspension::ToolCallOutcome::from_tool_result(
                    &tool_result,
                ),
            });

            run_phase_hooks(store, Phase::AfterToolExecute)?;

            let tool_content = tool_result_to_content(&tool_result);
            messages.push(Arc::new(Message::tool(&call.id, tool_content)));
        }

        // --- State transition: StepCompleted ---
        commit_lifecycle(
            store,
            RunLifecycleUpdate::StepCompleted {
                updated_at: now_ms(),
            },
        )?;
        run_phase_hooks(store, Phase::StepEnd)?;
        events.push(AgentEvent::StepEnd);
    };

    // --- State transition: Done ---
    let (_, done_reason) = termination.to_run_status();
    commit_lifecycle(
        store,
        RunLifecycleUpdate::Done {
            done_reason: done_reason.unwrap_or_else(|| "unknown".into()),
            updated_at: now_ms(),
        },
    )?;

    run_phase_hooks(store, Phase::RunEnd)?;

    let response = messages
        .iter()
        .rev()
        .find(|m| m.role == crate::contract::message::Role::Assistant)
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

fn commit_lifecycle(
    store: &StateStore,
    update: RunLifecycleUpdate,
) -> Result<(), crate::error::StateError> {
    let mut patch = MutationBatch::new();
    patch.update::<RunLifecycleSlot>(update);
    store.commit(patch)?;
    Ok(())
}

fn run_phase_hooks(_store: &StateStore, _phase: Phase) -> Result<(), AgentLoopError> {
    // Phase hooks will be wired through PhaseRuntime in a follow-up commit.
    Ok(())
}

async fn execute_tool(agent: &AgentConfig, call: &ToolCall) -> Result<ToolResult, AgentLoopError> {
    let tool = agent
        .tools
        .get(&call.name)
        .ok_or_else(|| AgentLoopError::ToolExecutionFailed {
            tool_name: call.name.clone(),
            message: format!("tool '{}' not found", call.name),
        })?;

    if let Err(e) = tool.validate_args(&call.arguments) {
        return Ok(ToolResult::error(&call.name, e.to_string()));
    }

    match tool.execute(call.arguments.clone()).await {
        Ok(result) => Ok(result),
        Err(e) => Ok(ToolResult::error(&call.name, e.to_string())),
    }
}

fn tool_result_to_content(result: &ToolResult) -> String {
    match &result.message {
        Some(msg) => msg.clone(),
        None => serde_json::to_string(&result.data).unwrap_or_default(),
    }
}
