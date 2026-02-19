use super::core::{
    clear_agent_pending_interaction, drain_agent_append_user_messages,
    set_agent_pending_interaction,
};
use super::plugin_runtime::emit_phase_checked;
use super::{
    AgentConfig, AgentLoopError, RunCancellationToken, TOOL_SCOPE_CALLER_MESSAGES_KEY,
    TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::plugin::phase::{Phase, StepContext, ToolContext};
use crate::contracts::Interaction;
use crate::contracts::runtime::{
    StreamResult, ToolExecution, ToolExecutionRequest, ToolExecutionResult,
    ToolExecutor, ToolExecutorError,
};
use crate::engine::tool_filter::{SCOPE_ALLOWED_TOOLS_KEY, SCOPE_EXCLUDED_TOOLS_KEY};
use crate::contracts::runtime::ActivityManager;
use crate::contracts::thread::Thread;
use crate::contracts::thread::{Message, MessageMetadata};
use crate::contracts::tool::{Tool, ToolDescriptor, ToolResult};
use crate::contracts::RunContext;
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
    pub(super) pending_interaction: Option<Interaction>,
    pub(super) state_snapshot: Option<Value>,
}

fn map_tool_executor_error(err: AgentLoopError) -> ToolExecutorError {
    match err {
        AgentLoopError::Cancelled { run_ctx } => ToolExecutorError::Cancelled {
            thread_id: run_ctx.thread_id().to_string(),
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
            request.run_config,
            request.thread_id,
            request.thread_messages,
            request.state_version,
            request.cancellation_token,
            request.run_overlay,
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
            request.run_config,
            request.thread_id,
            request.thread_messages,
            request.state_version,
            request.cancellation_token,
            request.run_overlay,
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
    run_ctx: &mut RunContext,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    check_parallel_patch_conflicts: bool,
) -> Result<AppliedToolResults, AgentLoopError> {
    apply_tool_results_impl(
        run_ctx,
        results,
        metadata,
        check_parallel_patch_conflicts,
        None,
    )
}

pub(super) fn apply_tool_results_impl(
    run_ctx: &mut RunContext,
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
    let mut patches: Vec<TrackedPatch> = collect_patches(
        &results
            .iter()
            .map(|r| r.execution.clone())
            .collect::<Vec<_>>(),
    );
    for r in results {
        patches.extend(r.pending_patches.iter().cloned());
    }
    let mut state_changed = !patches.is_empty();
    run_ctx.add_patches(patches);

    // Add tool result messages for all executions.
    let tool_messages: Vec<Arc<Message>> = results
        .iter()
        .flat_map(|r| {
            let mut msgs = if r.pending_interaction.is_some() {
                vec![Message::tool(
                    &r.execution.call.id,
                    format!(
                        "Tool '{}' is awaiting approval. Execution paused.",
                        r.execution.call.name
                    ),
                )]
            } else {
                let mut tool_msg = tool_response(&r.execution.call.id, &r.execution.result);
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
            if let Some(ref meta) = metadata {
                for msg in &mut msgs {
                    msg.metadata = Some(meta.clone());
                }
            }
            msgs.into_iter().map(Arc::new).collect::<Vec<_>>()
        })
        .collect();

    run_ctx.add_messages(tool_messages);
    let appended_count = drain_agent_append_user_messages(run_ctx, results, metadata.as_ref())?;
    if appended_count > 0 {
        state_changed = true;
    }

    if let Some(interaction) = pending_interaction.clone() {
        let state = run_ctx
            .rebuild_state()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        let patch = set_agent_pending_interaction(&state, interaction.clone());
        if !patch.patch().is_empty() {
            state_changed = true;
            run_ctx.add_patch(patch);
        }
        let state_snapshot = if state_changed {
            Some(
                run_ctx
                    .rebuild_state()
                    .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
            )
        } else {
            None
        };
        return Ok(AppliedToolResults {
            pending_interaction: Some(interaction),
            state_snapshot,
        });
    }

    // If a previous run left a persisted pending interaction, clear it once we successfully
    // complete tool execution without creating a new pending interaction.
    let state = run_ctx
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
            run_ctx.add_patch(patch);
        }
    }

    let state_snapshot = if state_changed {
        Some(
            run_ctx
                .rebuild_state()
                .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
        )
    } else {
        None
    };

    Ok(AppliedToolResults {
        pending_interaction: None,
        state_snapshot,
    })
}

fn tool_result_metadata_from_run_ctx(run_ctx: &RunContext) -> Option<MessageMetadata> {
    let run_id = run_ctx
        .run_config
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .or_else(|| {
            run_ctx.messages().iter().rev().find_map(|m| {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.run_id.as_ref().cloned())
            })
        });

    let step_index = run_ctx
        .messages()
        .iter()
        .rev()
        .find_map(|m| m.metadata.as_ref().and_then(|meta| meta.step_index));

    if run_id.is_none() && step_index.is_none() {
        None
    } else {
        Some(MessageMetadata { run_id, step_index })
    }
}

#[allow(dead_code)]
pub(super) fn next_step_index(run_ctx: &RunContext) -> u32 {
    run_ctx
        .messages()
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
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<Thread, AgentLoopError> {
    execute_tools_with_plugins(thread, result, tools, parallel, &[]).await
}

/// Execute tool calls with phase-based plugin hooks.
pub async fn execute_tools_with_config(
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
) -> Result<Thread, AgentLoopError> {
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
    run_ctx: &RunContext,
    state: &Value,
    _config: Option<&AgentConfig>,
) -> Result<carve_agent_contract::RunConfig, AgentLoopError> {
    let mut rt = run_ctx.run_config.clone();
    if rt.value(TOOL_SCOPE_CALLER_THREAD_ID_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_THREAD_ID_KEY, run_ctx.thread_id().to_string())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_SCOPE_CALLER_STATE_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_STATE_KEY, state.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_SCOPE_CALLER_MESSAGES_KEY).is_none() {
        rt.set(TOOL_SCOPE_CALLER_MESSAGES_KEY, run_ctx.messages().to_vec())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    Ok(rt)
}

/// Execute tool calls with plugin hooks.
pub async fn execute_tools_with_plugins(
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<Thread, AgentLoopError> {
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
    thread: Thread,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    executor: &dyn ToolExecutor,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<Thread, AgentLoopError> {
    if result.tool_calls.is_empty() {
        return Ok(thread);
    }

    // Build RunContext from thread for internal use
    let rebuilt_state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let mut run_ctx = RunContext::new(
        &thread.id,
        rebuilt_state.clone(),
        thread.messages.clone(),
        thread.run_config.clone(),
    );

    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let rt_for_tools = scope_with_tool_caller_context(&run_ctx, &rebuilt_state, None)?;
    let results = executor
        .execute(ToolExecutionRequest {
            tools,
            calls: &result.tool_calls,
            state: &rebuilt_state,
            tool_descriptors: &tool_descriptors,
            plugins,
            activity_manager: None,
            run_config: Some(&rt_for_tools),
            thread_id: run_ctx.thread_id(),
            thread_messages: run_ctx.messages(),
            state_version: run_ctx.version(),
            cancellation_token: None,
            run_overlay: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
        .await?;

    let metadata = tool_result_metadata_from_run_ctx(&run_ctx);
    let applied = apply_tool_results_to_session(
        &mut run_ctx,
        &results,
        metadata,
        executor.requires_parallel_patch_conflict_check(),
    )?;
    if let Some(interaction) = applied.pending_interaction {
        return Err(AgentLoopError::PendingInteraction {
            run_ctx: Box::new(run_ctx),
            interaction: Box::new(interaction),
        });
    }

    // Reconstruct thread from RunContext delta
    let delta = run_ctx.take_delta();
    let mut out_thread = thread;
    for msg in delta.messages {
        out_thread = out_thread.with_message((*msg).clone());
    }
    out_thread = out_thread.with_patches(delta.patches);
    Ok(out_thread)
}

/// Execute tools in parallel with phase hooks.
pub(super) async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::thread::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_agent_contract::RunConfig>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    state_version: u64,
    cancellation_token: Option<&RunCancellationToken>,
    run_overlay: Arc<std::sync::Mutex<Vec<carve_state::Op>>>,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use futures::future::join_all;

    if cancellation_token.is_some_and(|token| token.is_cancelled()) {
        return Err(AgentLoopError::Cancelled {
            run_ctx: Box::new(RunContext::new(
                thread_id, serde_json::json!({}), vec![],
                carve_agent_contract::RunConfig::default(),
            )),
        });
    }

    // Clone scope state for parallel tasks (RunConfig is Clone).
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
        let run_overlay = run_overlay.clone();

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
                run_overlay,
            )
            .await
        }
    });

    let join_future = join_all(futures);
    let results = if let Some(token) = cancellation_token {
        tokio::select! {
            _ = token.cancelled() => {
                return Err(AgentLoopError::Cancelled {
                    run_ctx: Box::new(RunContext::new(
                        thread_id.clone(), serde_json::json!({}), vec![],
                        carve_agent_contract::RunConfig::default(),
                    )),
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
    calls: &[crate::contracts::thread::ToolCall],
    initial_state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_agent_contract::RunConfig>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    state_version: u64,
    cancellation_token: Option<&RunCancellationToken>,
    run_overlay: Arc<std::sync::Mutex<Vec<carve_state::Op>>>,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use carve_state::apply_patch;

    if cancellation_token.is_some_and(|token| token.is_cancelled()) {
        return Err(AgentLoopError::Cancelled {
            run_ctx: Box::new(RunContext::new(
                thread_id, serde_json::json!({}), vec![],
                carve_agent_contract::RunConfig::default(),
            )),
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
                        run_ctx: Box::new(RunContext::new(
                            thread_id, serde_json::json!({}), vec![],
                            carve_agent_contract::RunConfig::default(),
                        )),
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
                    run_overlay.clone(),
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
                run_overlay.clone(),
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
    call: &crate::contracts::thread::ToolCall,
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    activity_manager: Option<Arc<dyn ActivityManager>>,
    scope: Option<&carve_agent_contract::RunConfig>,
    thread_id: &str,
    thread_messages: &[Arc<Message>],
    _state_version: u64,
    run_overlay: Arc<std::sync::Mutex<Vec<carve_state::Op>>>,
) -> Result<ToolExecutionResult, AgentLoopError> {
    // Create ToolCallContext for plugin phases
    let doc = carve_state::DocCell::new(state.clone());
    let ops = std::sync::Mutex::new(Vec::new());
    let pending_messages = std::sync::Mutex::new(Vec::new());
    let empty_scope = carve_agent_contract::RunConfig::default();
    let plugin_scope = scope.unwrap_or(&empty_scope);
    let plugin_tool_call_ctx = crate::contracts::ToolCallContext::new(
        &doc,
        &ops,
        run_overlay.clone(),
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
        let tool_overlay = run_overlay.clone();
        let tool_pending_msgs = std::sync::Mutex::new(Vec::new());
        let tool_ctx = crate::contracts::ToolCallContext::new(
            &tool_doc,
            &tool_ops,
            tool_overlay,
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
