use super::core::{
    clear_agent_pending_interaction, drain_agent_append_user_messages, reduce_thread_mutations,
    set_agent_pending_interaction, ThreadMutationBatch,
};
use super::plugin_runtime::emit_phase_checked;
use super::{
    AgentConfig, AgentLoopError, RunCancellationToken, TOOL_SCOPE_CALLER_MESSAGES_KEY,
    TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::runtime::phase::{Phase, StepContext, ToolContext};
use crate::contracts::runtime::{
    Interaction, StreamResult, ToolExecution, ToolExecutionRequest, ToolExecutionResult,
    ToolExecutor, ToolExecutorError,
};
use crate::engine::tool_filter::{SCOPE_ALLOWED_TOOLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY};
use crate::contracts::state::{ActivityManager, AgentState};
use crate::contracts::state::{Message, MessageMetadata};
use crate::contracts::tool::{Tool, ToolDescriptor, ToolResult};
use crate::engine::convert::tool_response;
use crate::engine::tool_execution::collect_patches;
use crate::runtime::control::LoopControlState;
use carve_state::State;
use async_trait::async_trait;
use carve_state::{PatchExt, TrackedPatch};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) struct AppliedToolResults {
    pub(super) thread: AgentState,
    pub(super) pending_interaction: Option<Interaction>,
    pub(super) state_snapshot: Option<Value>,
}

fn map_tool_executor_error(err: AgentLoopError) -> ToolExecutorError {
    match err {
        AgentLoopError::Cancelled { state } => ToolExecutorError::Cancelled {
            thread_id: state.id.clone(),
        },
        other => ToolExecutorError::Failed {
            message: other.to_string(),
        },
    }
}

/// Executes all tool calls concurrently.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParallelToolExecutor;

#[async_trait]
impl ToolExecutor for ParallelToolExecutor {
    async fn execute(
        &self,
        request: ToolExecutionRequest<'_>,
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError> {
        execute_tools_parallel_with_phases(
            request.tools,
            request.calls,
            request.state,
            request.tool_descriptors,
            request.plugins,
            request.activity_manager,
            request.scope,
            request.thread_id,
            request.thread_messages,
            request.state_version,
            request.cancellation_token,
        )
        .await
        .map_err(map_tool_executor_error)
    }

    fn name(&self) -> &'static str {
        "parallel"
    }

    fn requires_parallel_patch_conflict_check(&self) -> bool {
        true
    }
}

/// Executes tool calls one-by-one in call order.
#[derive(Debug, Clone, Copy, Default)]
pub struct SequentialToolExecutor;

#[async_trait]
impl ToolExecutor for SequentialToolExecutor {
    async fn execute(
        &self,
        request: ToolExecutionRequest<'_>,
    ) -> Result<Vec<ToolExecutionResult>, ToolExecutorError> {
        execute_tools_sequential_with_phases(
            request.tools,
            request.calls,
            request.state,
            request.tool_descriptors,
            request.plugins,
            request.activity_manager,
            request.scope,
            request.thread_id,
            request.thread_messages,
            request.state_version,
            request.cancellation_token,
        )
        .await
        .map_err(map_tool_executor_error)
    }

    fn name(&self) -> &'static str {
        "sequential"
    }
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

pub(super) fn apply_tool_results_to_session(
    thread: AgentState,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    check_parallel_patch_conflicts: bool,
) -> Result<AppliedToolResults, AgentLoopError> {
    apply_tool_results_impl(
        thread,
        results,
        metadata,
        check_parallel_patch_conflicts,
        None,
    )
}

pub(super) fn apply_tool_results_impl(
    thread: AgentState,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    check_parallel_patch_conflicts: bool,
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

    if check_parallel_patch_conflicts {
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
        .get(LoopControlState::PATH)
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

fn tool_result_metadata_from_session(thread: &AgentState) -> Option<MessageMetadata> {
    let run_id = thread
        .scope
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

pub(super) fn next_step_index(thread: &AgentState) -> u32 {
    thread
        .messages
        .iter()
        .filter_map(|m| m.metadata.as_ref().and_then(|meta| meta.step_index))
        .max()
        .map(|v| v.saturating_add(1))
        .unwrap_or(0)
}

pub(super) fn step_metadata(run_id: Option<String>, step_index: u32) -> MessageMetadata {
    MessageMetadata {
        run_id,
        step_index: Some(step_index),
    }
}

/// Execute tool calls (simplified version without plugins).
///
/// This is the simpler API for tests and cases where plugins aren't needed.
pub async fn execute_tools(
    thread: AgentState,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<AgentState, AgentLoopError> {
    execute_tools_with_plugins(thread, result, tools, parallel, &[]).await
}

/// Execute tool calls with phase-based plugin hooks.
pub async fn execute_tools_with_config(
    thread: AgentState,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
) -> Result<AgentState, AgentLoopError> {
    execute_tools_with_plugins_and_executor(
        thread,
        result,
        tools,
        config.tool_executor.as_ref(),
        &config.plugins,
    )
    .await
}

pub(super) fn scope_with_tool_caller_context(
    thread: &AgentState,
    state: &Value,
    _config: Option<&AgentConfig>,
) -> Result<carve_state::ScopeState, AgentLoopError> {
    let mut rt = thread.scope.clone();
    if rt.value(TOOL_SCOPE_CALLER_THREAD_ID_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, thread.id.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_SCOPE_CALLER_STATE_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_STATE_KEY, state.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_SCOPE_CALLER_MESSAGES_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_MESSAGES_KEY, thread.messages.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    Ok(rt)
}

/// Execute tool calls with plugin hooks.
pub async fn execute_tools_with_plugins(
    thread: AgentState,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<AgentState, AgentLoopError> {
    let parallel_executor = ParallelToolExecutor;
    let sequential_executor = SequentialToolExecutor;
    let executor: &dyn ToolExecutor = if parallel {
        &parallel_executor
    } else {
        &sequential_executor
    };
    execute_tools_with_plugins_and_executor(thread, result, tools, executor, plugins).await
}

pub async fn execute_tools_with_plugins_and_executor(
    thread: AgentState,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    executor: &dyn ToolExecutor,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<AgentState, AgentLoopError> {
    if result.tool_calls.is_empty() {
        return Ok(thread);
    }

    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let rt_for_tools = scope_with_tool_caller_context(&thread, &state, None)?;
    let results = executor
        .execute(ToolExecutionRequest {
            tools,
            calls: &result.tool_calls,
            state: &state,
            tool_descriptors: &tool_descriptors,
            plugins,
            activity_manager: None,
            scope: Some(&rt_for_tools),
            thread_id: &thread.id,
            thread_messages: &thread.messages,
            state_version: super::agent_state_version(&thread),
            cancellation_token: None,
        })
        .await?;

    let metadata = tool_result_metadata_from_session(&thread);
    let applied = apply_tool_results_to_session(
        thread,
        &results,
        metadata,
        executor.requires_parallel_patch_conflict_check(),
    )?;
    if let Some(interaction) = applied.pending_interaction {
        return Err(AgentLoopError::PendingInteraction {
            state: Box::new(applied.thread),
            interaction: Box::new(interaction),
        });
    }
    Ok(applied.thread)
}

/// Execute tools in parallel with phase hooks.
pub(super) async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::state::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_state::ScopeState>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    state_version: u64,
    cancellation_token: Option<&RunCancellationToken>,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use futures::future::join_all;

    if cancellation_token.is_some_and(|token| token.is_cancelled()) {
        return Err(AgentLoopError::Cancelled {
            state: Box::new(AgentState::new(thread_id.to_string())),
        });
    }

    // Clone scope state for parallel tasks (ScopeState is Clone).
    let scope_owned = scope.cloned();
    let thread_id = thread_id.to_string();
    let thread_messages = Arc::new(thread_messages.to_vec());

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();
        let tool_descriptors = tool_descriptors.to_vec();
        let activity_manager = activity_manager.clone();
        let rt = scope_owned.clone();
        let sid = thread_id.clone();
        let thread_messages = thread_messages.clone();

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
                thread_messages.as_slice(),
                state_version,
            )
            .await
        }
    });

    let join_future = join_all(futures);
    let results = if let Some(token) = cancellation_token {
        tokio::select! {
            _ = token.cancelled() => {
                return Err(AgentLoopError::Cancelled {
                    state: Box::new(AgentState::new(thread_id.clone())),
                });
            }
            results = join_future => results,
        }
    } else {
        join_future.await
    };
    results.into_iter().collect()
}

/// Execute tools sequentially with phase hooks.
pub(super) async fn execute_tools_sequential_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::state::ToolCall],
    initial_state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_state::ScopeState>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    state_version: u64,
    cancellation_token: Option<&RunCancellationToken>,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use carve_state::apply_patch;

    if cancellation_token.is_some_and(|token| token.is_cancelled()) {
        return Err(AgentLoopError::Cancelled {
            state: Box::new(AgentState::new(thread_id.to_string())),
        });
    }

    let mut state = initial_state.clone();
    let mut results = Vec::with_capacity(calls.len());

    for call in calls {
        let tool = tools.get(&call.name).cloned();
        let result = if let Some(token) = cancellation_token {
            tokio::select! {
                _ = token.cancelled() => {
                    return Err(AgentLoopError::Cancelled {
                        state: Box::new(AgentState::new(thread_id.to_string())),
                    });
                }
                result = execute_single_tool_with_phases(
                    tool.as_deref(),
                    call,
                    &state,
                    tool_descriptors,
                    plugins,
                    activity_manager.clone(),
                    scope,
                    thread_id,
                    thread_messages,
                    state_version,
                ) => result?
            }
        } else {
            execute_single_tool_with_phases(
                tool.as_deref(),
                call,
                &state,
                tool_descriptors,
                plugins,
                activity_manager.clone(),
                scope,
                thread_id,
                thread_messages,
                state_version,
            )
            .await?
        };

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

/// Execute a single tool with phase hooks.
pub(super) async fn execute_single_tool_with_phases(
    tool: Option<&dyn Tool>,
    call: &crate::contracts::state::ToolCall,
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_state::ScopeState>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    _state_version: u64,
) -> Result<ToolExecutionResult, AgentLoopError> {
    // Create ToolCallContext for plugin phases
    let doc = carve_state::DocCell::new(state.clone());
    let ops = std::sync::Mutex::new(Vec::new());
    let pending_messages = std::sync::Mutex::new(Vec::new());
    let empty_scope = carve_state::ScopeState::default();
    let plugin_scope = scope.unwrap_or(&empty_scope);
    let plugin_tool_call_ctx = crate::contracts::ToolCallContext::new(
        &doc,
        &ops,
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        "plugin_phase",
        "plugin:tool_phase",
        plugin_scope,
        &pending_messages,
        None,
    );

    // Create StepContext for this tool
    let mut step = StepContext::new(
        plugin_tool_call_ctx,
        thread_id,
        thread_messages,
        tool_descriptors.to_vec(),
    );
    step.tool = Some(ToolContext::new(call));

    // Phase: BeforeToolExecute
    emit_phase_checked(Phase::BeforeToolExecute, &mut step, plugins).await?;

    // Check if blocked or pending
    let (execution, pending_interaction) = if !crate::engine::tool_filter::is_scope_allowed(
        scope,
        &call.name,
        SCOPE_ALLOWED_TOOLS_KEY,
        SCOPE_EXCLUDED_TOOLS_KEY,
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
        // Execute the tool with its own ToolCallContext
        let tool_doc = carve_state::DocCell::new(state.clone());
        let tool_ops = std::sync::Mutex::new(Vec::new());
        let tool_overlay = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let tool_pending_msgs = std::sync::Mutex::new(Vec::new());
        let tool_ctx = crate::contracts::ToolCallContext::new(
            &tool_doc,
            &tool_ops,
            tool_overlay,
            // `tool_call_id` is the stable idempotency key seen by the tool.
            &call.id,
            format!("tool:{}", call.name),
            plugin_scope,
            &tool_pending_msgs,
            activity_manager,
        );
        let result = match tool
            .unwrap()
            .execute(call.arguments.clone(), &tool_ctx)
            .await
        {
            Ok(r) => r,
            Err(e) => ToolResult::error(&call.name, e.to_string()),
        };

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
    emit_phase_checked(Phase::AfterToolExecute, &mut step, plugins).await?;

    // Flush plugin state ops into pending patches
    let plugin_patch = step.ctx().take_patch();
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
