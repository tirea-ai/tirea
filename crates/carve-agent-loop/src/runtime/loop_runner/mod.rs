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

use crate::contracts::runtime::phase::Phase;
use crate::contracts::runtime::{AgentEvent, Interaction, StreamResult, TerminationReason};
use crate::contracts::state::CheckpointReason;
use crate::contracts::state::{gen_message_id, Message, MessageMetadata};
use crate::contracts::state::{ActivityManager, AgentState};
use crate::contracts::tool::Tool;
use crate::engine::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::engine::stop_conditions::{check_stop_conditions, StopReason};
use crate::runtime::activity::ActivityHub;
#[cfg(test)]
use crate::runtime::control::AGENT_STATE_PATH;
use crate::runtime::streaming::StreamCollector;
use async_stream::stream;
use async_trait::async_trait;
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
use crate::contracts::plugin::AgentPlugin;
#[cfg(test)]
use crate::contracts::runtime::phase::StepContext;
pub use crate::runtime::run_context::{
    RunCancellationToken, RunContext, StateCommitError, StateCommitter,
    TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY, TOOL_SCOPE_CALLER_STATE_KEY,
    TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use carve_state::TrackedPatch;
pub use config::{AgentConfig, LlmRetryPolicy};
pub use config::{StaticStepToolProvider, StepToolInput, StepToolProvider, StepToolSnapshot};
#[cfg(test)]
use core::build_messages;
#[cfg(test)]
use core::set_agent_pending_interaction;
use core::{
    apply_pending_patches, build_request_for_filtered_tools, clear_agent_pending_interaction,
    drain_agent_outbox, inference_inputs_from_step, pending_interaction_from_thread,
    reduce_thread_mutations, ThreadMutationBatch,
};
pub use outcome::{run_step_cycle, tool_map, tool_map_from_arc, AgentLoopError, StepResult};
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
use stream_runner::run_loop_stream_impl_with_provider;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use tool_exec::execute_tools_parallel_with_phases;
use tool_exec::{
    apply_tool_results_impl, apply_tool_results_to_session, execute_single_tool_with_phases,
    next_step_index, scope_with_tool_caller_context, step_metadata, ToolExecutionRequest,
    ToolExecutionResult,
};
pub use tool_exec::{
    execute_tools, execute_tools_with_config, execute_tools_with_plugins,
    execute_tools_with_plugins_and_executor, ParallelToolExecutor, SequentialToolExecutor,
    ToolExecutor,
};

fn uuid_v7() -> String {
    Uuid::now_v7().simple().to_string()
}

pub(super) fn thread_state_version(thread: &AgentState) -> u64 {
    thread.metadata.version.unwrap_or(0)
}

pub fn set_thread_state_version(
    thread: &mut AgentState,
    version: u64,
    version_timestamp: Option<u64>,
) {
    thread.metadata.version = Some(version);
    if let Some(timestamp) = version_timestamp {
        thread.metadata.version_timestamp = Some(timestamp);
    }
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
    fallback_tools: &HashMap<String, Arc<dyn Tool>>,
) -> Arc<dyn StepToolProvider> {
    config.step_tool_provider.clone().unwrap_or_else(|| {
        Arc::new(StaticStepToolProvider::new(fallback_tools.clone())) as Arc<dyn StepToolProvider>
    })
}

pub(super) async fn resolve_step_tool_snapshot(
    step_tool_provider: &Arc<dyn StepToolProvider>,
    thread: &AgentState,
) -> Result<StepToolSnapshot, AgentLoopError> {
    step_tool_provider.provide(StepToolInput { thread }).await
}

fn mark_step_completed(run_state: &mut RunState) {
    run_state.completed_steps += 1;
}

fn build_loop_outcome(
    thread: AgentState,
    termination: TerminationReason,
    response: Option<String>,
    run_state: &RunState,
    failure: Option<outcome::LoopFailure>,
) -> LoopOutcome {
    LoopOutcome {
        thread,
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
    thread: &AgentState,
    stop_conditions: &[Arc<dyn crate::engine::stop_conditions::StopCondition>],
) -> Option<StopReason> {
    let stop_ctx = run_state.to_check_context(result, thread);
    check_stop_conditions(stop_conditions, &stop_ctx)
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
trait ChatProvider: Send + Sync {
    async fn exec_chat_response(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse>;
}

#[async_trait]
impl ChatProvider for Client {
    async fn exec_chat_response(
        &self,
        model: &str,
        chat_req: genai::chat::ChatRequest,
        options: Option<&ChatOptions>,
    ) -> genai::Result<genai::chat::ChatResponse> {
        self.exec_chat(model, chat_req, options).await
    }
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
    thread: &AgentState,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
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

pub(super) struct PreparedStep {
    pub(super) messages: Vec<Message>,
    pub(super) filtered_tools: Vec<String>,
    pub(super) skip_inference: bool,
    pub(super) pending_patches: Vec<TrackedPatch>,
    pub(super) tracing_span: Option<tracing::Span>,
}

pub(super) async fn prepare_step_execution(
    thread: &AgentState,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    config: &AgentConfig,
) -> Result<PreparedStep, AgentLoopError> {
    let (messages, filtered_tools, skip_inference, tracing_span, pending) =
        run_step_prepare_phases(thread, tool_descriptors, config).await?;
    Ok(PreparedStep {
        messages,
        filtered_tools,
        skip_inference,
        pending_patches: pending,
        tracing_span,
    })
}

pub(super) async fn apply_llm_error_cleanup(
    thread: &mut AgentState,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
    error_type: &'static str,
    message: String,
) -> Result<(), AgentLoopError> {
    *thread = emit_cleanup_phases_and_apply(
        thread.clone(),
        tool_descriptors,
        plugins,
        error_type,
        message,
    )
    .await?;
    Ok(())
}

pub(super) async fn complete_step_after_inference(
    thread: &mut AgentState,
    result: &StreamResult,
    step_meta: MessageMetadata,
    assistant_message_id: Option<String>,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
) -> Result<(), AgentLoopError> {
    let pending = emit_phase_block(
        Phase::AfterInference,
        thread,
        tool_descriptors,
        plugins,
        |step| {
            step.response = Some(result.clone());
        },
    )
    .await?;
    let thread_after_after_inference = apply_pending_patches(thread.clone(), pending);

    let assistant = assistant_turn_message(result, step_meta, assistant_message_id);
    let thread_after_message = reduce_thread_mutations(
        thread_after_after_inference,
        ThreadMutationBatch::default().with_message(assistant),
    );

    let pending = emit_phase_block(
        Phase::StepEnd,
        &thread_after_message,
        tool_descriptors,
        plugins,
        |_| {},
    )
    .await?;
    *thread = apply_pending_patches(thread_after_message, pending);
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

pub(super) struct ToolExecutionContext {
    pub(super) state: serde_json::Value,
    pub(super) scope: carve_state::ScopeState,
}

pub(super) fn prepare_tool_execution_context(
    thread: &AgentState,
    config: Option<&AgentConfig>,
) -> Result<ToolExecutionContext, AgentLoopError> {
    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let scope = scope_with_tool_caller_context(thread, &state, config)?;
    Ok(ToolExecutionContext { state, scope })
}

pub(super) async fn finalize_run_end(
    thread: AgentState,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    plugins: &[Arc<dyn crate::contracts::plugin::AgentPlugin>],
) -> AgentState {
    emit_run_end_phase(thread, tool_descriptors, plugins).await
}

fn stream_result_from_chat_response(response: &genai::chat::ChatResponse) -> StreamResult {
    let text = response
        .first_text()
        .map(|s| s.to_string())
        .unwrap_or_default();
    let tool_calls: Vec<crate::contracts::state::ToolCall> = response
        .tool_calls()
        .into_iter()
        .map(|tc| {
            crate::contracts::state::ToolCall::new(
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
    thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(AgentState, StreamResult), AgentLoopError> {
    run_step_with_provider(client, config, thread, tools).await
}

async fn run_step_with_provider(
    provider: &dyn ChatProvider,
    config: &AgentConfig,
    thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(AgentState, StreamResult), AgentLoopError> {
    let step_tool_provider = step_tool_provider_for_run(config, tools);
    let mut thread = thread;
    let step_tools = resolve_step_tool_snapshot(&step_tool_provider, &thread).await?;
    let tool_descriptors = step_tools.descriptors.clone();
    let prepared = prepare_step_execution(&thread, &tool_descriptors, config).await?;
    thread = apply_pending_patches(thread, prepared.pending_patches);
    let messages = prepared.messages;
    let filtered_tools = prepared.filtered_tools;

    // Skip inference if requested
    if prepared.skip_inference {
        if let Some(interaction) = pending_interaction_from_thread(&thread) {
            return Err(AgentLoopError::PendingInteraction {
                thread: Box::new(thread),
                interaction: Box::new(interaction),
            });
        }
        return Ok((
            thread,
            StreamResult {
                text: String::new(),
                tool_calls: vec![],
                usage: None,
            },
        ));
    }

    // Call LLM with unified retry + fallback model strategy.
    let inference_span = prepared.tracing_span.unwrap_or_else(tracing::Span::none);
    let attempt_outcome =
        run_llm_with_retry_and_fallback(config, None, true, "unknown llm error", |model| {
            let request =
                build_request_for_filtered_tools(&messages, &step_tools.tools, &filtered_tools);
            async move {
                provider
                    .exec_chat_response(&model, request, config.chat_options.as_ref())
                    .await
            }
            .instrument(inference_span.clone())
        })
        .await;
    let response = match attempt_outcome {
        LlmAttemptOutcome::Success { value, .. } => value,
        LlmAttemptOutcome::Cancelled => {
            return Err(AgentLoopError::Cancelled {
                thread: Box::new(thread),
            });
        }
        LlmAttemptOutcome::Exhausted { last_error, .. } => {
            apply_llm_error_cleanup(
                &mut thread,
                &tool_descriptors,
                &config.plugins,
                "llm_exec_error",
                last_error.clone(),
            )
            .await?;
            return Err(AgentLoopError::LlmError(last_error));
        }
    };

    let result = stream_result_from_chat_response(&response);

    // Add assistant message
    let step_meta = step_metadata(
        thread
            .scope
            .value("run_id")
            .and_then(|v| v.as_str().map(String::from)),
        next_step_index(&thread),
    );
    complete_step_after_inference(
        &mut thread,
        &result,
        step_meta,
        None,
        &tool_descriptors,
        &config.plugins,
    )
    .await?;

    Ok((thread, result))
}

/// Run the full agent loop until completion or a stop condition is met.
///
/// Returns the final thread and the last response text.
pub async fn run_loop(
    client: &Client,
    config: &AgentConfig,
    thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(AgentState, String), AgentLoopError> {
    run_loop_with_context(client, config, thread, tools, RunContext::default()).await
}

/// Run the full agent loop with explicit run context.
///
/// This is the non-streaming counterpart of `run_loop_stream(..., run_ctx)`,
/// allowing cooperative cancellation via `run_ctx.cancellation_token`.
pub async fn run_loop_with_context(
    client: &Client,
    config: &AgentConfig,
    thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Result<(AgentState, String), AgentLoopError> {
    run_loop_with_context_provider(client, config, thread, tools, run_ctx).await
}

fn legacy_result_from_outcome(
    outcome: LoopOutcome,
) -> Result<(AgentState, String), AgentLoopError> {
    let LoopOutcome {
        thread,
        termination,
        response,
        failure,
        ..
    } = outcome;

    match termination {
        TerminationReason::NaturalEnd | TerminationReason::PluginRequested => {
            Ok((thread, response.unwrap_or_default()))
        }
        TerminationReason::Stopped(reason) => Err(AgentLoopError::Stopped {
            thread: Box::new(thread),
            reason,
        }),
        TerminationReason::Cancelled => Err(AgentLoopError::Cancelled {
            thread: Box::new(thread),
        }),
        TerminationReason::PendingInteraction => {
            if let Some(interaction) = pending_interaction_from_thread(&thread) {
                Err(AgentLoopError::PendingInteraction {
                    thread: Box::new(thread),
                    interaction: Box::new(interaction),
                })
            } else {
                Err(AgentLoopError::StateError(
                    "pending interaction termination without pending interaction state".to_string(),
                ))
            }
        }
        TerminationReason::Error => match failure {
            Some(outcome::LoopFailure::Llm(message)) => Err(AgentLoopError::LlmError(message)),
            Some(outcome::LoopFailure::State(message)) => Err(AgentLoopError::StateError(message)),
            None => Err(AgentLoopError::StateError(
                "loop terminated with error".to_string(),
            )),
        },
    }
}

async fn run_loop_with_context_provider(
    provider: &dyn ChatProvider,
    config: &AgentConfig,
    thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Result<(AgentState, String), AgentLoopError> {
    let outcome =
        run_loop_outcome_with_context_provider(provider, config, thread, tools, run_ctx).await;
    legacy_result_from_outcome(outcome)
}

async fn run_loop_outcome_with_context_provider(
    provider: &dyn ChatProvider,
    config: &AgentConfig,
    mut thread: AgentState,
    tools: &HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> LoopOutcome {
    let mut run_state = RunState::new();
    let stop_conditions = effective_stop_conditions(config);
    let run_cancellation_token = run_ctx.run_cancellation_token().cloned();
    let mut last_text = String::new();
    let step_tool_provider = step_tool_provider_for_run(config, tools);
    let run_id = thread
        .scope
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            let id = uuid_v7();
            let _ = thread.scope.set("run_id", &id);
            id
        });
    let initial_step_tools = match resolve_step_tool_snapshot(&step_tool_provider, &thread).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return build_loop_outcome(
                thread,
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
            thread = finalize_run_end(thread, &active_tool_descriptors, &config.plugins).await;
            return build_loop_outcome(thread, $termination, $response, &run_state, $failure);
        }};
    }

    // Phase: RunStart
    let pending = match emit_phase_block(
        Phase::RunStart,
        &thread,
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
    thread = apply_pending_patches(thread, pending);

    loop {
        if is_run_cancelled(run_cancellation_token.as_ref()) {
            terminate_run!(TerminationReason::Cancelled, None, None);
        }

        let step_tools = match resolve_step_tool_snapshot(&step_tool_provider, &thread).await {
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

        let prepared = match prepare_step_execution(&thread, &active_tool_descriptors, config).await
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
        thread = apply_pending_patches(thread, prepared.pending_patches);

        if prepared.skip_inference {
            if pending_interaction_from_thread(&thread).is_some() {
                terminate_run!(TerminationReason::PendingInteraction, None, None);
            }
            terminate_run!(
                TerminationReason::PluginRequested,
                Some(last_text.clone()),
                None
            );
        }

        // Call LLM with unified retry + fallback model strategy.
        let inference_span = prepared.tracing_span.unwrap_or_else(tracing::Span::none);
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
                async move {
                    provider
                        .exec_chat_response(&model, request, config.chat_options.as_ref())
                        .await
                }
                .instrument(inference_span.clone())
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
                // Ensure AfterInference and StepEnd run so plugins can observe the error and clean up.
                if let Err(phase_error) = apply_llm_error_cleanup(
                    &mut thread,
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
            &mut thread,
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
            if let Some(reason) =
                stop_reason_for_step(&run_state, &result, &thread, &stop_conditions)
            {
                terminate_run!(TerminationReason::Stopped(reason), None, None);
            }
            terminate_run!(TerminationReason::NaturalEnd, Some(last_text.clone()), None);
        }

        // Execute tools with phase hooks using configured execution strategy.
        let tool_context = match prepare_tool_execution_context(&thread, Some(config)) {
            Ok(ctx) => ctx,
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };
        let thread_messages_for_tools = thread.messages.clone();
        let thread_version_for_tools = thread_state_version(&thread);

        let tool_exec_future = config.tool_executor.execute(ToolExecutionRequest {
            tools: &step_tools.tools,
            calls: &result.tool_calls,
            state: &tool_context.state,
            tool_descriptors: &active_tool_descriptors,
            plugins: &config.plugins,
            activity_manager: None,
            scope: Some(&tool_context.scope),
            thread_id: &thread.id,
            thread_messages: &thread_messages_for_tools,
            state_version: thread_version_for_tools,
        });
        let results = if let Some(ref token) = run_cancellation_token {
            tokio::select! {
                _ = token.cancelled() => {
                    terminate_run!(TerminationReason::Cancelled, None, None);
                }
                results = tool_exec_future => results,
            }
        } else {
            tool_exec_future.await
        };

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };

        let thread_before_apply = thread.clone();
        let applied = match apply_tool_results_to_session(
            thread,
            &results,
            Some(step_meta),
            config
                .tool_executor
                .requires_parallel_patch_conflict_check(),
        ) {
            Ok(a) => a,
            Err(e) => {
                thread = thread_before_apply;
                terminate_run!(
                    TerminationReason::Error,
                    None,
                    Some(outcome::LoopFailure::State(e.to_string()))
                );
            }
        };
        thread = applied.thread;

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
        if let Some(reason) = stop_reason_for_step(&run_state, &result, &thread, &stop_conditions) {
            terminate_run!(TerminationReason::Stopped(reason), None, None);
        }
    }

}

/// Run the agent loop with streaming output.
///
/// Returns a stream of AgentEvent for real-time updates.
pub fn run_loop_stream(
    client: Client,
    config: AgentConfig,
    thread: AgentState,
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
