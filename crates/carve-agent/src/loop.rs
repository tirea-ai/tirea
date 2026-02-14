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
mod outcome;

use crate::activity::ActivityHub;
use crate::agent::uuid_v7;
use crate::convert::{assistant_message, assistant_tool_calls, build_request, tool_response};
use crate::execute::{collect_patches, ToolExecution};
use crate::phase::{Phase, StepContext, ToolContext};
use crate::plugin::AgentPlugin;
use crate::state_types::{AgentState, Interaction, AGENT_STATE_PATH};
use crate::stop::{check_stop_conditions, StopCheckContext, StopCondition, StopReason};
use crate::storage::{CheckpointReason, ThreadDelta};
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::thread::Thread;
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::types::{gen_message_id, Message, MessageMetadata};
use async_stream::stream;
use async_trait::async_trait;
use carve_state::{ActivityManager, Context, PatchExt, TrackedPatch};
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tracing::Instrument;

pub use config::{
    AgentConfig, AgentDefinition, RunCancellationToken, RunContext, ScratchpadMergePolicy,
    StateCommitError, StateCommitter,
};
pub(crate) use config::{
    TOOL_RUNTIME_CALLER_AGENT_ID_KEY, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
pub use outcome::{run_round, tool_map, tool_map_from_arc, AgentLoopError, RoundResult};
#[cfg(test)]
use tokio_util::sync::CancellationToken;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PhaseMutationSnapshot {
    skip_inference: bool,
    tool_ids: Vec<String>,
    tool_call_id: Option<String>,
    tool_name: Option<String>,
    tool_blocked: bool,
    tool_pending: bool,
    tool_pending_interaction_id: Option<String>,
    tool_has_result: bool,
}

fn phase_mutation_snapshot(step: &StepContext<'_>) -> PhaseMutationSnapshot {
    PhaseMutationSnapshot {
        skip_inference: step.skip_inference,
        tool_ids: step.tools.iter().map(|t| t.id.clone()).collect(),
        tool_call_id: step.tool.as_ref().map(|t| t.id.clone()),
        tool_name: step.tool.as_ref().map(|t| t.name.clone()),
        tool_blocked: step.tool_blocked(),
        tool_pending: step.tool_pending(),
        tool_pending_interaction_id: step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref().map(|i| i.id.clone())),
        tool_has_result: step.tool.as_ref().and_then(|t| t.result.as_ref()).is_some(),
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

fn validate_phase_mutation(
    phase: Phase,
    plugin_id: &str,
    before: &PhaseMutationSnapshot,
    after: &PhaseMutationSnapshot,
) -> Result<(), AgentLoopError> {
    // Tool list (filtering) is only valid in BeforeInference.
    if before.tool_ids != after.tool_ids && phase != Phase::BeforeInference {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool filtering outside BeforeInference ({phase})",
            plugin_id
        )));
    }

    // Skip inference signal must be set only in BeforeInference.
    if before.skip_inference != after.skip_inference && phase != Phase::BeforeInference {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated skip_inference outside BeforeInference ({phase})",
            plugin_id
        )));
    }

    // Plugins must not rewrite tool identity.
    if before.tool_call_id != after.tool_call_id || before.tool_name != after.tool_name {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool identity in phase {phase}",
            plugin_id
        )));
    }

    // Tool gate (block/pending/interaction) is only valid in BeforeToolExecute.
    let tool_gate_changed = before.tool_blocked != after.tool_blocked
        || before.tool_pending != after.tool_pending
        || before.tool_pending_interaction_id != after.tool_pending_interaction_id;
    if tool_gate_changed && phase != Phase::BeforeToolExecute {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool gate outside BeforeToolExecute ({phase})",
            plugin_id
        )));
    }

    // Tool result is produced by loop/tool execution, not by plugins.
    if before.tool_has_result != after.tool_has_result {
        return Err(AgentLoopError::StateError(format!(
            "plugin '{}' mutated tool result in phase {phase}",
            plugin_id
        )));
    }

    Ok(())
}

async fn emit_phase_checked(
    phase: Phase,
    step: &mut StepContext<'_>,
    ctx: &Context<'_>,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<(), AgentLoopError> {
    for plugin in plugins {
        let before = phase_mutation_snapshot(step);
        plugin.on_phase(phase, step, ctx).await;
        let after = phase_mutation_snapshot(step);
        validate_phase_mutation(phase, plugin.id(), &before, &after)?;
    }
    Ok(())
}

fn take_step_side_effects(
    scratchpad: &mut ScratchpadRuntimeData,
    step: &mut StepContext<'_>,
) -> Vec<TrackedPatch> {
    let pending = std::mem::take(&mut step.pending_patches);
    scratchpad.sync_from_step(step);
    pending
}

fn apply_pending_patches(thread: Thread, pending: Vec<TrackedPatch>) -> Thread {
    if pending.is_empty() {
        thread
    } else {
        thread.with_patches(pending)
    }
}

async fn run_phase_block<R, Setup, Extract>(
    thread: &Thread,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    phases: &[Phase],
    setup: Setup,
    extract: Extract,
) -> Result<(R, Vec<TrackedPatch>), AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
    Extract: FnOnce(&mut StepContext<'_>) -> R,
{
    let current_state = thread
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    let ctx =
        Context::new(&current_state, "phase", "plugin:phase").with_runtime(Some(&thread.runtime));
    let mut step = scratchpad.new_step_context(thread, tool_descriptors.to_vec());
    setup(&mut step);
    for phase in phases {
        emit_phase_checked(*phase, &mut step, &ctx, plugins).await?;
    }
    // Flush plugin state ops into pending patches
    let plugin_patch = ctx.take_patch();
    if !plugin_patch.patch().is_empty() {
        step.pending_patches.push(plugin_patch);
    }
    let output = extract(&mut step);
    let pending = take_step_side_effects(scratchpad, &mut step);
    Ok((output, pending))
}

async fn emit_phase_block<Setup>(
    phase: Phase,
    thread: &Thread,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    setup: Setup,
) -> Result<Vec<TrackedPatch>, AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
{
    let (_, pending) = run_phase_block(
        thread,
        scratchpad,
        tool_descriptors,
        plugins,
        &[phase],
        setup,
        |_| (),
    )
    .await?;
    Ok(pending)
}

async fn emit_cleanup_phases_and_apply(
    thread: Thread,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    error_type: &'static str,
    message: String,
) -> Result<Thread, AgentLoopError> {
    let pending = emit_phase_block(
        Phase::AfterInference,
        &thread,
        scratchpad,
        tool_descriptors,
        plugins,
        |step| {
            step.scratchpad_set(
                "llmmetry.inference_error",
                serde_json::json!({ "type": error_type, "message": message }),
            );
        },
    )
    .await?;
    let thread = apply_pending_patches(thread, pending);
    let pending = emit_phase_block(
        Phase::StepEnd,
        &thread,
        scratchpad,
        tool_descriptors,
        plugins,
        |_| {},
    )
    .await?;
    Ok(apply_pending_patches(thread, pending))
}

/// Emit SessionEnd phase and apply any resulting patches to the session.
async fn emit_session_end(
    thread: Thread,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
) -> Thread {
    let pending = {
        let current_state = match thread.rebuild_state() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "SessionEnd: failed to rebuild state");
                return thread;
            }
        };
        let ctx = Context::new(&current_state, "phase", "plugin:session_end")
            .with_runtime(Some(&thread.runtime));
        let mut step = scratchpad.new_step_context(&thread, tool_descriptors.to_vec());
        if let Err(e) = emit_phase_checked(Phase::SessionEnd, &mut step, &ctx, plugins).await {
            tracing::warn!(error = %e, "SessionEnd plugin phase validation failed");
        }
        let plugin_patch = ctx.take_patch();
        if !plugin_patch.patch().is_empty() {
            step.pending_patches.push(plugin_patch);
        }
        take_step_side_effects(scratchpad, &mut step)
    };
    apply_pending_patches(thread, pending)
}

/// Build terminal error events after running SessionEnd cleanup.
async fn prepare_stream_error_termination(
    thread: Thread,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    run_id: &str,
    message: String,
) -> (Thread, AgentEvent, AgentEvent) {
    let thread = emit_session_end(thread, scratchpad, tool_descriptors, plugins).await;
    let error = AgentEvent::Error { message };
    let finish = AgentEvent::RunFinish {
        thread_id: thread.id.clone(),
        run_id: run_id.to_string(),
        result: None,
        stop_reason: None,
    };
    (thread, error, finish)
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

/// Build initial scratchpad map.
fn initial_scratchpad(plugins: &[Arc<dyn AgentPlugin>]) -> HashMap<String, Value> {
    let mut data = HashMap::new();
    for plugin in plugins {
        if let Some((key, value)) = plugin.initial_scratchpad() {
            data.insert(key.to_string(), value);
        }
    }
    data
}

/// Runtime scratchpad shared across phase contexts.
#[derive(Debug, Clone, Default)]
struct ScratchpadRuntimeData {
    data: HashMap<String, Value>,
}

impl ScratchpadRuntimeData {
    fn new(plugins: &[Arc<dyn AgentPlugin>]) -> Self {
        Self {
            data: initial_scratchpad(plugins),
        }
    }

    fn new_step_context<'a>(
        &self,
        thread: &'a Thread,
        tools: Vec<ToolDescriptor>,
    ) -> StepContext<'a> {
        let mut step = StepContext::new(thread, tools);
        step.set_scratchpad_map(self.data.clone());
        step
    }

    fn sync_from_step(&mut self, step: &StepContext<'_>) {
        self.data = step.scratchpad_snapshot();
    }
}

fn tool_descriptors_for_config(
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
) -> Vec<ToolDescriptor> {
    tools
        .values()
        .map(|t| t.descriptor().clone())
        .filter(|td| {
            crate::tool_filter::is_tool_allowed(
                &td.id,
                config.allowed_tools.as_deref(),
                config.excluded_tools.as_deref(),
            )
        })
        .collect()
}

/// Build the message list for an LLM request from step context.
fn build_messages(step: &StepContext<'_>, system_prompt: &str) -> Vec<Message> {
    let mut messages = Vec::new();

    // [1] System Prompt + Context
    let system = if step.system_context.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, step.system_context.join("\n"))
    };

    if !system.is_empty() {
        messages.push(Message::system(system));
    }

    // [2] Thread Context
    for ctx in &step.session_context {
        messages.push(Message::system(ctx.clone()));
    }

    // [3+] History from session (Arc messages, cheap to clone)
    for msg in &step.thread.messages {
        messages.push((**msg).clone());
    }

    messages
}

type InferenceInputs = (Vec<Message>, Vec<String>, bool, Option<tracing::Span>);

fn inference_inputs_from_step(step: &mut StepContext<'_>, system_prompt: &str) -> InferenceInputs {
    let messages = build_messages(step, system_prompt);
    let filtered_tools = step
        .tools
        .iter()
        .map(|td| td.id.clone())
        .collect::<Vec<_>>();
    let skip_inference = step.skip_inference;
    let tracing_span = step.tracing_span.take();
    (messages, filtered_tools, skip_inference, tracing_span)
}

fn build_request_for_filtered_tools(
    messages: &[Message],
    tools: &HashMap<String, Arc<dyn Tool>>,
    filtered_tools: &[String],
) -> genai::chat::ChatRequest {
    let filtered_tool_refs: Vec<&dyn Tool> = tools
        .values()
        .filter(|t| filtered_tools.contains(&t.descriptor().id))
        .map(|t| t.as_ref())
        .collect();
    build_request(messages, &filtered_tool_refs)
}

fn set_agent_pending_interaction(state: &Value, interaction: Interaction) -> TrackedPatch {
    let ctx = Context::new(state, "agent_pending", "agent_loop");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.set_pending_interaction(Some(interaction));
    ctx.take_patch()
}

fn clear_agent_pending_interaction(state: &Value) -> TrackedPatch {
    let ctx = Context::new(state, "agent_pending_clear", "agent_loop");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.pending_interaction_none();
    ctx.take_patch()
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
            for text in appended_user_messages_from_tool_result(&r.execution.result) {
                msgs.push(Message::user(text));
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

    let mut thread = thread.with_patches(patches).with_messages(tool_messages);

    if let Some(interaction) = pending_interaction.clone() {
        let state = thread
            .rebuild_state()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        let patch = set_agent_pending_interaction(&state, interaction.clone());
        if !patch.patch().is_empty() {
            state_changed = true;
            thread = thread.with_patch(patch);
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
            thread = thread.with_patch(patch);
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

fn appended_user_messages_from_tool_result(result: &ToolResult) -> Vec<String> {
    let Some(raw) = result
        .metadata
        .get(crate::skills::APPEND_USER_MESSAGES_METADATA_KEY)
    else {
        return Vec::new();
    };

    match raw {
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                Vec::new()
            } else {
                vec![s.to_string()]
            }
        }
        _ => Vec::new(),
    }
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
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);

    let phase_block = run_phase_block(
        &thread,
        &mut scratchpad,
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
                &mut scratchpad,
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
        &mut scratchpad,
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
        &mut scratchpad,
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
    let scratchpad = initial_scratchpad(plugins);
    let rt_for_tools = runtime_with_tool_caller_context(&thread, &state, None)?;

    let results = if parallel {
        execute_tools_parallel_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            plugins,
            scratchpad,
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
            scratchpad,
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
    scratchpad: HashMap<String, Value>,
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
        let scratchpad = scratchpad.clone();
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
                scratchpad,
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
    mut scratchpad: HashMap<String, Value>,
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
            scratchpad.clone(),
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

        scratchpad = result.scratchpad.clone();
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

fn scratchpad_deltas_from_base(
    base: &HashMap<String, Value>,
    snapshot: &HashMap<String, Value>,
) -> Vec<(String, Option<Value>)> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    keys.extend(base.keys().cloned());
    keys.extend(snapshot.keys().cloned());

    let mut deltas = Vec::new();
    for key in keys {
        let before = base.get(&key);
        let after = snapshot.get(&key);
        if before != after {
            deltas.push((key, after.cloned()));
        }
    }
    deltas
}

fn apply_scratchpad_changes(
    base: &HashMap<String, Value>,
    changes: HashMap<String, Option<Value>>,
) -> HashMap<String, Value> {
    let mut merged = base.clone();
    for (key, value) in changes {
        if let Some(v) = value {
            merged.insert(key, v);
        } else {
            merged.remove(&key);
        }
    }
    merged
}

fn merge_parallel_scratchpad(
    base: &HashMap<String, Value>,
    results: &[ToolExecutionResult],
    policy: ScratchpadMergePolicy,
) -> Result<HashMap<String, Value>, AgentLoopError> {
    match policy {
        ScratchpadMergePolicy::Strict => {
            let mut changes: HashMap<String, Option<Value>> = HashMap::new();

            for result in results {
                for (key, proposed) in scratchpad_deltas_from_base(base, &result.scratchpad) {
                    if let Some(existing) = changes.get(&key) {
                        if existing != &proposed {
                            return Err(AgentLoopError::StateError(format!(
                                "conflicting parallel scratchpad updates for key '{}'",
                                key
                            )));
                        }
                    } else {
                        changes.insert(key, proposed);
                    }
                }
            }
            Ok(apply_scratchpad_changes(base, changes))
        }
        ScratchpadMergePolicy::DeterministicLww => {
            let mut changes: HashMap<String, Option<Value>> = HashMap::new();

            // `results` are in tool-call order, so overriding here is deterministic.
            for result in results {
                for (key, proposed) in scratchpad_deltas_from_base(base, &result.scratchpad) {
                    changes.insert(key, proposed);
                }
            }
            Ok(apply_scratchpad_changes(base, changes))
        }
    }
}

/// Result of tool execution with phase hooks.
pub struct ToolExecutionResult {
    /// The tool execution result.
    pub execution: ToolExecution,
    /// System reminders collected during execution.
    pub reminders: Vec<String>,
    /// Pending interaction if tool is waiting for user action.
    pub pending_interaction: Option<crate::state_types::Interaction>,
    /// Plugin data snapshot after this tool execution.
    pub scratchpad: HashMap<String, Value>,
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
    scratchpad: HashMap<String, Value>,
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
    step.set_scratchpad_map(scratchpad);
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
        scratchpad: step.scratchpad_snapshot(),
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
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);
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
        &mut scratchpad,
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
            &mut scratchpad,
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
                let _finalized =
                    emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
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
                    &mut scratchpad,
                    &tool_descriptors,
                    &config.plugins,
                    "llm_exec_error",
                    e.to_string(),
                )
                .await
                {
                    Ok(s) => s,
                    Err(phase_error) => {
                        let _finalized = emit_session_end(
                            thread,
                            &mut scratchpad,
                            &tool_descriptors,
                            &config.plugins,
                        )
                        .await;
                        return Err(phase_error);
                    }
                };
                let _finalized =
                    emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
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
            &mut scratchpad,
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
                    emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        }

        // Add assistant message
        let assistant_msg_id = gen_message_id();
        let step_meta = step_metadata(Some(run_id.clone()), loop_state.rounds as u32);
        thread = if result.tool_calls.is_empty() {
            thread.with_message(
                assistant_message(&result.text)
                    .with_id(assistant_msg_id.clone())
                    .with_metadata(step_meta.clone()),
            )
        } else {
            thread.with_message(
                assistant_tool_calls(&result.text, result.tool_calls.clone())
                    .with_id(assistant_msg_id.clone())
                    .with_metadata(step_meta.clone()),
            )
        };

        // Phase: StepEnd
        match emit_phase_block(
            Phase::StepEnd,
            &thread,
            &mut scratchpad,
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
                    emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
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
                emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                return Err(AgentLoopError::StateError(e.to_string()));
            }
        };
        let rt_for_tools = match runtime_with_tool_caller_context(&thread, &state, Some(config)) {
            Ok(rt) => rt,
            Err(e) => {
                emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
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
                scratchpad.data.clone(),
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
                scratchpad.data.clone(),
                None,
                Some(&rt_for_tools),
                &thread.id,
            )
            .await
        };

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                let _finalized =
                    emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        };

        if config.parallel_tools {
            scratchpad.data = match merge_parallel_scratchpad(
                &scratchpad.data,
                &results,
                config.scratchpad_merge_policy,
            ) {
                Ok(v) => v,
                Err(e) => {
                    let _finalized = emit_session_end(
                        thread,
                        &mut scratchpad,
                        &tool_descriptors,
                        &config.plugins,
                    )
                    .await;
                    return Err(e);
                }
            };
        } else if let Some(last) = results.last() {
            scratchpad.data = last.scratchpad.clone();
        }

        let session_before_apply = thread.clone();
        let applied = match apply_tool_results_to_session(
            thread,
            &results,
            Some(step_meta),
            config.parallel_tools,
        ) {
            Ok(a) => a,
            Err(e) => {
                let _finalized = emit_session_end(
                    session_before_apply,
                    &mut scratchpad,
                    &tool_descriptors,
                    &config.plugins,
                )
                .await;
                return Err(e);
            }
        };
        thread = applied.thread;

        // Pause if any tool is waiting for client response.
        if let Some(interaction) = applied.pending_interaction {
            thread =
                emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
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
            thread =
                emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
            return Err(AgentLoopError::Stopped {
                thread: Box::new(thread),
                reason,
            });
        }
    }

    // Phase: SessionEnd
    thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;

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

/// A streaming agent run with access to intermediate session checkpoints.
///
/// Intended for persistence layers to save at durable boundaries while streaming.
/// The `checkpoints` channel carries `ThreadDelta` values directly — each delta
/// contains only the new messages and patches since the last checkpoint.
pub struct StreamWithCheckpoints {
    pub events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    pub checkpoints: tokio::sync::mpsc::UnboundedReceiver<ThreadDelta>,
}

/// Run the agent loop and return a stream of `AgentEvent`s plus session checkpoints.
pub fn run_loop_stream_with_checkpoints(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> StreamWithCheckpoints {
    let (checkpoint_tx, checkpoint_rx) = tokio::sync::mpsc::unbounded_channel();
    let run_ctx = run_ctx.with_state_committer(Arc::new(ChannelStateCommitter::new(checkpoint_tx)));
    let events = run_loop_stream_impl_with_provider(
        Arc::new(client),
        config,
        thread,
        tools,
        run_ctx,
    );
    StreamWithCheckpoints {
        events,
        checkpoints: checkpoint_rx,
    }
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
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);

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

        macro_rules! terminate_stream_error {
            ($message:expr) => {{
                let (finalized, error, finish) = prepare_stream_error_termination(
                    thread,
                    &mut scratchpad,
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
            &mut scratchpad,
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

        if let Some(raw) = scratchpad
            .data
            .remove(crate::interaction::INTERACTION_RESOLUTIONS_KEY)
        {
            let resolutions: Vec<crate::interaction::InteractionResolution> =
                serde_json::from_value(raw).unwrap_or_default();
            for resolution in resolutions {
                yield AgentEvent::InteractionResolved {
                    interaction_id: resolution.interaction_id,
                    result: resolution.result,
                };
            }
        }

        // Resume pending tool execution via plugin mechanism.
        // Plugins can request tool replay during SessionStart by populating
        // `replay_tool_calls` in the step context. The loop handles actual execution
        // since only it has access to the tools map.
        let replay_calls = match scratchpad.data.get("__replay_tool_calls") {
            Some(raw) => match serde_json::from_value::<Vec<crate::types::ToolCall>>(raw.clone()) {
                Ok(calls) => calls,
                Err(e) => {
                    terminate_stream_error!(format!("failed to parse __replay_tool_calls: {e}"));
                }
            },
            None => Vec::new(),
        };
        if !replay_calls.is_empty() {
            scratchpad.data.remove("__replay_tool_calls");
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
                    scratchpad.data.clone(),
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
                scratchpad.data = replay_result.scratchpad.clone();

                if replay_result.pending_interaction.is_some() {
                    terminate_stream_error!(format!(
                        "replayed tool '{}' requested pending interaction",
                        tool_call.id
                    ));
                }

                // Append real replay tool result as a new tool message (append-only log).
                let replay_msg_id = gen_message_id();
                let real_msg =
                    tool_response(&tool_call.id, &replay_result.execution.result)
                        .with_id(replay_msg_id.clone());
                thread = thread.with_message(real_msg);

                // Preserve reminder emission semantics for replayed tool calls.
                if !replay_result.reminders.is_empty() {
                    let msgs = replay_result.reminders.iter().map(|reminder| {
                        Message::internal_system(format!(
                            "<system-reminder>{}</system-reminder>",
                            reminder
                        ))
                    });
                    thread = thread.with_messages(msgs);
                }

                if let Some(patch) = replay_result.execution.patch.clone() {
                    replay_state_changed = true;
                    thread = thread.with_patch(patch);
                }
                if !replay_result.pending_patches.is_empty() {
                    replay_state_changed = true;
                    thread = thread.with_patches(replay_result.pending_patches.clone());
                }

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
                thread = thread.with_patch(clear_patch);
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
            if let Some(raw) = scratchpad
                .data
                .remove(crate::interaction::INTERACTION_RESOLUTIONS_KEY)
            {
                let resolutions: Vec<crate::interaction::InteractionResolution> =
                    serde_json::from_value(raw).unwrap_or_default();
                for resolution in resolutions {
                    yield AgentEvent::InteractionResolved {
                        interaction_id: resolution.interaction_id,
                        result: resolution.result,
                    };
                }
            }

            // Check cancellation at the top of each iteration.
            if let Some(ref token) = run_cancellation_token {
                if token.is_cancelled() {
                    thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                    emit_run_finished_delta!();
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
                &mut scratchpad,
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
                thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                emit_run_finished_delta!();
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
                        &mut scratchpad,
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
                            thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                            emit_run_finished_delta!();
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
                            &mut scratchpad,
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
                &mut scratchpad,
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
            thread = if result.tool_calls.is_empty() {
                thread.with_message(assistant_message(&result.text).with_id(assistant_msg_id.clone()).with_metadata(step_meta.clone()))
            } else {
                thread.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()).with_id(assistant_msg_id.clone()).with_metadata(step_meta.clone()))
            };

            // Phase: StepEnd (with new context) — run plugin cleanup before yielding StepEnd
            match emit_phase_block(
                Phase::StepEnd,
                &thread,
                &mut scratchpad,
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
                thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                emit_run_finished_delta!();
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
                        scratchpad.data.clone(),
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
                        scratchpad.data.clone(),
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
                        thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                        emit_run_finished_delta!();
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

            if config.parallel_tools {
                scratchpad.data = match merge_parallel_scratchpad(
                    &scratchpad.data,
                    &results,
                    config.scratchpad_merge_policy,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
            } else if let Some(last) = results.last() {
                scratchpad.data = last.scratchpad.clone();
            }

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
                thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                emit_run_finished_delta!();
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
                thread = emit_session_end(thread, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                emit_run_finished_delta!();
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
