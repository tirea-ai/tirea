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
//! Each phase dispatches to its typed plugin hook:
//!
//! ```text
//! RunStart (once)
//!     │
//!     ▼
//! ┌─────────────────────────┐
//! │      StepStart          │ ← plugins can apply state patches
//! ├─────────────────────────┤
//! │    BeforeInference      │ ← plugins can inject prompt context, filter tools
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
use crate::contracts::runtime::state_paths::PERMISSIONS_STATE_PATH;
use crate::contracts::runtime::ActivityManager;
use crate::contracts::runtime::{
    StopPolicy, StreamResult, ToolExecutionRequest, ToolExecutionResult,
};
use crate::contracts::thread::CheckpointReason;
use crate::contracts::thread::{gen_message_id, Message, MessageMetadata, ToolCall};
use crate::contracts::tool::Tool;
use crate::contracts::RunContext;
use crate::contracts::StopReason;
use crate::contracts::{
    AgentEvent, FrontendToolInvocation, Interaction, InteractionResponse, SuspendedCall,
    TerminationReason,
};
use crate::engine::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::engine::stop_conditions::check_stop_policies;
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
pub use crate::contracts::runtime::{LlmExecutor, ToolExecutor};
pub use crate::runtime::run_context::{
    await_or_cancel, is_cancelled, CancelAware, RunCancellationToken, StateCommitError,
    StateCommitter, TOOL_SCOPE_CALLER_AGENT_ID_KEY, TOOL_SCOPE_CALLER_MESSAGES_KEY,
    TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use config::StaticStepToolProvider;
pub use config::{AgentConfig, GenaiLlmExecutor, LlmRetryPolicy};
pub use config::{StepToolInput, StepToolProvider, StepToolSnapshot};
#[cfg(test)]
use core::build_messages;
#[cfg(test)]
use core::set_agent_pending_interaction;
use core::{
    build_request_for_filtered_tools, clear_all_suspended_calls, clear_suspended_call,
    drain_agent_outbox, enqueue_interaction_resolution, inference_inputs_from_step,
    set_agent_suspended_calls, suspended_calls_from_ctx,
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
use state_commit::PendingDeltaCommitContext;
use std::time::{SystemTime, UNIX_EPOCH};
use tirea_state::TrackedPatch;
#[cfg(test)]
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use tool_exec::execute_tools_parallel_with_phases;
use tool_exec::{
    apply_tool_results_impl, apply_tool_results_to_session, execute_single_tool_with_phases,
    scope_with_tool_caller_context, step_metadata, ToolPhaseContext,
};
pub use tool_exec::{
    execute_tools, execute_tools_with_config, execute_tools_with_plugins,
    execute_tools_with_plugins_and_executor, ParallelToolExecutor, SequentialToolExecutor,
};

/// Fully resolved agent wiring ready for execution.
///
/// Contains everything needed to run an agent loop: the loop configuration,
/// the resolved tool map, and the runtime config. This is a pure data struct
/// that can be inspected, mutated, and tested independently.
pub struct ResolvedRun {
    /// Loop configuration (model, plugins, stop conditions, ...).
    pub config: AgentConfig,
    /// Resolved tool map after filtering and wiring.
    pub tools: HashMap<String, Arc<dyn Tool>>,
    /// Runtime configuration (user_id, run_id, ...).
    pub run_config: crate::contracts::RunConfig,
}

impl ResolvedRun {
    /// Add or replace a tool in the resolved tool map.
    #[must_use]
    pub fn with_tool(mut self, id: String, tool: Arc<dyn Tool>) -> Self {
        self.tools.insert(id, tool);
        self
    }

    /// Add a plugin to the resolved config.
    #[must_use]
    pub fn with_plugin(mut self, plugin: Arc<dyn crate::contracts::plugin::AgentPlugin>) -> Self {
        self.config.plugins.push(plugin);
        self
    }

    /// Overlay tools from a tool registry (insert-if-absent semantics).
    pub fn overlay_tool_registry(&mut self, registry: &dyn crate::contracts::ToolRegistry) {
        for (id, tool) in registry.snapshot() {
            self.tools.entry(id).or_insert(tool);
        }
    }
}

fn uuid_v7() -> String {
    Uuid::now_v7().simple().to_string()
}

pub(crate) fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CancellationStage {
    Inference,
    ToolExecution,
}

pub(super) const CANCELLATION_INFERENCE_USER_MESSAGE: &str =
    "The previous run was interrupted during inference. Please continue from the current context.";
pub(super) const CANCELLATION_TOOL_USER_MESSAGE: &str =
    "The previous run was interrupted while using tools. Please continue from the current context.";

pub(super) fn append_cancellation_user_message(run_ctx: &mut RunContext, stage: CancellationStage) {
    let content = match stage {
        CancellationStage::Inference => CANCELLATION_INFERENCE_USER_MESSAGE,
        CancellationStage::ToolExecution => CANCELLATION_TOOL_USER_MESSAGE,
    };
    run_ctx.add_message(Arc::new(Message::user(content)));
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
    match await_or_cancel(
        run_cancellation_token,
        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)),
    )
    .await
    {
        CancelAware::Cancelled => true,
        CancelAware::Value(_) => false,
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
    is_cancelled(token)
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
    step_tool_provider
        .provide(StepToolInput { state: run_ctx })
        .await
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
            let response_res =
                match await_or_cancel(run_cancellation_token, invoke(model.clone())).await {
                    CancelAware::Cancelled => return LlmAttemptOutcome::Cancelled,
                    CancelAware::Value(resp) => resp,
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
    (
        Vec<Message>,
        Vec<String>,
        bool,
        Option<TerminationReason>,
        Vec<TrackedPatch>,
    ),
    AgentLoopError,
> {
    let ((messages, filtered_tools, skip_inference, termination_request), pending) =
        run_phase_block(
            run_ctx,
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
        termination_request,
        pending,
    ))
}

pub(super) struct PreparedStep {
    pub(super) messages: Vec<Message>,
    pub(super) filtered_tools: Vec<String>,
    pub(super) skip_inference: bool,
    pub(super) termination_request: Option<TerminationReason>,
    pub(super) pending_patches: Vec<TrackedPatch>,
}

pub(super) async fn prepare_step_execution(
    run_ctx: &RunContext,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
    config: &AgentConfig,
) -> Result<PreparedStep, AgentLoopError> {
    let (messages, filtered_tools, skip_inference, termination_request, pending) =
        run_step_prepare_phases(run_ctx, tool_descriptors, config).await?;
    Ok(PreparedStep {
        messages,
        filtered_tools,
        skip_inference,
        termination_request,
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
    emit_cleanup_phases_and_apply(run_ctx, tool_descriptors, plugins, error_type, message).await
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

    let pending =
        emit_phase_block(Phase::StepEnd, run_ctx, tool_descriptors, plugins, |_| {}).await?;
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

pub(super) fn has_suspended_calls(run_ctx: &RunContext) -> bool {
    !suspended_calls_from_ctx(run_ctx).is_empty()
}

pub(super) fn suspended_call_pending_events(run_ctx: &RunContext) -> Vec<AgentEvent> {
    let mut calls: Vec<SuspendedCall> = suspended_calls_from_ctx(run_ctx).into_values().collect();
    calls.sort_by(|left, right| left.call_id.cmp(&right.call_id));
    calls
        .into_iter()
        .flat_map(|call| pending_tool_events(&call.interaction, call.frontend_invocation.as_ref()))
        .collect()
}

pub(super) struct ToolExecutionContext {
    pub(super) state: serde_json::Value,
    pub(super) run_config: tirea_contract::RunConfig,
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

struct RunStartDrainOutcome {
    events: Vec<AgentEvent>,
    replayed: bool,
}

async fn drain_run_start_outbox_and_replay(
    run_ctx: &mut RunContext,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
    tool_descriptors: &[crate::contracts::tool::ToolDescriptor],
) -> Result<RunStartDrainOutcome, AgentLoopError> {
    let outbox = drain_agent_outbox(run_ctx, "agent_outbox_run_start")?;
    let mut events = outbox
        .interaction_resolutions
        .into_iter()
        .map(|resolution| AgentEvent::InteractionResolved {
            interaction_id: resolution.interaction_id,
            result: resolution.result,
        })
        .collect::<Vec<_>>();

    let replay_calls = outbox.replay_tool_calls;
    if replay_calls.is_empty() {
        return Ok(RunStartDrainOutcome {
            events,
            replayed: false,
        });
    }

    let mut replay_state_changed = false;
    for tool_call in &replay_calls {
        let state = run_ctx
            .snapshot()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        let tool = tools.get(&tool_call.name).cloned();
        let rt_for_replay = scope_with_tool_caller_context(run_ctx, &state, Some(config))?;
        let replay_phase_ctx = ToolPhaseContext {
            tool_descriptors,
            plugins: &config.plugins,
            activity_manager: None,
            run_config: &rt_for_replay,
            thread_id: run_ctx.thread_id(),
            thread_messages: run_ctx.messages(),
            cancellation_token: None,
        };
        let replay_result =
            execute_single_tool_with_phases(tool.as_deref(), tool_call, &state, &replay_phase_ctx)
                .await?;

        let replay_msg_id = gen_message_id();
        let replay_msg = tool_response(&tool_call.id, &replay_result.execution.result)
            .with_id(replay_msg_id.clone());
        run_ctx.add_message(Arc::new(replay_msg));

        if !replay_result.reminders.is_empty() {
            let msgs: Vec<Arc<Message>> = replay_result
                .reminders
                .iter()
                .map(|reminder| {
                    Arc::new(Message::internal_system(format!(
                        "<system-reminder>{}</system-reminder>",
                        reminder
                    )))
                })
                .collect();
            run_ctx.add_messages(msgs);
        }

        if let Some(patch) = replay_result.execution.patch.clone() {
            replay_state_changed = true;
            run_ctx.add_thread_patch(patch);
        }
        if !replay_result.pending_patches.is_empty() {
            replay_state_changed = true;
            run_ctx.add_thread_patches(replay_result.pending_patches.clone());
        }

        events.push(AgentEvent::ToolCallDone {
            id: tool_call.id.clone(),
            result: replay_result.execution.result,
            patch: replay_result.execution.patch,
            message_id: replay_msg_id,
        });

        if let Some(new_interaction) = replay_result.pending_interaction {
            let new_frontend_invocation = replay_result.pending_frontend_invocation.clone();
            let state = run_ctx
                .snapshot()
                .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
            let call_id = new_frontend_invocation
                .as_ref()
                .map(|inv| inv.call_id.clone())
                .unwrap_or_else(|| new_interaction.id.clone());
            let tool_name = new_frontend_invocation
                .as_ref()
                .map(|inv| inv.tool_name.clone())
                .unwrap_or_default();
            let patch = set_agent_suspended_calls(
                &state,
                vec![SuspendedCall {
                    call_id,
                    tool_name,
                    interaction: new_interaction.clone(),
                    frontend_invocation: new_frontend_invocation.clone(),
                }],
            )?;
            if !patch.patch().is_empty() {
                run_ctx.add_thread_patch(patch);
            }
            for event in pending_tool_events(&new_interaction, new_frontend_invocation.as_ref()) {
                events.push(event);
            }
            let snapshot = run_ctx
                .snapshot()
                .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
            events.push(AgentEvent::StateSnapshot { snapshot });
            return Ok(RunStartDrainOutcome {
                events,
                replayed: true,
            });
        }
    }

    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let clear_patch = clear_all_suspended_calls(&state)?;
    if !clear_patch.patch().is_empty() {
        replay_state_changed = true;
        run_ctx.add_thread_patch(clear_patch);
    }

    if replay_state_changed {
        let snapshot = run_ctx
            .snapshot()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        events.push(AgentEvent::StateSnapshot { snapshot });
    }

    Ok(RunStartDrainOutcome {
        events,
        replayed: true,
    })
}

fn normalize_frontend_tool_result(
    response: &serde_json::Value,
    fallback_arguments: &serde_json::Value,
) -> serde_json::Value {
    match response {
        serde_json::Value::Bool(_) => fallback_arguments.clone(),
        value => value.clone(),
    }
}

fn find_tool_call_in_messages(run_ctx: &RunContext, call_id: &str) -> Option<ToolCall> {
    run_ctx.messages().iter().rev().find_map(|message| {
        message
            .tool_calls
            .as_ref()
            .and_then(|calls| calls.iter().find(|call| call.id == call_id).cloned())
    })
}

fn replay_tool_call_for_resolution(
    run_ctx: &RunContext,
    suspended_call: &SuspendedCall,
    response: &InteractionResponse,
) -> Option<ToolCall> {
    if InteractionResponse::is_denied(&response.result) {
        return None;
    }

    if let Some(invocation) = suspended_call.frontend_invocation.as_ref() {
        match invocation.routing {
            crate::contracts::ResponseRouting::ReplayOriginalTool => match &invocation.origin {
                crate::contracts::InvocationOrigin::ToolCallIntercepted {
                    backend_call_id,
                    backend_tool_name,
                    backend_arguments,
                } => {
                    return Some(ToolCall::new(
                        backend_call_id.clone(),
                        backend_tool_name.clone(),
                        backend_arguments.clone(),
                    ));
                }
                crate::contracts::InvocationOrigin::PluginInitiated { .. } => {
                    return Some(ToolCall::new(
                        invocation.call_id.clone(),
                        invocation.tool_name.clone(),
                        invocation.arguments.clone(),
                    ));
                }
            },
            crate::contracts::ResponseRouting::UseAsToolResult
            | crate::contracts::ResponseRouting::PassToLLM => {
                return Some(ToolCall::new(
                    invocation.call_id.clone(),
                    invocation.tool_name.clone(),
                    normalize_frontend_tool_result(&response.result, &invocation.arguments),
                ));
            }
        }
    }

    if !InteractionResponse::is_approved(&response.result) {
        return None;
    }

    find_tool_call_in_messages(run_ctx, &suspended_call.call_id).or_else(|| {
        if suspended_call.tool_name.is_empty() {
            None
        } else {
            Some(ToolCall::new(
                suspended_call.call_id.clone(),
                suspended_call.tool_name.clone(),
                suspended_call.interaction.parameters.clone(),
            ))
        }
    })
}

fn one_shot_approval_call_id(
    suspended_call: &SuspendedCall,
    response: &InteractionResponse,
) -> Option<String> {
    if !InteractionResponse::is_approved(&response.result) {
        return None;
    }

    let invocation = suspended_call.frontend_invocation.as_ref()?;
    if !matches!(
        invocation.routing,
        crate::contracts::ResponseRouting::ReplayOriginalTool
    ) {
        return None;
    }

    match &invocation.origin {
        crate::contracts::InvocationOrigin::ToolCallIntercepted {
            backend_call_id, ..
        } => Some(backend_call_id.clone()),
        crate::contracts::InvocationOrigin::PluginInitiated { .. } => None,
    }
}

fn set_permission_one_shot_approval(
    run_ctx: &mut RunContext,
    approved_call_id: &str,
) -> Result<(), AgentLoopError> {
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let mut approved_calls = match state
        .get(PERMISSIONS_STATE_PATH)
        .and_then(|permissions| permissions.get("approved_calls"))
        .cloned()
    {
        Some(raw) => serde_json::from_value::<HashMap<String, bool>>(raw).map_err(|e| {
            AgentLoopError::StateError(format!("failed to parse permissions.approved_calls: {e}"))
        })?,
        None => HashMap::new(),
    };
    approved_calls.insert(approved_call_id.to_string(), true);
    let patch = tirea_state::Patch::new().with_op(tirea_state::Op::set(
        tirea_state::Path::root()
            .key(PERMISSIONS_STATE_PATH)
            .key("approved_calls"),
        serde_json::to_value(approved_calls).map_err(|e| {
            AgentLoopError::StateError(format!(
                "failed to serialize permissions.approved_calls: {e}"
            ))
        })?,
    ));
    if !patch.is_empty() {
        run_ctx.add_thread_patch(TrackedPatch::new(patch).with_source("agent_loop"));
    }
    Ok(())
}

pub(super) fn resolve_suspended_call(
    run_ctx: &mut RunContext,
    response: &InteractionResponse,
) -> Result<bool, AgentLoopError> {
    let suspended_calls = suspended_calls_from_ctx(run_ctx);
    if suspended_calls.is_empty() {
        return Ok(false);
    }

    let suspended_call = suspended_calls
        .get(&response.interaction_id)
        .cloned()
        .or_else(|| {
            suspended_calls
                .values()
                .find(|call| call.interaction.id == response.interaction_id)
                .cloned()
        });
    let Some(suspended_call) = suspended_call else {
        return Ok(false);
    };

    if let Some(approved_call_id) = one_shot_approval_call_id(&suspended_call, response) {
        set_permission_one_shot_approval(run_ctx, &approved_call_id)?;
    }

    let replay_call = replay_tool_call_for_resolution(run_ctx, &suspended_call, response);
    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let outbox_patch = enqueue_interaction_resolution(&state, response.clone(), replay_call)?;
    if !outbox_patch.patch().is_empty() {
        run_ctx.add_thread_patch(outbox_patch);
    }

    let state = run_ctx
        .snapshot()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let clear_patch = clear_suspended_call(&state, &suspended_call.call_id)?;
    if !clear_patch.patch().is_empty() {
        run_ctx.add_thread_patch(clear_patch);
    }

    Ok(true)
}

pub(super) fn drain_decision_channel(
    run_ctx: &mut RunContext,
    decision_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<InteractionResponse>>,
) -> Result<bool, AgentLoopError> {
    let Some(rx) = decision_rx.as_mut() else {
        return Ok(false);
    };

    let mut resolved_any = false;
    loop {
        match rx.try_recv() {
            Ok(response) => {
                if resolve_suspended_call(run_ctx, &response)? {
                    resolved_any = true;
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            | Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
        }
    }
    Ok(resolved_any)
}

async fn apply_decisions_and_replay(
    run_ctx: &mut RunContext,
    decision_rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<InteractionResponse>>,
    step_tool_provider: &Arc<dyn StepToolProvider>,
    config: &AgentConfig,
    active_tool_descriptors: &mut Vec<crate::contracts::tool::ToolDescriptor>,
    pending_delta_commit: &PendingDeltaCommitContext<'_>,
) -> Result<Vec<AgentEvent>, AgentLoopError> {
    let decisions_applied = drain_decision_channel(run_ctx, decision_rx)?;
    if !decisions_applied {
        return Ok(Vec::new());
    }

    let decision_tools = resolve_step_tool_snapshot(step_tool_provider, run_ctx).await?;
    *active_tool_descriptors = decision_tools.descriptors.clone();

    let decision_drain = drain_run_start_outbox_and_replay(
        run_ctx,
        &decision_tools.tools,
        config,
        active_tool_descriptors,
    )
    .await?;

    pending_delta_commit
        .commit(run_ctx, CheckpointReason::ToolResultsCommitted, false)
        .await?;

    Ok(decision_drain.events)
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
    state_committer: Option<Arc<dyn StateCommitter>>,
    mut decision_rx: Option<tokio::sync::mpsc::UnboundedReceiver<InteractionResponse>>,
) -> LoopOutcome {
    let executor = llm_executor_for_run(config);
    let mut run_state = RunState::new();
    let stop_conditions = effective_stop_conditions(config);
    let run_cancellation_token = cancellation_token;
    let mut last_text = String::new();
    let step_tool_provider = step_tool_provider_for_run(config, tools);
    let run_identity = stream_core::resolve_stream_run_identity(&mut run_ctx);
    let run_id = run_identity.run_id;
    let parent_run_id = run_identity.parent_run_id;
    let pending_delta_commit =
        PendingDeltaCommitContext::new(&run_id, parent_run_id.as_deref(), state_committer.as_ref());
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
    let StepToolSnapshot {
        tools: initial_tools,
        descriptors: initial_descriptors,
    } = initial_step_tools;
    let mut active_tool_descriptors = initial_descriptors;

    macro_rules! terminate_run {
        ($termination:expr, $response:expr, $failure:expr) => {{
            let reason: TerminationReason = $termination;
            // When suspended calls exist, unresolved external input should keep
            // the run in PendingInteraction regardless of plugin termination hint.
            let final_termination = if !matches!(
                reason,
                TerminationReason::Error | TerminationReason::Cancelled
            ) && has_suspended_calls(&run_ctx)
            {
                TerminationReason::PendingInteraction
            } else {
                reason
            };
            let final_response = if final_termination == TerminationReason::PendingInteraction {
                None
            } else {
                $response
            };
            finalize_run_end(&mut run_ctx, &active_tool_descriptors, &config.plugins).await;
            if let Err(error) = pending_delta_commit
                .commit(&mut run_ctx, CheckpointReason::RunFinished, true)
                .await
            {
                return build_loop_outcome(
                    run_ctx,
                    TerminationReason::Error,
                    None,
                    &run_state,
                    Some(outcome::LoopFailure::State(error.to_string())),
                );
            }
            return build_loop_outcome(
                run_ctx,
                final_termination,
                final_response,
                &run_state,
                $failure,
            );
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
    if let Err(error) = pending_delta_commit
        .commit(&mut run_ctx, CheckpointReason::UserMessage, false)
        .await
    {
        terminate_run!(
            TerminationReason::Error,
            None,
            Some(outcome::LoopFailure::State(error.to_string()))
        );
    }

    let run_start_drain = match drain_run_start_outbox_and_replay(
        &mut run_ctx,
        &initial_tools,
        config,
        &active_tool_descriptors,
    )
    .await
    {
        Ok(replayed) => replayed,
        Err(error) => {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
            );
        }
    };
    if run_start_drain.replayed {
        if let Err(error) = pending_delta_commit
            .commit(&mut run_ctx, CheckpointReason::ToolResultsCommitted, false)
            .await
        {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
            );
        }
    }
    loop {
        if let Err(error) = apply_decisions_and_replay(
            &mut run_ctx,
            &mut decision_rx,
            &step_tool_provider,
            config,
            &mut active_tool_descriptors,
            &pending_delta_commit,
        )
        .await
        {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
            );
        }

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

        let prepared =
            match prepare_step_execution(&run_ctx, &active_tool_descriptors, config).await {
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

        if let Some(reason) = prepared.termination_request {
            terminate_run!(reason, None, None);
        }
        if prepared.skip_inference {
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
                append_cancellation_user_message(&mut run_ctx, CancellationStage::Inference);
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
        if let Err(error) = pending_delta_commit
            .commit(
                &mut run_ctx,
                CheckpointReason::AssistantTurnCommitted,
                false,
            )
            .await
        {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
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
            run_config: &tool_context.run_config,
            thread_id: run_ctx.thread_id(),
            thread_messages: &thread_messages_for_tools,
            state_version: thread_version_for_tools,
            cancellation_token: run_cancellation_token.as_ref(),
        });
        let results = tool_exec_future.await.map_err(AgentLoopError::from);

        let results = match results {
            Ok(r) => r,
            Err(AgentLoopError::Cancelled { .. }) => {
                append_cancellation_user_message(&mut run_ctx, CancellationStage::ToolExecution);
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
        if let Err(error) = pending_delta_commit
            .commit(&mut run_ctx, CheckpointReason::ToolResultsCommitted, false)
            .await
        {
            terminate_run!(
                TerminationReason::Error,
                None,
                Some(outcome::LoopFailure::State(error.to_string()))
            );
        }

        // If ALL tools are suspended (no completed results), terminate immediately.
        if !applied.suspended_calls.is_empty() {
            let has_completed = results.iter().any(|r| r.pending_interaction.is_none());
            if !has_completed {
                terminate_run!(TerminationReason::PendingInteraction, None, None);
            }
        }

        // Track tool-step metrics for post-tool stop condition evaluation.
        let error_count = results
            .iter()
            .filter(|r| r.execution.result.is_error())
            .count();
        run_state.record_tool_step(&result.tool_calls, error_count);

        // Check stop conditions.
        if let Some(reason) = stop_reason_for_step(&run_state, &result, &run_ctx, &stop_conditions)
        {
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
    decision_rx: Option<tokio::sync::mpsc::UnboundedReceiver<InteractionResponse>>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    stream_runner::run_stream(
        config,
        tools,
        run_ctx,
        cancellation_token,
        state_committer,
        decision_rx,
    )
}

#[cfg(test)]
mod tests;
