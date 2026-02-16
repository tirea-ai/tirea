//! Agent loop implementation with Phase-based plugin execution.
//!
//! The agent loop orchestrates the conversation between user, LLM, and tools:
//!
//! ```text
//! User Input → LLM → Tool Calls? → Execute Tools → LLM → ... → Final Response
//! ```
//!
//! # Phase Execution
//!
//! Each phase emits events to all plugins via `on_phase()`:
//!
//! ```text
//! SessionStart (once)
//!     │
//!     ▼
//! ┌─────────────────────────┐
//! │      StepStart          │ ← plugins can inject system context
//! ├─────────────────────────┤
//! │    BeforeInference      │ ← plugins can filter tools, add session context
//! ├─────────────────────────┤
//! │      [LLM CALL]         │
//! ├─────────────────────────┤
//! │    AfterInference       │
//! ├─────────────────────────┤
//! │  ┌───────────────────┐  │
//! │  │ BeforeToolExecute │  │ ← plugins can block/pending
//! │  ├───────────────────┤  │
//! │  │   [TOOL EXEC]     │  │
//! │  ├───────────────────┤  │
//! │  │ AfterToolExecute  │  │ ← plugins can add reminders
//! │  └───────────────────┘  │
//! ├─────────────────────────┤
//! │       StepEnd           │
//! └─────────────────────────┘
//!     │
//!     ▼
//! SessionEnd (once)
//! ```

mod core;
mod loop_state;
mod outcome;
mod plugin_runtime;
mod state_commit;
mod tool_exec;

use crate::contracts::events::{AgentEvent, StreamResult};
use crate::contracts::phase::Phase;
use crate::contracts::state_types::{Interaction, AGENT_STATE_PATH};
use crate::contracts::traits::tool::Tool;
use crate::engine::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::engine::stop_conditions::{check_stop_conditions, StopReason};
use crate::runtime::activity::ActivityHub;
use crate::runtime::streaming::StreamCollector;
use crate::thread::Thread;
use crate::thread_store::CheckpointReason;
#[cfg(test)]
use crate::types::MessageMetadata;
use crate::types::{gen_message_id, Message};
use async_stream::stream;
use async_trait::async_trait;
use carve_state::ActivityManager;
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

#[cfg(test)]
use crate::contracts::agent_plugin::AgentPlugin;
#[cfg(test)]
use crate::contracts::phase::StepContext;
pub use carve_agent_contract::agent::{
    AgentConfig, AgentDefinition, RunCancellationToken, RunContext, StateCommitError,
    StateCommitter,
};
pub(crate) use carve_agent_contract::agent::{
    TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
#[cfg(test)]
use carve_state::TrackedPatch;
#[cfg(test)]
use core::build_messages;
#[cfg(test)]
use core::set_agent_pending_interaction;
use core::{
    apply_pending_patches, build_request_for_filtered_tools, clear_agent_pending_interaction,
    drain_agent_outbox, inference_inputs_from_step, reduce_thread_mutations,
    tool_descriptors_for_config, ThreadMutationBatch,
};
use loop_state::{effective_stop_conditions, LoopState};
pub use outcome::{run_round, tool_map, tool_map_from_arc, AgentLoopError, RoundResult};
#[cfg(test)]
use plugin_runtime::emit_phase_checked;
use plugin_runtime::{
    emit_cleanup_phases_and_apply, emit_phase_block, emit_session_end,
    prepare_stream_error_termination, run_phase_block,
};
use state_commit::commit_pending_delta;
pub use state_commit::ChannelStateCommitter;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
use tool_exec::{
    apply_tool_results_impl, apply_tool_results_to_session, execute_single_tool_with_phases,
    execute_tool_calls_with_phases, next_step_index, runtime_with_tool_caller_context,
    step_metadata, ToolExecutionResult,
};
pub use tool_exec::{execute_tools, execute_tools_with_config, execute_tools_with_plugins};
#[cfg(test)]
use tool_exec::{execute_tools_parallel_with_phases, execute_tools_sequential_with_phases};

fn uuid_v7() -> String {
    Uuid::now_v7().simple().to_string()
}

#[async_trait]
trait ChatStreamProvider: Send + Sync {
    async fn exec_chat_stream_events(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<
        Pin<Box<dyn Stream<Item = genai::Result<genai::chat::ChatStreamEvent>> + Send>>,
    >;
}

#[async_trait]
impl ChatStreamProvider for Client {
    async fn exec_chat_stream_events(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<
        Pin<Box<dyn Stream<Item = genai::Result<genai::chat::ChatStreamEvent>> + Send>>,
    > {
        let resp = self.exec_chat_stream(model, chat_req, options).await?;
        Ok(Box::pin(resp.stream))
    }
}

/// Run one step of the agent loop (non-streaming).
///
/// A step consists of:
/// 1. Emit StepStart phase
/// 2. Emit BeforeInference phase
/// 3. Send messages to LLM
/// 4. Emit AfterInference phase
/// 5. Emit StepEnd phase
/// 6. Return the session and result (caller handles tool execution)
pub async fn run_step(
    client: &Client,
    config: &AgentConfig,
    thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Thread, StreamResult), AgentLoopError> {
    let tool_descriptors = tool_descriptors_for_config(tools, config);

    let phase_block = run_phase_block(
        &thread,
        &tool_descriptors,
        &config.plugins,
        &[Phase::StepStart, Phase::BeforeInference],
        |_| {},
        |step| inference_inputs_from_step(step, &config.system_prompt),
    )
    .await?;
    let ((messages, filtered_tools, skip_inference, tracing_span), pending) = phase_block;
    let thread = apply_pending_patches(thread, pending);

    // Skip inference if requested
    if skip_inference {
        return Ok((
            thread,
            StreamResult {
                text: String::new(),
                tool_calls: vec![],
                usage: None,
            },
        ));
    }

    // Build request with filtered tools
    let request = build_request_for_filtered_tools(&messages, tools, &filtered_tools);

    // Call LLM (instrumented with tracing span for context propagation)
    let inference_span = tracing_span.unwrap_or_else(tracing::Span::none);
    let response_res = async {
        client
            .exec_chat(&config.model, request, config.chat_options.as_ref())
            .await
    }
    .instrument(inference_span)
    .await;
    let response = match response_res {
        Ok(r) => r,
        Err(e) => {
            // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
            let _session = emit_cleanup_phases_and_apply(
                thread,
                &tool_descriptors,
                &config.plugins,
                "llm_exec_error",
                e.to_string(),
            )
            .await?;
            return Err(AgentLoopError::LlmError(e.to_string()));
        }
    };

    // Extract text and tool calls from response
    let text = response
        .first_text()
        .map(|s| s.to_string())
        .unwrap_or_default();

    let tool_calls: Vec<crate::types::ToolCall> = response
        .tool_calls()
        .into_iter()
        .map(|tc| crate::types::ToolCall::new(&tc.call_id, &tc.fn_name, tc.fn_arguments.clone()))
        .collect();

    let usage = Some(response.usage.clone());
    let result = StreamResult {
        text,
        tool_calls,
        usage,
    };

    // Phase: AfterInference (with new context)
    let pending = emit_phase_block(
        Phase::AfterInference,
        &thread,
        &tool_descriptors,
        &config.plugins,
        |step| {
            step.response = Some(result.clone());
        },
    )
    .await?;
    let thread = apply_pending_patches(thread, pending);

    // Add assistant message
    let step_meta = step_metadata(
        thread
            .runtime
            .value("run_id")
            .and_then(|v| v.as_str().map(String::from)),
        next_step_index(&thread),
    );
    let thread = if result.tool_calls.is_empty() {
        thread.with_message(assistant_message(&result.text).with_metadata(step_meta))
    } else {
        thread.with_message(
            assistant_tool_calls(&result.text, result.tool_calls.clone()).with_metadata(step_meta),
        )
    };

    // Phase: StepEnd (with new context)
    let pending = emit_phase_block(
        Phase::StepEnd,
        &thread,
        &tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await?;
    let thread = apply_pending_patches(thread, pending);

    Ok((thread, result))
}

/// Run the full agent loop until completion or a stop condition is met.
///
/// Returns the final session and the last response text.
pub async fn run_loop(
    client: &Client,
    config: &AgentConfig,
    mut thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Thread, String), AgentLoopError> {
    let mut loop_state = LoopState::new();
    let stop_conditions = effective_stop_conditions(config);
    let mut last_text = String::new();
    let run_id = thread
        .runtime
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            let id = uuid_v7();
            let _ = thread.runtime.set("run_id", &id);
            id
        });

    let tool_descriptors = tool_descriptors_for_config(tools, config);

    // Phase: SessionStart
    let pending = emit_phase_block(
        Phase::SessionStart,
        &thread,
        &tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await?;
    thread = apply_pending_patches(thread, pending);

    loop {
        // Phase: StepStart and BeforeInference
        let step_prepare = run_phase_block(
            &thread,
            &tool_descriptors,
            &config.plugins,
            &[Phase::StepStart, Phase::BeforeInference],
            |_| {},
            |step| inference_inputs_from_step(step, &config.system_prompt),
        )
        .await;
        let ((messages, filtered_tools, skip_inference, tracing_span), pending) = match step_prepare
        {
            Ok(v) => v,
            Err(e) => {
                let _finalized = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        };
        thread = apply_pending_patches(thread, pending);

        if skip_inference {
            break;
        }

        // Build request with filtered tools
        let request = build_request_for_filtered_tools(&messages, tools, &filtered_tools);

        // Call LLM (instrumented with tracing span)
        let inference_span = tracing_span.unwrap_or_else(tracing::Span::none);
        let response_res = async {
            client
                .exec_chat(&config.model, request, config.chat_options.as_ref())
                .await
        }
        .instrument(inference_span)
        .await;
        let response = match response_res {
            Ok(r) => r,
            Err(e) => {
                // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                thread = match emit_cleanup_phases_and_apply(
                    thread.clone(),
                    &tool_descriptors,
                    &config.plugins,
                    "llm_exec_error",
                    e.to_string(),
                )
                .await
                {
                    Ok(s) => s,
                    Err(phase_error) => {
                        let _finalized =
                            emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                        return Err(phase_error);
                    }
                };
                let _finalized = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::LlmError(e.to_string()));
            }
        };

        // Extract text and tool calls from response
        let text = response
            .first_text()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let tool_calls: Vec<crate::types::ToolCall> = response
            .tool_calls()
            .into_iter()
            .map(|tc| {
                crate::types::ToolCall::new(&tc.call_id, &tc.fn_name, tc.fn_arguments.clone())
            })
            .collect();

        let usage = Some(response.usage.clone());
        let result = StreamResult {
            text,
            tool_calls,
            usage,
        };
        loop_state.update_from_response(&result);
        last_text = result.text.clone();

        // Phase: AfterInference
        match emit_phase_block(
            Phase::AfterInference,
            &thread,
            &tool_descriptors,
            &config.plugins,
            |step| {
                step.response = Some(result.clone());
            },
        )
        .await
        {
            Ok(pending) => {
                thread = apply_pending_patches(thread, pending);
            }
            Err(e) => {
                let _finalized = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        }

        // Add assistant message
        let assistant_msg_id = gen_message_id();
        let step_meta = step_metadata(Some(run_id.clone()), loop_state.rounds as u32);
        let assistant_msg = if result.tool_calls.is_empty() {
            assistant_message(&result.text)
                .with_id(assistant_msg_id.clone())
                .with_metadata(step_meta.clone())
        } else {
            assistant_tool_calls(&result.text, result.tool_calls.clone())
                .with_id(assistant_msg_id.clone())
                .with_metadata(step_meta.clone())
        };
        thread = reduce_thread_mutations(
            thread,
            ThreadMutationBatch::default().with_message(assistant_msg),
        );

        // Phase: StepEnd
        match emit_phase_block(
            Phase::StepEnd,
            &thread,
            &tool_descriptors,
            &config.plugins,
            |_| {},
        )
        .await
        {
            Ok(pending) => {
                thread = apply_pending_patches(thread, pending);
            }
            Err(e) => {
                let _finalized = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        }

        if !result.needs_tools() {
            break;
        }

        // Execute tools with phase hooks (respect config.parallel_tools).
        let state = match thread.rebuild_state() {
            Ok(s) => s,
            Err(e) => {
                emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::StateError(e.to_string()));
            }
        };
        let rt_for_tools = match runtime_with_tool_caller_context(&thread, &state, Some(config)) {
            Ok(rt) => rt,
            Err(e) => {
                emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        };

        let results = execute_tool_calls_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            &config.plugins,
            config.parallel_tools,
            None,
            Some(&rt_for_tools),
            &thread.id,
        )
        .await;

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                let _finalized = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        };

        let session_before_apply = thread.clone();
        let applied = match apply_tool_results_to_session(
            thread,
            &results,
            Some(step_meta),
            config.parallel_tools,
        ) {
            Ok(a) => a,
            Err(e) => {
                let _finalized =
                    emit_session_end(session_before_apply, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        };
        thread = applied.thread;

        // Pause if any tool is waiting for client response.
        if let Some(interaction) = applied.pending_interaction {
            thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
            return Err(AgentLoopError::PendingInteraction {
                thread: Box::new(thread),
                interaction: Box::new(interaction),
            });
        }

        // Track tool round metrics for stop condition evaluation.
        let error_count = results
            .iter()
            .filter(|r| r.execution.result.is_error())
            .count();
        loop_state.record_tool_round(&result.tool_calls, error_count);
        loop_state.rounds += 1;

        // Check stop conditions.
        let stop_ctx = loop_state.to_check_context(&result, &thread);
        if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
            thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
            return Err(AgentLoopError::Stopped {
                thread: Box::new(thread),
                reason,
            });
        }
    }

    // Phase: SessionEnd
    thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;

    Ok((thread, last_text))
}

/// Run the agent loop with streaming output.
///
/// Returns a stream of AgentEvent for real-time updates.
pub fn run_loop_stream(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    run_loop_stream_impl_with_provider(Arc::new(client), config, thread, tools, run_ctx)
}

fn run_loop_stream_impl_with_provider(
    provider: Arc<dyn ChatStreamProvider>,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut thread = thread;
    let mut loop_state = LoopState::new();
    let stop_conditions = effective_stop_conditions(&config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
    let state_committer = run_ctx.state_committer().cloned();
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

    let tool_descriptors = tool_descriptors_for_config(&tools, &config);

        // Resolve run_id: from runtime if pre-set, otherwise generate.
        // NOTE: runtime is mutated in-place here. This is intentional —
        // runtime is transient (not persisted) and the owned-builder pattern
        // (`with_runtime`) is impractical inside the loop where `session` is
        // borrowed across yield points.
        let run_id = thread.runtime.value("run_id")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| {
                let id = uuid_v7();
                // Best-effort: set into runtime (may already be set).
                let _ = thread.runtime.set("run_id", &id);
                id
            });
        let parent_run_id = thread.runtime.value("parent_run_id")
            .and_then(|v| v.as_str().map(String::from));

        macro_rules! emit_run_finished_delta {
            () => {
                if let Err(e) = commit_pending_delta(
                    &mut thread,
                    CheckpointReason::RunFinished,
                    true,
                    &run_id,
                    parent_run_id.as_deref(),
                    state_committer.as_ref(),
                )
                .await
                {
                    tracing::warn!(error = %e, "failed to commit run-finished delta");
                }
            };
        }

        macro_rules! ensure_run_finished_delta_or_error {
            () => {
                if let Err(e) = commit_pending_delta(
                    &mut thread,
                    CheckpointReason::RunFinished,
                    true,
                    &run_id,
                    parent_run_id.as_deref(),
                    state_committer.as_ref(),
                )
                .await
                {
                    yield AgentEvent::Error {
                        message: e.to_string(),
                    };
                    return;
                }
            };
        }

        macro_rules! terminate_stream_error {
            ($message:expr) => {{
                let (finalized, error, finish) = prepare_stream_error_termination(
                    thread,
                    &tool_descriptors,
                    &config.plugins,
                    &run_id,
                    $message,
                )
                .await;
                thread = finalized;
                emit_run_finished_delta!();
                yield error;
                yield finish;
                return;
            }};
        }

        // Phase: SessionStart (use scoped block to manage borrow)
        match emit_phase_block(
            Phase::SessionStart,
            &thread,
            &tool_descriptors,
            &config.plugins,
            |_| {},
        )
        .await
        {
            Ok(pending) => {
                thread = apply_pending_patches(thread, pending);
            }
            Err(e) => {
                terminate_stream_error!(e.to_string());
            }
        }

        yield AgentEvent::RunStart {
            thread_id: thread.id.clone(),
            run_id: run_id.clone(),
            parent_run_id: parent_run_id.clone(),
        };

        let (next_thread, outbox) =
            match drain_agent_outbox(thread.clone(), "agent_outbox_session_start")
        {
            Ok(v) => v,
            Err(e) => {
                terminate_stream_error!(e.to_string());
            }
        };
        thread = next_thread;
        for resolution in outbox.interaction_resolutions {
            yield AgentEvent::InteractionResolved {
                interaction_id: resolution.interaction_id,
                result: resolution.result,
            };
        }

        // Resume pending tool execution via plugin mechanism.
        // Plugins request replay by writing AgentState.replay_tool_calls.
        // The loop executes those calls at session start.
        let replay_calls = outbox.replay_tool_calls;
        if !replay_calls.is_empty() {
            let mut replay_state_changed = false;
            for tool_call in &replay_calls {
                let state = match thread.rebuild_state() {
                    Ok(s) => s,
                    Err(e) => {
                        terminate_stream_error!(format!(
                            "failed to rebuild state before replaying tool '{}': {e}",
                            tool_call.id
                        ));
                    }
                };

                let tool = tools.get(&tool_call.name).cloned();
                let rt_for_replay =
                    match runtime_with_tool_caller_context(&thread, &state, Some(&config)) {
                    Ok(rt) => rt,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
                let replay_result = match execute_single_tool_with_phases(
                    tool.as_deref(),
                    tool_call,
                    &state,
                    &tool_descriptors,
                    &config.plugins,
                    None,
                    Some(&rt_for_replay),
                    &thread.id,
                )
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };

                if replay_result.pending_interaction.is_some() {
                    terminate_stream_error!(format!(
                        "replayed tool '{}' requested pending interaction",
                        tool_call.id
                    ));
                }

                // Append real replay tool result as a new tool message (append-only log).
                let replay_msg_id = gen_message_id();
                let mut replay_mutations = ThreadMutationBatch::default().with_message(
                    tool_response(&tool_call.id, &replay_result.execution.result)
                        .with_id(replay_msg_id.clone()),
                );

                // Preserve reminder emission semantics for replayed tool calls.
                if !replay_result.reminders.is_empty() {
                    let msgs = replay_result.reminders.iter().map(|reminder| {
                        Message::internal_system(format!(
                            "<system-reminder>{}</system-reminder>",
                            reminder
                        ))
                    });
                    replay_mutations = replay_mutations.with_messages(msgs);
                }

                if let Some(patch) = replay_result.execution.patch.clone() {
                    replay_state_changed = true;
                    replay_mutations = replay_mutations.with_patch(patch);
                }
                if !replay_result.pending_patches.is_empty() {
                    replay_state_changed = true;
                    replay_mutations =
                        replay_mutations.with_patches(replay_result.pending_patches.clone());
                }
                thread = reduce_thread_mutations(thread, replay_mutations);

                yield AgentEvent::ToolCallDone {
                    id: tool_call.id.clone(),
                    result: replay_result.execution.result,
                    patch: replay_result.execution.patch,
                    message_id: replay_msg_id,
                };
            }

            // Clear pending_interaction state after replaying tools.
            let state = match thread.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(format!("failed to rebuild state after replay: {e}"));
                }
            };

                let clear_patch = clear_agent_pending_interaction(&state);
                if !clear_patch.patch().is_empty() {
                    replay_state_changed = true;
                    thread = reduce_thread_mutations(
                        thread,
                        ThreadMutationBatch::default().with_patch(clear_patch),
                    );
                }

            if replay_state_changed {
                let snapshot = match thread.rebuild_state() {
                    Ok(s) => s,
                    Err(e) => {
                        terminate_stream_error!(format!("failed to rebuild replay snapshot: {e}"));
                    }
                };
                yield AgentEvent::StateSnapshot {
                    snapshot,
                };
            }
        }

        loop {
            let (next_thread, outbox) =
                match drain_agent_outbox(thread.clone(), "agent_outbox_loop_tick")
            {
                Ok(v) => v,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            thread = next_thread;
            for resolution in outbox.interaction_resolutions {
                yield AgentEvent::InteractionResolved {
                    interaction_id: resolution.interaction_id,
                    result: resolution.result,
                };
            }
            if !outbox.replay_tool_calls.is_empty() {
                terminate_stream_error!(
                    "unexpected replay_tool_calls outside session start".to_string()
                );
            }

            // Check cancellation at the top of each iteration.
            if let Some(ref token) = run_cancellation_token {
                if token.is_cancelled() {
                    thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                    ensure_run_finished_delta_or_error!();
                    yield AgentEvent::RunFinish {
                        thread_id: thread.id.clone(),
                        run_id: run_id.clone(),
                        result: None,
                        stop_reason: Some(StopReason::Cancelled),
                    };
                    return;
                }
            }

            // Phase: StepStart and BeforeInference (collect messages and tools filter)
            let step_prepare = run_phase_block(
                &thread,
                &tool_descriptors,
                &config.plugins,
                &[Phase::StepStart, Phase::BeforeInference],
                |_| {},
                |step| inference_inputs_from_step(step, &config.system_prompt),
            )
            .await;
            let ((messages, filtered_tools, skip_inference, tracing_span), pending) =
                match step_prepare {
                    Ok(v) => v,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
            thread = apply_pending_patches(thread, pending);

            // Skip inference if requested
            if skip_inference {
                let pending_interaction = thread
                    .rebuild_state()
                    .ok()
                    .and_then(|s| {
                        s.get(AGENT_STATE_PATH)?
                            .get("pending_interaction")
                            .cloned()
                    })
                    .and_then(|v| serde_json::from_value::<Interaction>(v).ok());
                if let Some(interaction) = pending_interaction.clone() {
                    yield AgentEvent::InteractionRequested {
                        interaction: interaction.clone(),
                    };
                    yield AgentEvent::Pending { interaction };
                }
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: if pending_interaction.is_some() {
                        None
                    } else {
                        Some(StopReason::PluginRequested)
                    },
                };
                return;
            }

            // Build request with filtered tools
            let request = build_request_for_filtered_tools(&messages, &tools, &filtered_tools);

            // Step boundary: starting LLM call
            let assistant_msg_id = gen_message_id();
            yield AgentEvent::StepStart { message_id: assistant_msg_id.clone() };

            // Stream LLM response (instrumented with tracing span)
            let inference_span = tracing_span.unwrap_or_else(tracing::Span::none);
            let stream_result = async {
                provider
                    .exec_chat_stream_events(&config.model, request, config.chat_options.as_ref())
                    .await
            }
            .instrument(inference_span)
            .await;

            let chat_stream_events = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                    match emit_cleanup_phases_and_apply(
                        thread.clone(),
                        &tool_descriptors,
                        &config.plugins,
                        "llm_stream_start_error",
                        e.to_string(),
                    )
                    .await
                    {
                        Ok(next_thread) => {
                            thread = next_thread;
                        }
                        Err(phase_error) => {
                            terminate_stream_error!(phase_error.to_string());
                        }
                    }
                    terminate_stream_error!(e.to_string());
                }
            };

            // Collect streaming response
            let inference_start = std::time::Instant::now();
            let mut collector = StreamCollector::new();
            let mut chat_stream = chat_stream_events;

            loop {
                let next_event = if let Some(ref token) = run_cancellation_token {
                    tokio::select! {
                        _ = token.cancelled() => {
                            thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                            ensure_run_finished_delta_or_error!();
                            yield AgentEvent::RunFinish {
                                thread_id: thread.id.clone(),
                                run_id: run_id.clone(),
                                result: None,
                                stop_reason: Some(StopReason::Cancelled),
                            };
                            return;
                        }
                        ev = chat_stream.next() => ev,
                    }
                } else {
                    chat_stream.next().await
                };

                let Some(event_result) = next_event else {
                    break;
                };

                match event_result {
                    Ok(event) => {
                        if let Some(output) = collector.process(event) {
                            match output {
                                crate::runtime::streaming::StreamOutput::TextDelta(delta) => {
                                    yield AgentEvent::TextDelta { delta };
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallStart { id, name } => {
                                    yield AgentEvent::ToolCallStart { id, name };
                                }
                                crate::runtime::streaming::StreamOutput::ToolCallDelta { id, args_delta } => {
                                    yield AgentEvent::ToolCallDelta { id, args_delta };
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                        match emit_cleanup_phases_and_apply(
                            thread.clone(),
                            &tool_descriptors,
                            &config.plugins,
                            "llm_stream_event_error",
                            e.to_string(),
                        )
                        .await
                        {
                            Ok(next_thread) => {
                                thread = next_thread;
                            }
                            Err(phase_error) => {
                                terminate_stream_error!(phase_error.to_string());
                            }
                        }
                        terminate_stream_error!(e.to_string());
                    }
                }
            }

            let result = collector.finish();
            loop_state.update_from_response(&result);
            let inference_duration_ms = inference_start.elapsed().as_millis() as u64;

            yield AgentEvent::InferenceComplete {
                model: config.model.clone(),
                usage: result.usage.clone(),
                duration_ms: inference_duration_ms,
            };

            // Phase: AfterInference (with new context)
            match emit_phase_block(
                Phase::AfterInference,
                &thread,
                &tool_descriptors,
                &config.plugins,
                |step| {
                    step.response = Some(result.clone());
                },
            )
            .await
            {
                Ok(pending) => {
                    thread = apply_pending_patches(thread, pending);
                }
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            }

            // Add assistant message with run/step metadata.
            let step_meta = step_metadata(Some(run_id.clone()), loop_state.rounds as u32);
            let assistant_msg = if result.tool_calls.is_empty() {
                assistant_message(&result.text)
                    .with_id(assistant_msg_id.clone())
                    .with_metadata(step_meta.clone())
            } else {
                assistant_tool_calls(&result.text, result.tool_calls.clone())
                    .with_id(assistant_msg_id.clone())
                    .with_metadata(step_meta.clone())
            };
            thread = reduce_thread_mutations(
                thread,
                ThreadMutationBatch::default().with_message(assistant_msg),
            );

            // Phase: StepEnd (with new context) — run plugin cleanup before yielding StepEnd
            match emit_phase_block(
                Phase::StepEnd,
                &thread,
                &tool_descriptors,
                &config.plugins,
                |_| {},
            )
            .await
            {
                Ok(pending) => {
                    thread = apply_pending_patches(thread, pending);
                }
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            }

            if let Err(e) = commit_pending_delta(
                &mut thread,
                CheckpointReason::AssistantTurnCommitted,
                false,
                &run_id,
                parent_run_id.as_deref(),
                state_committer.as_ref(),
            )
            .await
            {
                terminate_stream_error!(e.to_string());
            }

            // Step boundary: finished LLM call
            yield AgentEvent::StepEnd;

            // Check if we need to execute tools
            if !result.needs_tools() {
                let result_value = if result.text.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({"response": result.text}))
                };
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: result_value,
                    stop_reason: Some(StopReason::NaturalEnd),
                };
                return;
            }

            // Emit ToolCallReady for each finalized tool call
            for tc in &result.tool_calls {
                yield AgentEvent::ToolCallReady {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                };
            }

            // Execute tools with phase hooks
            let state = match thread.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };

            let rt_for_tools =
                match runtime_with_tool_caller_context(&thread, &state, Some(&config)) {
                Ok(rt) => rt,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            let sid_for_tools = thread.id.clone();
            let mut tool_future: Pin<Box<dyn Future<Output = Result<Vec<ToolExecutionResult>, AgentLoopError>> + Send>> =
                Box::pin(execute_tool_calls_with_phases(
                    &tools,
                    &result.tool_calls,
                    &state,
                    &tool_descriptors,
                    &config.plugins,
                    config.parallel_tools,
                    Some(activity_manager.clone()),
                    Some(&rt_for_tools),
                    &sid_for_tools,
                ));
            let mut activity_closed = false;
            let results = loop {
                tokio::select! {
                    _ = async {
                        if let Some(ref token) = run_cancellation_token {
                            token.cancelled().await;
                        } else {
                            futures::future::pending::<()>().await;
                        }
                    } => {
                        thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                        ensure_run_finished_delta_or_error!();
                        yield AgentEvent::RunFinish {
                            thread_id: thread.id.clone(),
                            run_id: run_id.clone(),
                            result: None,
                            stop_reason: Some(StopReason::Cancelled),
                        };
                        return;
                    }
                    activity = activity_rx.recv(), if !activity_closed => {
                        match activity {
                            Some(event) => {
                                yield event;
                            }
                            None => {
                                activity_closed = true;
                            }
                        }
                    }
                    res = &mut tool_future => {
                        break res;
                    }
                }
            };

            while let Ok(event) = activity_rx.try_recv() {
                yield event;
            }

            let results = match results {
                Ok(r) => r,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };

            // Emit pending interaction event(s) first.
            for exec_result in &results {
                if let Some(ref interaction) = exec_result.pending_interaction {
                    yield AgentEvent::InteractionRequested {
                        interaction: interaction.clone(),
                    };
                    yield AgentEvent::Pending {
                        interaction: interaction.clone(),
                    };
                }
            }
            let session_before_apply = thread.clone();
            // Pre-generate message IDs for tool results so streaming events
            // and stored Messages share the same ID.
            let tool_msg_ids: HashMap<String, String> = results
                .iter()
                .filter(|r| r.pending_interaction.is_none())
                .map(|r| (r.execution.call.id.clone(), gen_message_id()))
                .collect();

            let applied = match apply_tool_results_impl(
                thread,
                &results,
                Some(step_meta),
                config.parallel_tools,
                Some(&tool_msg_ids),
            ) {
                Ok(a) => a,
                Err(e) => {
                    thread = session_before_apply;
                    terminate_stream_error!(e.to_string());
                }
            };
            thread = applied.thread;

            if let Err(e) = commit_pending_delta(
                &mut thread,
                CheckpointReason::ToolResultsCommitted,
                false,
                &run_id,
                parent_run_id.as_deref(),
                state_committer.as_ref(),
            )
            .await
            {
                terminate_stream_error!(e.to_string());
            }

            // Emit non-pending tool results (pending ones pause the run).
            for exec_result in &results {
                if exec_result.pending_interaction.is_none() {
                    yield AgentEvent::ToolCallDone {
                        id: exec_result.execution.call.id.clone(),
                        result: exec_result.execution.result.clone(),
                        patch: exec_result.execution.patch.clone(),
                        message_id: tool_msg_ids.get(&exec_result.execution.call.id).cloned().unwrap_or_default(),
                    };
                }
            }

            // Emit state snapshot when we mutated state (tool patches or AgentState pending/clear).
            if let Some(snapshot) = applied.state_snapshot {
                yield AgentEvent::StateSnapshot { snapshot };
            }

            // If there are pending interactions, pause the loop.
            // Client must respond and start a new run to continue.
            if applied.pending_interaction.is_some() {
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: None, // Pause, not a stop
                };
                return;
            }

            // Track tool round metrics for stop condition evaluation.
            let error_count = results
                .iter()
                .filter(|r| r.execution.result.is_error())
                .count();
            loop_state.record_tool_round(&result.tool_calls, error_count);
            loop_state.rounds += 1;

            // Check stop conditions.
            let stop_ctx = loop_state.to_check_context(&result, &thread);
            if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
                thread = emit_session_end(thread, &tool_descriptors, &config.plugins).await;
                ensure_run_finished_delta_or_error!();
                yield AgentEvent::RunFinish {
                    thread_id: thread.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: Some(reason),
                };
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests;
