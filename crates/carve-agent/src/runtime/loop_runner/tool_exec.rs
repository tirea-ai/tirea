use super::core::{
    clear_agent_pending_interaction, drain_agent_append_user_messages, reduce_thread_mutations,
    set_agent_pending_interaction, ThreadMutationBatch,
};
use super::plugin_runtime::emit_phase_checked;
use super::{
    AgentConfig, AgentLoopError, TOOL_RUNTIME_CALLER_MESSAGES_KEY, TOOL_RUNTIME_CALLER_STATE_KEY,
    TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
use crate::contracts::agent_plugin::AgentPlugin;
use crate::contracts::conversation::Thread;
use crate::contracts::conversation::{Message, MessageMetadata};
use crate::contracts::events::StreamResult;
use crate::contracts::phase::{Phase, StepContext, ToolContext};
use crate::contracts::state_types::{Interaction, AGENT_STATE_PATH};
use crate::contracts::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::engine::convert::tool_response;
use crate::engine::tool_execution::{collect_patches, ToolExecution};
use carve_state::{ActivityManager, Context, PatchExt, TrackedPatch};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::Instrument;

pub(super) struct AppliedToolResults {
    pub(super) thread: Thread,
    pub(super) pending_interaction: Option<Interaction>,
    pub(super) state_snapshot: Option<Value>,
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
    thread: Thread,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    parallel_tools: bool,
) -> Result<AppliedToolResults, AgentLoopError> {
    apply_tool_results_impl(thread, results, metadata, parallel_tools, None)
}

pub(super) fn apply_tool_results_impl(
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

pub(super) fn next_step_index(thread: &Thread) -> u32 {
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
    crate::engine::tool_filter::set_runtime_filters_from_definition_if_absent(
        &mut thread.runtime,
        config,
    )
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

pub(super) fn runtime_with_tool_caller_context(
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
        crate::engine::tool_filter::set_runtime_filters_from_definition_if_absent(&mut rt, cfg)
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
    if result.tool_calls.is_empty() {
        return Ok(thread);
    }

    let state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let rt_for_tools = runtime_with_tool_caller_context(&thread, &state, None)?;
    let results = execute_tool_calls_with_phases(
        tools,
        &result.tool_calls,
        &state,
        &tool_descriptors,
        plugins,
        parallel,
        None,
        Some(&rt_for_tools),
        &thread.id,
    )
    .await?;

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

pub(super) async fn execute_tool_calls_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::conversation::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    parallel: bool,
    activity_manager: Option<Arc<dyn ActivityManager>>,
    runtime: Option<&carve_state::Runtime>,
    thread_id: &str,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    if parallel {
        execute_tools_parallel_with_phases(
            tools,
            calls,
            state,
            tool_descriptors,
            plugins,
            activity_manager,
            runtime,
            thread_id,
        )
        .await
    } else {
        execute_tools_sequential_with_phases(
            tools,
            calls,
            state,
            tool_descriptors,
            plugins,
            activity_manager,
            runtime,
            thread_id,
        )
        .await
    }
}

/// Execute tools in parallel with phase hooks.
pub(super) async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::conversation::ToolCall],
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
pub(super) async fn execute_tools_sequential_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::contracts::conversation::ToolCall],
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
    pub pending_interaction: Option<Interaction>,
    /// Pending patches from plugins during tool phases.
    pub pending_patches: Vec<TrackedPatch>,
}

/// Execute a single tool with phase hooks.
pub(super) async fn execute_single_tool_with_phases(
    tool: Option<&dyn Tool>,
    call: &crate::contracts::conversation::ToolCall,
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
    let (execution, pending_interaction) = if !crate::engine::tool_filter::is_runtime_allowed(
        runtime,
        &call.name,
        crate::engine::tool_filter::RUNTIME_ALLOWED_TOOLS_KEY,
        crate::engine::tool_filter::RUNTIME_EXCLUDED_TOOLS_KEY,
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
