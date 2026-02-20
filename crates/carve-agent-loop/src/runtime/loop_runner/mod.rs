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

mod config;
mod core;
mod outcome;
mod plugin_runtime;
mod run_state;
mod state_commit;
mod stream_core;
mod stream_runner;
mod tool_exec;

use crate::contracts::plugin::phase::Phase;
use crate::contracts::{AgentEvent, FrontendToolInvocation, Interaction, TerminationReason};
use crate::contracts::runtime::{
    StopPolicy, StreamResult, ToolExecutionRequest,
    ToolExecutionResult,
};
use crate::contracts::thread::CheckpointReason;
use crate::contracts::thread::{gen_message_id, Message, MessageMetadata};
use crate::contracts::runtime::ActivityManager;
use crate::contracts::tool::Tool;
use crate::contracts::RunContext;
use crate::engine::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::engine::stop_conditions::check_stop_policies;
use crate::contracts::StopReason;
use crate::runtime::activity::ActivityHub;

use crate::runtime::streaming::StreamCollector;
use async_stream::stream;
use futures::{Stream, StreamExt};
use genai::Client;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

#[cfg(test)]
use crate::contracts::plugin::AgentPlugin;
#[cfg(test)]
use crate::contracts::plugin::phase::StepContext;
pub use crate::runtime::run_context::{
    RunCancellationToken, StateCommitError, StateCommitter,
    TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY, TOOL_SCOPE_CALLER_STATE_KEY,
    TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use carve_state::TrackedPatch;
pub use crate::contracts::runtime::{LlmExecutor, ToolExecutor};
pub use config::{AgentConfig, GenaiLlmExecutor, LlmRetryPolicy};
pub use config::{StepToolInput, StepToolProvider, StepToolSnapshot};
use config::StaticStepToolProvider;
#[cfg(test)]
use core::build_messages;
use core::{
    build_request_for_filtered_tools, clear_agent_pending_interaction,
    drain_agent_outbox, inference_inputs_from_step, pending_frontend_invocation_from_ctx,
    pending_interaction_from_ctx, set_agent_pending_interaction,
};
pub use outcome::{tool_map, tool_map_from_arc, AgentLoopError};
pub use outcome::{LoopOutcome, LoopStats, LoopUsage};
#[cfg(test)]
use plugin_runtime::emit_phase_checked;
use plugin_runtime::{
    emit_cleanup_phases_and_apply, emit_phase_block, emit_run_end_phase, run_phase_block,
};
use run_state::{effective_stop_conditions, RunState};
pub use state_commit::ChannelStateCommitter;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(test)]
use stream_runner::run_loop_stream_impl;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use tool_exec::execute_tools_parallel_with_phases;
use tool_exec::{
    apply_tool_results_impl, apply_tool_results_to_session, execute_single_tool_with_phases,
    scope_with_tool_caller_context, step_metadata,
};
pub use tool_exec::{
    execute_tools, execute_tools_with_config, execute_tools_with_plugins,
    execute_tools_with_plugins_and_executor, ParallelToolExecutor, SequentialToolExecutor,
};

fn uuid_v7() -> String {
    Uuid::now_v7().simple().to_string()
}


pub(crate) fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
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
    Success {
        value: T,
        model: String,
        attempts: usize,
    },
    Cancelled,
    Exhausted {
        last_error: String,
        attempts: usize,
    },
}

fn is_run_cancelled(token: Option<&RunCancellationToken>) -> bool {
    token.is_some_and(|t| t.is_cancelled())
}

pub(super) fn step_tool_provider_for_run(
    config: &AgentConfig,
    tools: HashMap<String, Arc<dyn Tool>>,
) -> Arc<dyn StepToolProvider> {
    config.step_tool_provider.clone().unwrap_or_else(|| {
        Arc::new(StaticStepToolProvider::new(tools)) as Arc<dyn StepToolProvider>
    })
}

pub(super) fn llm_executor_for_run(config: &AgentConfig) -> Arc<dyn LlmExecutor> {
    config
        .llm_executor
        .clone()
        .unwrap_or_else(|| Arc::new(GenaiLlmExecutor::new(Client::default())))
}

pub(super) async fn resolve_step_tool_snapshot(
    step_tool_provider: &Arc<dyn StepToolProvider>,
    run_ctx: &RunContext,
) -> Result<StepToolSnapshot, AgentLoopError> {
    step_tool_provider.provide(StepToolInput { state: run_ctx }).await
}

fn mark_step_completed(run_state: &mut RunState) {
    run_state.completed_steps += 1;
}

fn build_loop_outcome(
    run_ctx: RunContext,
    termination: TerminationReason,
    response: Option<String>,
    run_state: &RunState,
    failure: Option<outcome::LoopFailure>,
) -> LoopOutcome {
    LoopOutcome {
        run_ctx,
        termination,
        response: response.filter(|text| !text.is_empty()),
        usage: run_state.usage(),
        stats: run_state.stats(),
        failure,
    }
}

fn stop_reason_for_step(
    run_state: &RunState,
    result: &StreamResult,
    run_ctx: &RunContext,
    stop_conditions: &[Arc<dyn StopPolicy>],
) -> Option<StopReason> {
    let stop_input = run_state.to_policy_input(result, run_ctx);
    check_stop_policies(stop_conditions, &stop_input)
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
    let mut total_attempts = 0usize;

    for model in model_candidates {
        for attempt in 1..=max_attempts {
            total_attempts = total_attempts.saturating_add(1);
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
                    return LlmAttemptOutcome::Success {
                        value,
                        model,
                        attempts: total_attempts,
                    };
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
        attempts: total_attempts,
    }
}

pub(super) async fn run_step_prepare_phases(
    run_ctx: &RunContext,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    config: &AgentConfig,
) -> Result<
    (Vec<Message>, Vec<String>, bool, Vec<TrackedPatch>),
    AgentLoopError,
> {
    let ((messages, filtered_tools, skip_inference), pending) = run_phase_block(
        run_ctx,
        tool_descriptors,
        &config.plugins,
        &[Phase::StepStart, Phase::BeforeInference],
        |_| {},
        |step| inference_inputs_from_step(step, &config.system_prompt),
    )
    .await?;
    Ok((messages, filtered_tools, skip_inference, pending))
}

pub(super) struct PreparedStep {
    pub(super) messages: Vec<Message>,
    pub(super) filtered_tools: Vec<String>,
    pub(super) skip_inference: bool,
    pub(super) pending_patches: Vec<TrackedPatch>,
}

pub(super) async fn prepare_step_execution(
    run_ctx: &RunContext,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    config: &AgentConfig,
) -> Result<PreparedStep, AgentLoopError> {
    let (messages, filtered_tools, skip_inference, pending) =
        run_step_prepare_phases(run_ctx, tool_descriptors, config).await?;
    Ok(PreparedStep {
        messages,
        filtered_tools,
        skip_inference,
        pending_patches: pending,
    })
}

pub(super) async fn apply_llm_error_cleanup(
    run_ctx: &mut RunContext,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
    error_type: &'static str,
    message: String,
) -> Result<(), AgentLoopError> {
    emit_cleanup_phases_and_apply(
        run_ctx,
        tool_descriptors,
        plugins,
        error_type,
        message,
    )
    .await
}

pub(super) async fn complete_step_after_inference(
    run_ctx: &mut RunContext,
    result: &StreamResult,
    step_meta: MessageMetadata,
    assistant_message_id: Option<String>,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
) -> Result<(), AgentLoopError> {
    let pending = emit_phase_block(
        Phase::AfterInference,
        run_ctx,
        tool_descriptors,
        plugins,
        |step| {
            step.response = Some(result.clone());
        },
    )
    .await?;
    run_ctx.add_thread_patches(pending);

    let assistant = assistant_turn_message(result, step_meta, assistant_message_id);
    run_ctx.add_message(Arc::new(assistant));

    let pending = emit_phase_block(
        Phase::StepEnd,
        run_ctx,
        tool_descriptors,
        plugins,
        |_| {},
    )
    .await?;
    run_ctx.add_thread_patches(pending);
    Ok(())
}

pub(super) fn interaction_requested_pending_events(interaction: &Interaction) -> [AgentEvent; 2] {
    [
        AgentEvent::InteractionRequested {
            interaction: interaction.clone(),
        },
        AgentEvent::Pending {
            interaction: interaction.clone(),
        },
    ]
}

/// Emit events for a pending frontend tool invocation.
///
/// When a `FrontendToolInvocation` is available, emits standard `ToolCallStart` +
/// `ToolCallReady` events using the frontend invocation's identity. This makes
/// frontend tools appear as regular tool calls in the event stream.
///
/// Falls back to legacy `InteractionRequested` + `Pending` events when no
/// `FrontendToolInvocation` is provided.
pub(super) fn pending_tool_events(
    interaction: &Interaction,
    frontend_invocation: Option<&FrontendToolInvocation>,
) -> Vec<AgentEvent> {
    if let Some(inv) = frontend_invocation {
        vec![
            AgentEvent::ToolCallStart {
                id: inv.call_id.clone(),
                name: inv.tool_name.clone(),
            },
            AgentEvent::ToolCallReady {
                id: inv.call_id.clone(),
                name: inv.tool_name.clone(),
                arguments: inv.arguments.clone(),
            },
        ]
    } else {
        interaction_requested_pending_events(interaction).to_vec()
    }
}

pub(super) struct ToolExecutionContext {
    pub(super) state: serde_json::Value,
    pub(super) run_config: carve_agent_contract::RunConfig,
}

pub(super) fn prepare_tool_execution_context(
    run_ctx: &RunContext,
    config: Option<&AgentConfig>,
) -> Result<ToolExecutionContext, AgentLoopError> {
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let run_config = scope_with_tool_caller_context(run_ctx, &state, config)?;
    Ok(ToolExecutionContext { state, run_config })
}

pub(super) async fn finalize_run_end(
    run_ctx: &mut RunContext,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
) {
    emit_run_end_phase(run_ctx, tool_descriptors, plugins).await
}

fn stream_result_from_chat_response(response: &genai::chat::ChatResponse) -> StreamResult {
    let text = response
        .first_text()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let tool_calls: Vec<crate::contracts::thread::ToolCall> = response
        .tool_calls()
        .into_iter()
        .map(|tc| {
            crate::contracts::thread::ToolCall::new(
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

/// Run the full agent loop until completion or a stop condition is met.
///
/// This is the primary non-streaming entry point. Tools are passed directly
/// and used as the default tool set unless `config.step_tool_provider` is set
/// (for dynamic per-step tool resolution).
pub async fn run_loop(
    config: &AgentConfig,
    tools: HashMap<String, Arc<dyn Tool>>,
    mut run_ctx: RunContext,
    cancellation_token: Option<RunCancellationToken>,
    _state_committer: Option<Arc<dyn StateCommitter>>,
) -> LoopOutcome {
    let executor = llm_executor_for_run(config);
    let mut run_state = RunState::new();
    let stop_conditions = effective_stop_conditions(config);
    let run_cancellation_token = cancellation_token;
    let mut last_text = String::new();
    let step_tool_provider = step_tool_provider_for_run(config, tools);
    let run_id = stream_core::resolve_stream_run_identity(&mut run_ctx).run_id;
    let initial_step_tools = match resolve_step_tool_snapshot(&step_tool_provider, &run_ctx).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return build_loop_outcome(
                run_ctx,
                TerminationReason::Error,
                None,
                &run_state,
                Some(outcome::LoopFailure::State(error.to_string())),
            );
        }
    };
    let mut active_tool_descriptors = initial_step_tools.descriptors;

    macro_rules! terminate_run {
        ($termination:expr, $response:expr, $failure:expr) => {{
            finalize_run_end(&mut run_ctx, &active_tool_descriptors, &config.plugins).await;
            return build_loop_outcome(run_ctx, $termination, $response, &run_state, $failure);
        }};
    }

    // Phase: RunStart
    let pending = match emit_phase_block(
        Phase::RunStart,
        &run_ctx,
        &active_tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await
    {
        Ok(pending) => pending,
        Err(error) => {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
            );
        }
    };
    run_ctx.add_thread_patches(pending);

    loop {
        if is_run_cancelled(run_cancellation_token.as_ref()) {
            terminate_run!(TerminationReason::Cancelled, None, None);
        }

        let step_tools = match resolve_step_tool_snapshot(&step_tool_provider, &run_ctx).await {
            Ok(snapshot) => snapshot,
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };
        active_tool_descriptors = step_tools.descriptors.clone();

        let prepared = match prepare_step_execution(&run_ctx, &active_tool_descriptors, config).await
        {
            Ok(v) => v,
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };
        run_ctx.add_thread_patches(prepared.pending_patches);

        if prepared.skip_inference {
            if pending_interaction_from_ctx(&run_ctx).is_some() {
                terminate_run!(TerminationReason::PendingInteraction, None, None);
            }
            terminate_run!(
                TerminationReason::PluginRequested,
                Some(last_text.clone()),
                None
            );
        }

        // Call LLM with unified retry + fallback model strategy.
        let messages = prepared.messages;
        let filtered_tools = prepared.filtered_tools;
        let attempt_outcome = run_llm_with_retry_and_fallback(
            config,
            run_cancellation_token.as_ref(),
            true,
            "unknown llm error",
            |model| {
                let request =
                    build_request_for_filtered_tools(&messages, &step_tools.tools, &filtered_tools);
                let executor = executor.clone();
                async move {
                    executor
                        .exec_chat_response(&model, request, config.chat_options.as_ref())
                        .await
                }
            },
        )
        .await;

        let response = match attempt_outcome {
            LlmAttemptOutcome::Success {
                value, attempts, ..
            } => {
                run_state.record_llm_attempts(attempts);
                value
            }
            LlmAttemptOutcome::Cancelled => {
                terminate_run!(TerminationReason::Cancelled, None, None);
            }
            LlmAttemptOutcome::Exhausted {
                last_error,
                attempts,
            } => {
                run_state.record_llm_attempts(attempts);
                if let Err(phase_error) = apply_llm_error_cleanup(
                    &mut run_ctx,
                    &active_tool_descriptors,
                    &config.plugins,
                    "llm_exec_error",
                    last_error.clone(),
                )
                .await
                {
                    terminate_run!(
                        TerminationReason::Error,
                        None,
                        Some(outcome::LoopFailure::State(phase_error.to_string()))
                    );
                }
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::Llm(last_error))
                );
            }
        };

        let result = stream_result_from_chat_response(&response);
        run_state.update_from_response(&result);
        last_text = result.text.clone();

        // Add assistant message
        let assistant_msg_id = gen_message_id();
        let step_meta = step_metadata(Some(run_id.clone()), run_state.completed_steps as u32);
        if let Err(e) = complete_step_after_inference(
            &mut run_ctx,
            &result,
            step_meta.clone(),
            Some(assistant_msg_id.clone()),
            &active_tool_descriptors,
            &config.plugins,
        )
        .await
        {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(e.to_string()))
            );
        }

        mark_step_completed(&mut run_state);

        if !result.needs_tools() {
            run_state.record_step_without_tools();
            if let Some(reason) =
                stop_reason_for_step(&run_state, &result, &run_ctx, &stop_conditions)
            {
                terminate_run!(TerminationReason::Stopped(reason), None, None);
            }
            terminate_run!(TerminationReason::NaturalEnd, Some(last_text.clone()), None);
        }

        // Execute tools with phase hooks using configured execution strategy.
        let tool_context = match prepare_tool_execution_context(&run_ctx, Some(config)) {
            Ok(ctx) => ctx,
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };
        let thread_messages_for_tools = run_ctx.messages().to_vec();
        let thread_version_for_tools = run_ctx.version();

        let tool_exec_future = config.tool_executor.execute(ToolExecutionRequest {
            tools: &step_tools.tools,
            calls: &result.tool_calls,
            state: &tool_context.state,
            tool_descriptors: &active_tool_descriptors,
            plugins: &config.plugins,
            activity_manager: None,
            run_config: Some(&tool_context.run_config),
            thread_id: run_ctx.thread_id(),
            thread_messages: &thread_messages_for_tools,
            state_version: thread_version_for_tools,
            cancellation_token: run_cancellation_token.as_ref(),
        });
        let results = tool_exec_future.await.map_err(AgentLoopError::from);

        let results = match results {
            Ok(r) => r,
            Err(AgentLoopError::Cancelled { .. }) => {
                terminate_run!(TerminationReason::Cancelled, None, None);
            }
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };

        let applied = match apply_tool_results_to_session(
            &mut run_ctx,
            &results,
            Some(step_meta),
            config
                .tool_executor
                .requires_parallel_patch_conflict_check(),
        ) {
            Ok(a) => a,
            Err(_e) => {
                // On error, we can't easily rollback RunContext, so just terminate
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(_e.to_string()))
                );
            }
        };

        // Pause if any tool is waiting for client response.
        if let Some(_interaction) = applied.pending_interaction {
            terminate_run!(TerminationReason::PendingInteraction, None, None);
        }

        // Track tool-step metrics for post-tool stop condition evaluation.
        let error_count = results
            .iter()
            .filter(|r| r.execution.result.is_error())
            .count();
        run_state.record_tool_step(&result.tool_calls, error_count);

        // Check stop conditions.
        if let Some(reason) = stop_reason_for_step(&run_state, &result, &run_ctx, &stop_conditions) {
            terminate_run!(TerminationReason::Stopped(reason), None, None);
        }
    }
}

/// Run the agent loop with streaming output.
///
/// Returns a stream of AgentEvent for real-time updates. Tools are passed
/// directly and used as the default tool set unless `config.step_tool_provider`
/// is set (for dynamic per-step tool resolution).
pub fn run_loop_stream(
    config: AgentConfig,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
    cancellation_token: Option<RunCancellationToken>,
    state_committer: Option<Arc<dyn StateCommitter>>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    stream_runner::run_loop_stream_impl(
        config,
        tools,
        run_ctx,
        cancellation_token,
        state_committer,
    )
}

#[cfg(test)]
mod tests;
