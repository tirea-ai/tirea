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
//! RunStart (once)
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
//! RunEnd (once)
//! ```

mod core;
mod outcome;
mod plugin_runtime;
mod run_state;
mod state_commit;
mod stream_core;
mod stream_runner;
mod tool_exec;

use crate::contracts::conversation::Thread;
use crate::contracts::conversation::{gen_message_id, Message, MessageMetadata};
use crate::contracts::events::{AgentEvent, StreamResult, TerminationReason};
use crate::contracts::phase::Phase;
use crate::contracts::state_types::{Interaction, AGENT_STATE_PATH};
use crate::contracts::storage::CheckpointReason;
use crate::contracts::traits::tool::Tool;
use crate::engine::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::engine::stop_conditions::{check_stop_conditions, StopReason};
use crate::runtime::activity::ActivityHub;
use crate::runtime::streaming::StreamCollector;
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
    AgentConfig, AgentDefinition, LlmRetryPolicy, RunCancellationToken, RunContext,
    StateCommitError, StateCommitter,
};
pub(crate) use carve_agent_contract::agent::{
    TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
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
pub use outcome::{run_step_cycle, tool_map, tool_map_from_arc, AgentLoopError, StepResult};
#[cfg(test)]
use plugin_runtime::emit_phase_checked;
use plugin_runtime::{
    emit_cleanup_phases_and_apply, emit_phase_block, emit_run_end_phase,
    prepare_stream_error_termination, run_phase_block,
};
use run_state::{effective_stop_conditions, RunState};
use state_commit::commit_pending_delta;
pub use state_commit::ChannelStateCommitter;
#[cfg(test)]
use stream_runner::run_loop_stream_impl_with_provider;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use tool_exec::execute_tools_parallel_with_phases;
use tool_exec::{
    apply_tool_results_impl, apply_tool_results_to_session, execute_single_tool_with_phases,
    execute_tool_calls_with_phases, next_step_index, runtime_with_tool_caller_context,
    step_metadata, ToolExecutionResult,
};
pub use tool_exec::{execute_tools, execute_tools_with_config, execute_tools_with_plugins};

fn uuid_v7() -> String {
    Uuid::now_v7().simple().to_string()
}

pub(super) fn effective_llm_models(config: &AgentConfig) -> Vec<String> {
    let mut models = Vec::with_capacity(1 + config.fallback_models.len());
    models.push(config.model.clone());
    for model in &config.fallback_models {
        if model.trim().is_empty() {
            continue;
        }
        if !models.iter().any(|m| m == model) {
            models.push(model.clone());
        }
    }
    models
}

pub(super) fn llm_retry_attempts(config: &AgentConfig) -> usize {
    config.llm_retry_policy.max_attempts_per_model.max(1)
}

pub(super) fn is_retryable_llm_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    let non_retryable = [
        "401",
        "403",
        "404",
        "400",
        "422",
        "unauthorized",
        "forbidden",
        "invalid api key",
        "invalid_request",
        "bad request",
    ];
    if non_retryable.iter().any(|p| lower.contains(p)) {
        return false;
    }
    let retryable = [
        "429",
        "too many requests",
        "rate limit",
        "timeout",
        "timed out",
        "temporar",
        "connection",
        "network",
        "unavailable",
        "server error",
        "502",
        "503",
        "504",
        "reset by peer",
        "eof",
    ];
    retryable.iter().any(|p| lower.contains(p))
}

pub(super) fn retry_backoff_ms(config: &AgentConfig, retry_index: usize) -> u64 {
    let initial = config.llm_retry_policy.initial_backoff_ms;
    let cap = config
        .llm_retry_policy
        .max_backoff_ms
        .max(config.llm_retry_policy.initial_backoff_ms);
    if retry_index == 0 {
        return initial.min(cap);
    }
    let shift = (retry_index - 1).min(20) as u32;
    let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
    initial.saturating_mul(factor).min(cap)
}

pub(super) async fn wait_retry_backoff(
    config: &AgentConfig,
    retry_index: usize,
    run_cancellation_token: Option<&RunCancellationToken>,
) -> bool {
    let wait_ms = retry_backoff_ms(config, retry_index);
    if let Some(token) = run_cancellation_token {
        tokio::select! {
            _ = token.cancelled() => true,
            _ = tokio::time::sleep(std::time::Duration::from_millis(wait_ms)) => false,
        }
    } else {
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        false
    }
}

pub(super) enum LlmAttemptOutcome<T> {
    Success { value: T, model: String },
    Cancelled,
    Exhausted { last_error: String },
}

pub(super) async fn run_llm_with_retry_and_fallback<T, Invoke, Fut>(
    config: &AgentConfig,
    run_cancellation_token: Option<&RunCancellationToken>,
    retry_current_model: bool,
    unknown_error: &str,
    mut invoke: Invoke,
) -> LlmAttemptOutcome<T>
where
    Invoke: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = genai::Result<T>>,
{
    let mut last_llm_error = unknown_error.to_string();
    let model_candidates = effective_llm_models(config);
    let max_attempts = llm_retry_attempts(config);

    for model in model_candidates {
        for attempt in 1..=max_attempts {
            let response_res = if let Some(token) = run_cancellation_token {
                tokio::select! {
                    _ = token.cancelled() => return LlmAttemptOutcome::Cancelled,
                    resp = invoke(model.clone()) => resp,
                }
            } else {
                invoke(model.clone()).await
            };

            match response_res {
                Ok(value) => {
                    return LlmAttemptOutcome::Success { value, model };
                }
                Err(e) => {
                    let message = e.to_string();
                    last_llm_error =
                        format!("model='{model}' attempt={attempt}/{max_attempts}: {message}");
                    let can_retry_same_model = retry_current_model
                        && attempt < max_attempts
                        && is_retryable_llm_error(&message);
                    if can_retry_same_model {
                        let cancelled =
                            wait_retry_backoff(config, attempt, run_cancellation_token).await;
                        if cancelled {
                            return LlmAttemptOutcome::Cancelled;
                        }
                        continue;
                    }
                    break;
                }
            }
        }
    }

    LlmAttemptOutcome::Exhausted {
        last_error: last_llm_error,
    }
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

async fn run_step_prepare_phases(
    thread: &Thread,
    tool_descriptors: &[crate::contracts::traits::tool::ToolDescriptor],
    config: &AgentConfig,
) -> Result<
    (
        Vec<Message>,
        Vec<String>,
        bool,
        Option<tracing::Span>,
        Vec<TrackedPatch>,
    ),
    AgentLoopError,
> {
    let ((messages, filtered_tools, skip_inference, tracing_span), pending) = run_phase_block(
        thread,
        tool_descriptors,
        &config.plugins,
        &[Phase::StepStart, Phase::BeforeInference],
        |_| {},
        |step| inference_inputs_from_step(step, &config.system_prompt),
    )
    .await?;
    Ok((
        messages,
        filtered_tools,
        skip_inference,
        tracing_span,
        pending,
    ))
}

fn stream_result_from_chat_response(response: &genai::chat::ChatResponse) -> StreamResult {
    let text = response
        .first_text()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let tool_calls: Vec<crate::contracts::conversation::ToolCall> = response
        .tool_calls()
        .into_iter()
        .map(|tc| {
            crate::contracts::conversation::ToolCall::new(
                &tc.call_id,
                &tc.fn_name,
                tc.fn_arguments.clone(),
            )
        })
        .collect();

    StreamResult {
        text,
        tool_calls,
        usage: Some(response.usage.clone()),
    }
}

fn assistant_turn_message(
    result: &StreamResult,
    step_meta: MessageMetadata,
    message_id: Option<String>,
) -> Message {
    let mut msg = if result.tool_calls.is_empty() {
        assistant_message(&result.text)
    } else {
        assistant_tool_calls(&result.text, result.tool_calls.clone())
    }
    .with_metadata(step_meta);
    if let Some(message_id) = message_id {
        msg = msg.with_id(message_id);
    }
    msg
}

/// Run one step of the agent loop (non-streaming).
///
/// A step consists of:
/// 1. Emit StepStart phase
/// 2. Emit BeforeInference phase
/// 3. Send messages to LLM
/// 4. Emit AfterInference phase
/// 5. Emit StepEnd phase
/// 6. Return the thread and result (caller handles tool execution)
pub async fn run_step(
    client: &Client,
    config: &AgentConfig,
    thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Thread, StreamResult), AgentLoopError> {
    let tool_descriptors = tool_descriptors_for_config(tools, config);

    let (messages, filtered_tools, skip_inference, tracing_span, pending) =
        run_step_prepare_phases(&thread, &tool_descriptors, config).await?;
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
            let _thread = emit_cleanup_phases_and_apply(
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

    let result = stream_result_from_chat_response(&response);

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
    let thread = thread.with_message(assistant_turn_message(&result, step_meta, None));

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
/// Returns the final thread and the last response text.
pub async fn run_loop(
    client: &Client,
    config: &AgentConfig,
    thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Thread, String), AgentLoopError> {
    run_loop_with_context(client, config, thread, tools, RunContext::default()).await
}

/// Run the full agent loop with explicit run context.
///
/// This is the non-streaming counterpart of `run_loop_stream(..., run_ctx)`,
/// allowing cooperative cancellation via `run_ctx.cancellation_token`.
pub async fn run_loop_with_context(
    client: &Client,
    config: &AgentConfig,
    mut thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Result<(Thread, String), AgentLoopError> {
    let mut run_state = RunState::new();
    let stop_conditions = effective_stop_conditions(config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
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

    // Phase: RunStart
    let pending = emit_phase_block(
        Phase::RunStart,
        &thread,
        &tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await?;
    thread = apply_pending_patches(thread, pending);

    loop {
        if let Some(ref token) = run_cancellation_token {
            if token.is_cancelled() {
                thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::Cancelled {
                    thread: Box::new(thread),
                });
            }
        }

        // Phase: StepStart and BeforeInference
        let (messages, filtered_tools, skip_inference, tracing_span, pending) =
            match run_step_prepare_phases(&thread, &tool_descriptors, config).await {
                Ok(v) => v,
                Err(e) => {
                    let _finalized =
                        emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                    return Err(e);
                }
            };
        thread = apply_pending_patches(thread, pending);

        if skip_inference {
            break;
        }

        // Call LLM with unified retry + fallback model strategy.
        let inference_span = tracing_span.unwrap_or_else(tracing::Span::none);
        let attempt_outcome = run_llm_with_retry_and_fallback(
            config,
            run_cancellation_token.as_ref(),
            true,
            "unknown llm error",
            |model| {
                let request = build_request_for_filtered_tools(&messages, tools, &filtered_tools);
                async move {
                    client
                        .exec_chat(&model, request, config.chat_options.as_ref())
                        .await
                }
                .instrument(inference_span.clone())
            },
        )
        .await;

        let response = match attempt_outcome {
            LlmAttemptOutcome::Success { value, .. } => value,
            LlmAttemptOutcome::Cancelled => {
                thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::Cancelled {
                    thread: Box::new(thread),
                });
            }
            LlmAttemptOutcome::Exhausted { last_error } => {
                // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                thread = match emit_cleanup_phases_and_apply(
                    thread.clone(),
                    &tool_descriptors,
                    &config.plugins,
                    "llm_exec_error",
                    last_error.clone(),
                )
                .await
                {
                    Ok(s) => s,
                    Err(phase_error) => {
                        let _finalized =
                            emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                        return Err(phase_error);
                    }
                };
                let _finalized =
                    emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::LlmError(last_error));
            }
        };

        let result = stream_result_from_chat_response(&response);
        run_state.update_from_response(&result);
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
                let _finalized =
                    emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        }

        // Add assistant message
        let assistant_msg_id = gen_message_id();
        let step_meta = step_metadata(Some(run_id.clone()), run_state.completed_steps as u32);
        let assistant_msg =
            assistant_turn_message(&result, step_meta.clone(), Some(assistant_msg_id.clone()));
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
                let _finalized =
                    emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        }

        if !result.needs_tools() {
            let stop_ctx = run_state.to_check_context(&result, &thread);
            if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
                thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::Stopped {
                    thread: Box::new(thread),
                    reason,
                });
            }
            break;
        }

        // Execute tools with phase hooks (respect config.parallel_tools).
        let state = match thread.rebuild_state() {
            Ok(s) => s,
            Err(e) => {
                emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::StateError(e.to_string()));
            }
        };
        let rt_for_tools = match runtime_with_tool_caller_context(&thread, &state, Some(config)) {
            Ok(rt) => rt,
            Err(e) => {
                emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        };

        let tool_exec_future = execute_tool_calls_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            &config.plugins,
            config.parallel_tools,
            None,
            Some(&rt_for_tools),
            &thread.id,
        );
        let results = if let Some(ref token) = run_cancellation_token {
            tokio::select! {
                _ = token.cancelled() => {
                    thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                    return Err(AgentLoopError::Cancelled {
                        thread: Box::new(thread),
                    });
                }
                results = tool_exec_future => results,
            }
        } else {
            tool_exec_future.await
        };

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                let _finalized =
                    emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
                return Err(e);
            }
        };

        let thread_before_apply = thread.clone();
        let applied = match apply_tool_results_to_session(
            thread,
            &results,
            Some(step_meta),
            config.parallel_tools,
        ) {
            Ok(a) => a,
            Err(e) => {
                let _finalized =
                    emit_run_end_phase(thread_before_apply, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        };
        thread = applied.thread;

        // Pause if any tool is waiting for client response.
        if let Some(interaction) = applied.pending_interaction {
            thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
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
        run_state.record_tool_step(&result.tool_calls, error_count);
        run_state.completed_steps += 1;

        // Check stop conditions.
        let stop_ctx = run_state.to_check_context(&result, &thread);
        if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
            thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;
            return Err(AgentLoopError::Stopped {
                thread: Box::new(thread),
                reason,
            });
        }
    }

    // Phase: RunEnd
    thread = emit_run_end_phase(thread, &tool_descriptors, &config.plugins).await;

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
    stream_runner::run_loop_stream_impl_with_provider(
        Arc::new(client),
        config,
        thread,
        tools,
        run_ctx,
    )
}

#[cfg(test)]
mod tests;
