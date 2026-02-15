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

mod config;
mod core;
mod outcome;
mod plugin_runtime;

use crate::activity::ActivityHub;
use crate::convert::{assistant_message, assistant_tool_calls, tool_response};
use crate::execute::{collect_patches, ToolExecution};
use crate::phase::{Phase, StepContext, ToolContext};
use crate::plugin::AgentPlugin;
use crate::state_types::{Interaction, AGENT_STATE_PATH};
use crate::stop::{check_stop_conditions, StopCheckContext, StopCondition, StopReason};
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::thread::Thread;
use crate::thread_store::{CheckpointReason, ThreadDelta};
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::types::{gen_message_id, Message, MessageMetadata};
use async_stream::stream;
use async_trait::async_trait;
use carve_state::{ActivityManager, Context, PatchExt, TrackedPatch};
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tracing::Instrument;
use uuid::Uuid;

pub use config::{
    AgentConfig, AgentDefinition, RunCancellationToken, RunContext, StateCommitError,
    StateCommitter,
};
pub(crate) use config::{
    TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
#[cfg(test)]
use core::build_messages;
use core::{
    apply_pending_patches, build_request_for_filtered_tools, clear_agent_pending_interaction,
    drain_agent_append_user_messages, drain_agent_outbox, inference_inputs_from_step,
    reduce_thread_mutations, set_agent_pending_interaction, tool_descriptors_for_config,
    ThreadMutationBatch,
};
pub use outcome::{run_round, tool_map, tool_map_from_arc, AgentLoopError, RoundResult};
use plugin_runtime::{
    emit_cleanup_phases_and_apply, emit_phase_block, emit_phase_checked, emit_session_end,
    prepare_stream_error_termination, run_phase_block,
};
#[cfg(test)]
use tokio_util::sync::CancellationToken;

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

#[derive(Clone)]
pub struct ChannelStateCommitter {
    tx: tokio::sync::mpsc::UnboundedSender<ThreadDelta>,
}

impl ChannelStateCommitter {
    pub fn new(tx: tokio::sync::mpsc::UnboundedSender<ThreadDelta>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl StateCommitter for ChannelStateCommitter {
    async fn commit(&self, _thread_id: &str, delta: ThreadDelta) -> Result<(), StateCommitError> {
        self.tx
            .send(delta)
            .map_err(|e| StateCommitError::new(format!("channel state commit failed: {e}")))
    }
}

async fn commit_pending_delta(
    thread: &mut Thread,
    reason: CheckpointReason,
    force: bool,
    run_id: &str,
    parent_run_id: Option<&str>,
    state_committer: Option<&Arc<dyn StateCommitter>>,
) -> Result<(), AgentLoopError> {
    let Some(committer) = state_committer else {
        return Ok(());
    };

    let pending = thread.take_pending();
    if !force && pending.is_empty() {
        return Ok(());
    }

    let delta = ThreadDelta {
        run_id: run_id.to_string(),
        parent_run_id: parent_run_id.map(str::to_string),
        reason,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    committer
        .commit(&thread.id, delta)
        .await
        .map_err(|e| AgentLoopError::StateError(format!("state commit failed: {e}")))
}

struct AppliedToolResults {
    thread: Thread,
    pending_interaction: Option<Interaction>,
    state_snapshot: Option<Value>,
}

fn validate_parallel_state_patch_conflicts(
    results: &[ToolExecutionResult],
) -> Result<(), AgentLoopError> {
    for (left_idx, left) in results.iter().enumerate() {
        let mut left_patches: Vec<&TrackedPatch> = Vec::new();
        if let Some(ref patch) = left.execution.patch {
            left_patches.push(patch);
        }
        left_patches.extend(left.pending_patches.iter());

        if left_patches.is_empty() {
            continue;
        }

        for right in results.iter().skip(left_idx + 1) {
            let mut right_patches: Vec<&TrackedPatch> = Vec::new();
            if let Some(ref patch) = right.execution.patch {
                right_patches.push(patch);
            }
            right_patches.extend(right.pending_patches.iter());

            if right_patches.is_empty() {
                continue;
            }

            for left_patch in &left_patches {
                for right_patch in &right_patches {
                    let conflicts = left_patch.patch().conflicts_with(right_patch.patch());
                    if let Some(conflict) = conflicts.first() {
                        return Err(AgentLoopError::StateError(format!(
                            "conflicting parallel state patches between '{}' and '{}' at {}",
                            left.execution.call.id, right.execution.call.id, conflict.path
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

fn apply_tool_results_to_session(
    thread: Thread,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    parallel_tools: bool,
) -> Result<AppliedToolResults, AgentLoopError> {
    apply_tool_results_impl(thread, results, metadata, parallel_tools, None)
}

fn apply_tool_results_impl(
    thread: Thread,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    parallel_tools: bool,
    tool_msg_ids: Option<&HashMap<String, String>>,
) -> Result<AppliedToolResults, AgentLoopError> {
    let mut pending_interactions = results
        .iter()
        .filter_map(|r| r.pending_interaction.clone())
        .collect::<Vec<_>>();

    if pending_interactions.len() > 1 {
        let ids = pending_interactions
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(AgentLoopError::StateError(format!(
            "multiple pending interactions in one tool round: [{ids}]"
        )));
    }

    let pending_interaction = pending_interactions.pop();

    if parallel_tools {
        validate_parallel_state_patch_conflicts(results)?;
    }

    // Collect patches from completed tools and plugin pending patches.
    let mut mutations = ThreadMutationBatch::default().with_patches(collect_patches(
        &results
            .iter()
            .map(|r| r.execution.clone())
            .collect::<Vec<_>>(),
    ));
    for r in results {
        mutations = mutations.with_patches(r.pending_patches.iter().cloned());
    }
    let mut state_changed = !mutations.patches.is_empty();

    // Add tool result messages for all executions.
    // Pending tools get a placeholder result so the message sequence stays valid
    // for LLMs that require tool results after every assistant tool_calls message.
    let tool_messages: Vec<Message> = results
        .iter()
        .flat_map(|r| {
            let mut msgs = if r.pending_interaction.is_some() {
                // Placeholder result keeps the message sequence valid while awaiting approval.
                vec![Message::tool(
                    &r.execution.call.id,
                    format!(
                        "Tool '{}' is awaiting approval. Execution paused.",
                        r.execution.call.name
                    ),
                )]
            } else {
                let mut tool_msg = tool_response(&r.execution.call.id, &r.execution.result);
                // Apply pre-generated message ID if provided.
                if let Some(id) = tool_msg_ids.and_then(|ids| ids.get(&r.execution.call.id)) {
                    tool_msg = tool_msg.with_id(id.clone());
                }
                vec![tool_msg]
            };
            for reminder in &r.reminders {
                msgs.push(Message::internal_system(format!(
                    "<system-reminder>{}</system-reminder>",
                    reminder
                )));
            }
            // Attach run/step metadata to each message.
            if let Some(ref meta) = metadata {
                for msg in &mut msgs {
                    msg.metadata = Some(meta.clone());
                }
            }
            msgs
        })
        .collect();

    mutations = mutations.with_messages(tool_messages);
    let mut thread = reduce_thread_mutations(thread, mutations);
    let (next_thread, appended_count) =
        drain_agent_append_user_messages(thread, results, metadata.as_ref())?;
    thread = next_thread;
    if appended_count > 0 {
        state_changed = true;
    }

    if let Some(interaction) = pending_interaction.clone() {
        let state = thread
            .rebuild_state()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        let patch = set_agent_pending_interaction(&state, interaction.clone());
        if !patch.patch().is_empty() {
            state_changed = true;
            thread =
                reduce_thread_mutations(thread, ThreadMutationBatch::default().with_patch(patch));
        }
        let state_snapshot = if state_changed {
            Some(
                thread
                    .rebuild_state()
                    .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
            )
        } else {
            None
        };
        return Ok(AppliedToolResults {
            thread,
            pending_interaction: Some(interaction),
            state_snapshot,
        });
    }

    // If a previous run left a persisted pending interaction, clear it once we successfully
    // complete tool execution without creating a new pending interaction.
    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    if state
        .get(AGENT_STATE_PATH)
        .and_then(|v| v.get("pending_interaction"))
        .is_some()
    {
        let patch = clear_agent_pending_interaction(&state);
        if !patch.patch().is_empty() {
            state_changed = true;
            thread =
                reduce_thread_mutations(thread, ThreadMutationBatch::default().with_patch(patch));
        }
    }

    let state_snapshot = if state_changed {
        Some(
            thread
                .rebuild_state()
                .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
        )
    } else {
        None
    };

    Ok(AppliedToolResults {
        thread,
        pending_interaction: None,
        state_snapshot,
    })
}

fn tool_result_metadata_from_session(thread: &Thread) -> Option<MessageMetadata> {
    let run_id = thread
        .runtime
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .or_else(|| {
            thread.messages.iter().rev().find_map(|m| {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.run_id.as_ref().cloned())
            })
        });

    let step_index = thread
        .messages
        .iter()
        .rev()
        .find_map(|m| m.metadata.as_ref().and_then(|meta| meta.step_index));

    if run_id.is_none() && step_index.is_none() {
        None
    } else {
        Some(MessageMetadata { run_id, step_index })
    }
}

fn next_step_index(thread: &Thread) -> u32 {
    thread
        .messages
        .iter()
        .filter_map(|m| m.metadata.as_ref().and_then(|meta| meta.step_index))
        .max()
        .map(|v| v.saturating_add(1))
        .unwrap_or(0)
}

fn step_metadata(run_id: Option<String>, step_index: u32) -> MessageMetadata {
    MessageMetadata {
        run_id,
        step_index: Some(step_index),
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

/// Execute tool calls (simplified version without plugins).
///
/// This is the simpler API for tests and cases where plugins aren't needed.
pub async fn execute_tools(
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<Thread, AgentLoopError> {
    execute_tools_with_plugins(thread, result, tools, parallel, &[]).await
}

/// Execute tool calls with phase-based plugin hooks.
pub async fn execute_tools_with_config(
    mut thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
) -> Result<Thread, AgentLoopError> {
    crate::tool_filter::set_runtime_filters_from_definition_if_absent(&mut thread.runtime, config)
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    execute_tools_with_plugins(
        thread,
        result,
        tools,
        config.parallel_tools,
        &config.plugins,
    )
    .await
}

fn runtime_with_tool_caller_context(
    thread: &Thread,
    state: &Value,
    config: Option<&AgentConfig>,
) -> Result<carve_state::Runtime, AgentLoopError> {
    let mut rt = thread.runtime.clone();
    if rt.value(TOOL_RUNTIME_CALLER_THREAD_ID_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_THREAD_ID_KEY, thread.id.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_RUNTIME_CALLER_STATE_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_STATE_KEY, state.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_RUNTIME_CALLER_MESSAGES_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_MESSAGES_KEY, thread.messages.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if let Some(cfg) = config {
        crate::tool_filter::set_runtime_filters_from_definition_if_absent(&mut rt, cfg)
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    Ok(rt)
}

/// Execute tool calls with plugin hooks (backward compatible).
pub async fn execute_tools_with_plugins(
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<Thread, AgentLoopError> {
    if result.tool_calls.is_empty() {
        return Ok(thread);
    }

    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let rt_for_tools = runtime_with_tool_caller_context(&thread, &state, None)?;

    let results = if parallel {
        execute_tools_parallel_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            plugins,
            None,
            Some(&rt_for_tools),
            &thread.id,
        )
        .await
    } else {
        execute_tools_sequential_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            plugins,
            None,
            Some(&rt_for_tools),
            &thread.id,
        )
        .await
    }?;

    let metadata = tool_result_metadata_from_session(&thread);
    let applied = apply_tool_results_to_session(thread, &results, metadata, parallel)?;
    if let Some(interaction) = applied.pending_interaction {
        return Err(AgentLoopError::PendingInteraction {
            thread: Box::new(applied.thread),
            interaction: Box::new(interaction),
        });
    }
    Ok(applied.thread)
}

/// Execute tools in parallel with phase hooks.
async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::types::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    runtime: Option<&carve_state::Runtime>,
    thread_id: &str,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use futures::future::join_all;

    // Clone runtime for parallel tasks (Runtime is Clone).
    let runtime_owned = runtime.cloned();
    let thread_id = thread_id.to_string();

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();
        let tool_descriptors = tool_descriptors.to_vec();
        let activity_manager = activity_manager.clone();
        let rt = runtime_owned.clone();
        let sid = thread_id.clone();

        async move {
            execute_single_tool_with_phases(
                tool.as_deref(),
                &call,
                &state,
                &tool_descriptors,
                &plugins,
                activity_manager,
                rt.as_ref(),
                &sid,
            )
            .await
        }
    });

    let results = join_all(futures).await;
    results.into_iter().collect()
}

/// Execute tools sequentially with phase hooks.
async fn execute_tools_sequential_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::types::ToolCall],
    initial_state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    runtime: Option<&carve_state::Runtime>,
    thread_id: &str,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use carve_state::apply_patch;

    let mut state = initial_state.clone();
    let mut results = Vec::with_capacity(calls.len());

    for call in calls {
        let tool = tools.get(&call.name).cloned();
        let result = execute_single_tool_with_phases(
            tool.as_deref(),
            call,
            &state,
            tool_descriptors,
            plugins,
            activity_manager.clone(),
            runtime,
            thread_id,
        )
        .await?;

        // Apply patch to state for next tool
        if let Some(ref patch) = result.execution.patch {
            state = apply_patch(&state, patch.patch()).map_err(|e| {
                AgentLoopError::StateError(format!(
                    "failed to apply tool patch for call '{}': {}",
                    result.execution.call.id, e
                ))
            })?;
        }
        // Apply pending patches from plugins to state for next tool
        for pp in &result.pending_patches {
            state = apply_patch(&state, pp.patch()).map_err(|e| {
                AgentLoopError::StateError(format!(
                    "failed to apply plugin patch for call '{}': {}",
                    result.execution.call.id, e
                ))
            })?;
        }

        results.push(result);

        // Best-effort flow control: in sequential mode, stop at the first pending interaction.
        // This prevents later tool calls from executing while the client still needs to respond.
        if results
            .last()
            .and_then(|r| r.pending_interaction.as_ref())
            .is_some()
        {
            break;
        }
    }

    Ok(results)
}

/// Result of tool execution with phase hooks.
pub struct ToolExecutionResult {
    /// The tool execution result.
    pub execution: ToolExecution,
    /// System reminders collected during execution.
    pub reminders: Vec<String>,
    /// Pending interaction if tool is waiting for user action.
    pub pending_interaction: Option<crate::state_types::Interaction>,
    /// Pending patches from plugins during tool phases.
    pub pending_patches: Vec<TrackedPatch>,
}

/// Execute a single tool with phase hooks.
async fn execute_single_tool_with_phases(
    tool: Option<&dyn Tool>,
    call: &crate::types::ToolCall,
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    runtime: Option<&carve_state::Runtime>,
    thread_id: &str,
) -> Result<ToolExecutionResult, AgentLoopError> {
    // Create a thread stub so plugins see the real thread id and runtime.
    let mut temp_thread = Thread::with_initial_state(thread_id, state.clone());
    if let Some(rt) = runtime {
        temp_thread.runtime = rt.clone();
    }

    // Create plugin Context for tool phases (separate from tool's own Context)
    let plugin_ctx = Context::new(state, "plugin_phase", "plugin:tool_phase").with_runtime(runtime);

    // Create StepContext for this tool
    let mut step = StepContext::new(&temp_thread, tool_descriptors.to_vec());
    step.tool = Some(ToolContext::new(call));

    // Phase: BeforeToolExecute
    emit_phase_checked(Phase::BeforeToolExecute, &mut step, &plugin_ctx, plugins).await?;

    // Check if blocked or pending
    let (execution, pending_interaction) = if !crate::tool_filter::is_runtime_allowed(
        runtime,
        &call.name,
        crate::tool_filter::RUNTIME_ALLOWED_TOOLS_KEY,
        crate::tool_filter::RUNTIME_EXCLUDED_TOOLS_KEY,
    ) {
        (
            ToolExecution {
                call: call.clone(),
                result: ToolResult::error(
                    &call.name,
                    format!("Tool '{}' is not allowed by current policy", call.name),
                ),
                patch: None,
            },
            None,
        )
    } else if step.tool_blocked() {
        let reason = step
            .tool
            .as_ref()
            .and_then(|t| t.block_reason.clone())
            .unwrap_or_else(|| "Blocked by plugin".to_string());
        (
            ToolExecution {
                call: call.clone(),
                result: ToolResult::error(&call.name, reason),
                patch: None,
            },
            None,
        )
    } else if step.tool_pending() {
        let interaction = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.clone());
        (
            ToolExecution {
                call: call.clone(),
                result: ToolResult::pending(&call.name, "Waiting for user confirmation"),
                patch: None,
            },
            interaction,
        )
    } else if tool.is_none() {
        (
            ToolExecution {
                call: call.clone(),
                result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
                patch: None,
            },
            None,
        )
    } else {
        // Execute the tool with its own Context (instrumented with tracing span)
        let tool_span = step.tracing_span.take().unwrap_or_else(tracing::Span::none);
        let tool_ctx = carve_state::Context::new_with_activity_manager(
            state,
            &call.id,
            format!("tool:{}", call.name),
            activity_manager,
        )
        .with_runtime(runtime);
        let result = async {
            match tool
                .unwrap()
                .execute(call.arguments.clone(), &tool_ctx)
                .await
            {
                Ok(r) => r,
                Err(e) => ToolResult::error(&call.name, e.to_string()),
            }
        }
        .instrument(tool_span)
        .await;

        let patch = tool_ctx.take_patch();
        let patch = if patch.patch().is_empty() {
            None
        } else {
            Some(patch)
        };

        (
            ToolExecution {
                call: call.clone(),
                result,
                patch,
            },
            None,
        )
    };

    // Set tool result in context
    step.set_tool_result(execution.result.clone());

    // Phase: AfterToolExecute
    emit_phase_checked(Phase::AfterToolExecute, &mut step, &plugin_ctx, plugins).await?;

    // Flush plugin state ops into pending patches
    let plugin_patch = plugin_ctx.take_patch();
    if !plugin_patch.patch().is_empty() {
        step.pending_patches.push(plugin_patch);
    }

    let pending_patches = std::mem::take(&mut step.pending_patches);

    Ok(ToolExecutionResult {
        execution,
        reminders: step.system_reminders.clone(),
        pending_interaction,
        pending_patches,
    })
}

// ---------------------------------------------------------------------------
// Loop State Tracking
// ---------------------------------------------------------------------------

/// Internal state tracked across loop iterations for stop condition evaluation.
struct LoopState {
    rounds: usize,
    total_input_tokens: usize,
    total_output_tokens: usize,
    consecutive_errors: usize,
    start_time: Instant,
    /// Tool call names per round (most recent last), capped at 20 entries.
    tool_call_history: VecDeque<Vec<String>>,
}

impl LoopState {
    fn new() -> Self {
        Self {
            rounds: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            consecutive_errors: 0,
            start_time: Instant::now(),
            tool_call_history: VecDeque::new(),
        }
    }

    fn update_from_response(&mut self, result: &StreamResult) {
        if let Some(ref usage) = result.usage {
            self.total_input_tokens += usage.prompt_tokens.unwrap_or(0) as usize;
            self.total_output_tokens += usage.completion_tokens.unwrap_or(0) as usize;
        }
    }

    fn record_tool_round(&mut self, tool_calls: &[crate::types::ToolCall], error_count: usize) {
        let mut names: Vec<String> = tool_calls.iter().map(|tc| tc.name.clone()).collect();
        names.sort();
        if self.tool_call_history.len() >= 20 {
            self.tool_call_history.pop_front();
        }
        self.tool_call_history.push_back(names);

        if error_count > 0 && error_count == tool_calls.len() {
            self.consecutive_errors += 1;
        } else {
            self.consecutive_errors = 0;
        }
    }

    fn to_check_context<'a>(
        &'a self,
        result: &'a StreamResult,
        thread: &'a Thread,
    ) -> StopCheckContext<'a> {
        StopCheckContext {
            rounds: self.rounds,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            consecutive_errors: self.consecutive_errors,
            elapsed: self.start_time.elapsed(),
            last_tool_calls: &result.tool_calls,
            last_text: &result.text,
            tool_call_history: &self.tool_call_history,
            thread,
        }
    }
}

/// Build the effective stop conditions for a run.
///
/// If the user explicitly configured stop conditions, use those.
/// Otherwise, create a default `MaxRounds` from `config.max_rounds`.
fn effective_stop_conditions(config: &AgentConfig) -> Vec<Arc<dyn StopCondition>> {
    let mut conditions = config.stop_conditions.clone();
    for spec in &config.stop_condition_specs {
        conditions.push(spec.clone().into_condition());
    }
    if conditions.is_empty() {
        return vec![Arc::new(crate::stop::MaxRounds(config.max_rounds))];
    }
    conditions
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

        let results = if config.parallel_tools {
            execute_tools_parallel_with_phases(
                tools,
                &result.tool_calls,
                &state,
                &tool_descriptors,
                &config.plugins,
                None,
                Some(&rt_for_tools),
                &thread.id,
            )
            .await
        } else {
            execute_tools_sequential_with_phases(
                tools,
                &result.tool_calls,
                &state,
                &tool_descriptors,
                &config.plugins,
                None,
                Some(&rt_for_tools),
                &thread.id,
            )
            .await
        };

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
                                crate::stream::StreamOutput::TextDelta(delta) => {
                                    yield AgentEvent::TextDelta { delta };
                                }
                                crate::stream::StreamOutput::ToolCallStart { id, name } => {
                                    yield AgentEvent::ToolCallStart { id, name };
                                }
                                crate::stream::StreamOutput::ToolCallDelta { id, args_delta } => {
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
                if config.parallel_tools {
                    Box::pin(execute_tools_parallel_with_phases(
                        &tools,
                        &result.tool_calls,
                        &state,
                        &tool_descriptors,
                        &config.plugins,
                        Some(activity_manager.clone()),
                        Some(&rt_for_tools),
                        &sid_for_tools,
                    ))
                } else {
                    Box::pin(execute_tools_sequential_with_phases(
                        &tools,
                        &result.tool_calls,
                        &state,
                        &tool_descriptors,
                        &config.plugins,
                        Some(activity_manager.clone()),
                        Some(&rt_for_tools),
                        &sid_for_tools,
                    ))
                };
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
