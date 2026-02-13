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

use crate::activity::ActivityHub;
use crate::agent::uuid_v7;
use crate::convert::{assistant_message, assistant_tool_calls, build_request, tool_response};
use crate::execute::{collect_patches, ToolExecution};
use crate::phase::{Phase, StepContext, ToolContext};
use crate::plugin::AgentPlugin;
use crate::session::Session;
use crate::state_types::{AgentState, Interaction, AGENT_STATE_PATH};
use crate::stop::{
    check_stop_conditions, StopCheckContext, StopCondition, StopConditionSpec, StopReason,
};
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::types::{Message, MessageMetadata};
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
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

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

/// Definition for the agent loop configuration.
#[derive(Clone)]
pub struct AgentDefinition {
    /// Unique identifier for this agent.
    pub id: String,
    /// Model identifier (e.g., "gpt-4", "claude-3-opus").
    pub model: String,
    /// System prompt for the LLM.
    pub system_prompt: String,
    /// Maximum number of tool call rounds before stopping.
    pub max_rounds: usize,
    /// Whether to execute tools in parallel.
    pub parallel_tools: bool,
    /// Merge policy for scratchpad updates produced by parallel tool execution.
    ///
    /// Scratchpad keys are intentionally developer-defined. Components may share
    /// namespaces/keys by convention; this policy controls how same-key updates are
    /// resolved when multiple parallel tool calls update scratchpad in one round.
    pub scratchpad_merge_policy: ScratchpadMergePolicy,
    /// Chat options for the LLM.
    pub chat_options: Option<ChatOptions>,
    /// Plugins to run during the agent loop.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Plugin references to resolve via AgentOs wiring.
    ///
    /// This keeps AgentDefinition decoupled from plugin construction/loading. The AgentOs
    /// instance decides how to map these ids to plugin instances.
    pub plugin_ids: Vec<String>,
    /// Policy references to resolve via AgentOs wiring.
    ///
    /// Policies are "guardrails" plugins. They are resolved from the same registry as plugins,
    /// but are wired ahead of non-policy plugins to run first within the non-system plugin chain.
    pub policy_ids: Vec<String>,
    /// Tool whitelist (None = all tools available).
    pub allowed_tools: Option<Vec<String>>,
    /// Tool blacklist.
    pub excluded_tools: Option<Vec<String>>,
    /// Composable stop conditions checked after each tool-call round.
    ///
    /// When empty (and `stop_condition_specs` is also empty), a default
    /// [`crate::stop::MaxRounds`] condition is created from `max_rounds`.
    /// When non-empty, `max_rounds` is ignored.
    pub stop_conditions: Vec<Arc<dyn StopCondition>>,
    /// Declarative stop condition specs, resolved to `Arc<dyn StopCondition>`
    /// at runtime. Analogous to `plugin_ids` for plugins.
    ///
    /// Specs are appended after explicit `stop_conditions` in evaluation order.
    pub stop_condition_specs: Vec<StopConditionSpec>,
}

/// Conflict resolution policy for scratchpad updates from parallel tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchpadMergePolicy {
    /// Fail the run when parallel tool executions propose different values for the same key.
    Strict,
    /// Resolve conflicts by deterministic last-writer-wins in tool-call order.
    DeterministicLww,
}

impl Default for ScratchpadMergePolicy {
    fn default() -> Self {
        Self::DeterministicLww
    }
}

/// Backwards-compatible alias.
pub type AgentConfig = AgentDefinition;

/// Optional lifecycle context for a streaming agent run.
///
/// Run-specific data (run_id, parent_run_id, etc.) should be set on
/// `session.runtime` before starting the loop. This struct only holds
/// the cancellation token which is orthogonal to the data model.
#[derive(Debug, Clone, Default)]
pub struct RunContext {
    /// Cancellation token for cooperative loop termination.
    ///
    /// When cancelled, the loop stops at the next check point and emits
    /// `RunFinish` with `StopReason::Cancelled`.
    pub cancellation_token: Option<CancellationToken>,
}

/// Runtime key: caller session id visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_SESSION_ID_KEY: &str = "__agent_tool_caller_session_id";
/// Runtime key: caller agent id visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_AGENT_ID_KEY: &str = "__agent_tool_caller_agent_id";
/// Runtime key: caller state snapshot visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_STATE_KEY: &str = "__agent_tool_caller_state";
/// Runtime key: caller message snapshot visible to tools.
pub(crate) const TOOL_RUNTIME_CALLER_MESSAGES_KEY: &str = "__agent_tool_caller_messages";

impl Default for AgentDefinition {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            parallel_tools: true,
            scratchpad_merge_policy: ScratchpadMergePolicy::default(),
            chat_options: Some(
                ChatOptions::default()
                    .with_capture_usage(true)
                    .with_capture_tool_calls(true),
            ),
            plugins: Vec::new(),
            plugin_ids: Vec::new(),
            policy_ids: Vec::new(),
            allowed_tools: None,
            excluded_tools: None,
            stop_conditions: Vec::new(),
            stop_condition_specs: Vec::new(),
        }
    }
}

impl std::fmt::Debug for AgentDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentDefinition")
            .field("id", &self.id)
            .field("model", &self.model)
            .field(
                "system_prompt",
                &format!("[{} chars]", self.system_prompt.len()),
            )
            .field("max_rounds", &self.max_rounds)
            .field("parallel_tools", &self.parallel_tools)
            .field("scratchpad_merge_policy", &self.scratchpad_merge_policy)
            .field("chat_options", &self.chat_options)
            .field("plugins", &format!("[{} plugins]", self.plugins.len()))
            .field("plugin_ids", &self.plugin_ids)
            .field("policy_ids", &self.policy_ids)
            .field("allowed_tools", &self.allowed_tools)
            .field("excluded_tools", &self.excluded_tools)
            .field(
                "stop_conditions",
                &format!("[{} conditions]", self.stop_conditions.len()),
            )
            .field("stop_condition_specs", &self.stop_condition_specs)
            .finish()
    }
}

impl AgentDefinition {
    /// Create a new definition with the given model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    /// Create a new definition with explicit id and model.
    pub fn with_id(id: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            ..Default::default()
        }
    }

    /// Set system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    /// Set max rounds.
    #[must_use]
    pub fn with_max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    /// Set parallel tool execution.
    #[must_use]
    pub fn with_parallel_tools(mut self, parallel: bool) -> Self {
        self.parallel_tools = parallel;
        self
    }

    /// Set scratchpad merge policy for parallel tool execution.
    #[must_use]
    pub fn with_scratchpad_merge_policy(mut self, policy: ScratchpadMergePolicy) -> Self {
        self.scratchpad_merge_policy = policy;
        self
    }

    /// Set chat options.
    #[must_use]
    pub fn with_chat_options(mut self, options: ChatOptions) -> Self {
        self.chat_options = Some(options);
        self
    }

    /// Set plugins.
    #[must_use]
    pub fn with_plugins(mut self, plugins: Vec<Arc<dyn AgentPlugin>>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Set plugin references to be resolved via AgentOs.
    #[must_use]
    pub fn with_plugin_ids(mut self, plugin_ids: Vec<String>) -> Self {
        self.plugin_ids = plugin_ids;
        self
    }

    /// Add a single plugin reference.
    #[must_use]
    pub fn with_plugin_id(mut self, plugin_id: impl Into<String>) -> Self {
        self.plugin_ids.push(plugin_id.into());
        self
    }

    /// Set policy references to be resolved via AgentOs.
    #[must_use]
    pub fn with_policy_ids(mut self, policy_ids: Vec<String>) -> Self {
        self.policy_ids = policy_ids;
        self
    }

    /// Add a single policy reference.
    #[must_use]
    pub fn with_policy_id(mut self, policy_id: impl Into<String>) -> Self {
        self.policy_ids.push(policy_id.into());
        self
    }

    /// Add a single plugin.
    #[must_use]
    pub fn with_plugin(mut self, plugin: Arc<dyn AgentPlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    /// Set allowed tools whitelist.
    #[must_use]
    pub fn with_allowed_tools(mut self, tools: Vec<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    /// Set excluded tools blacklist.
    #[must_use]
    pub fn with_excluded_tools(mut self, tools: Vec<String>) -> Self {
        self.excluded_tools = Some(tools);
        self
    }

    /// Add a stop condition.
    ///
    /// When any stop conditions are set, the `max_rounds` field is ignored
    /// and only explicit stop conditions are checked.
    #[must_use]
    pub fn with_stop_condition(mut self, condition: impl StopCondition + 'static) -> Self {
        self.stop_conditions.push(Arc::new(condition));
        self
    }

    /// Set all stop conditions, replacing any previously set.
    #[must_use]
    pub fn with_stop_conditions(mut self, conditions: Vec<Arc<dyn StopCondition>>) -> Self {
        self.stop_conditions = conditions;
        self
    }

    /// Add a declarative stop condition spec.
    ///
    /// Specs are resolved to `Arc<dyn StopCondition>` at runtime and
    /// appended after explicit `stop_conditions` in evaluation order.
    #[must_use]
    pub fn with_stop_condition_spec(mut self, spec: StopConditionSpec) -> Self {
        self.stop_condition_specs.push(spec);
        self
    }

    /// Set all declarative stop condition specs, replacing any previously set.
    #[must_use]
    pub fn with_stop_condition_specs(mut self, specs: Vec<StopConditionSpec>) -> Self {
        self.stop_condition_specs = specs;
        self
    }

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty() || !self.plugin_ids.is_empty() || !self.policy_ids.is_empty()
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
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<(), AgentLoopError> {
    for plugin in plugins {
        let before = phase_mutation_snapshot(step);
        plugin.on_phase(phase, step).await;
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

fn apply_pending_patches(session: Session, pending: Vec<TrackedPatch>) -> Session {
    if pending.is_empty() {
        session
    } else {
        session.with_patches(pending)
    }
}

async fn run_phase_block<R, Setup, Extract>(
    session: &Session,
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
    let mut step = scratchpad.new_step_context(session, tool_descriptors.to_vec());
    setup(&mut step);
    for phase in phases {
        emit_phase_checked(*phase, &mut step, plugins).await?;
    }
    let output = extract(&mut step);
    let pending = take_step_side_effects(scratchpad, &mut step);
    Ok((output, pending))
}

async fn emit_phase_block<Setup>(
    phase: Phase,
    session: &Session,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    setup: Setup,
) -> Result<Vec<TrackedPatch>, AgentLoopError>
where
    Setup: FnOnce(&mut StepContext<'_>),
{
    let (_, pending) = run_phase_block(
        session,
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
    session: Session,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    error_type: &'static str,
    message: String,
) -> Result<Session, AgentLoopError> {
    let pending = emit_phase_block(
        Phase::AfterInference,
        &session,
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
    let session = apply_pending_patches(session, pending);
    let pending = emit_phase_block(
        Phase::StepEnd,
        &session,
        scratchpad,
        tool_descriptors,
        plugins,
        |_| {},
    )
    .await?;
    Ok(apply_pending_patches(session, pending))
}

/// Emit SessionEnd phase and apply any resulting patches to the session.
async fn emit_session_end(
    session: Session,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
) -> Session {
    let pending = {
        let mut step = scratchpad.new_step_context(&session, tool_descriptors.to_vec());
        if let Err(e) = emit_phase_checked(Phase::SessionEnd, &mut step, plugins).await {
            tracing::warn!(error = %e, "SessionEnd plugin phase validation failed");
        }
        take_step_side_effects(scratchpad, &mut step)
    };
    apply_pending_patches(session, pending)
}

/// Build terminal error events after running SessionEnd cleanup.
async fn prepare_stream_error_termination(
    session: Session,
    scratchpad: &mut ScratchpadRuntimeData,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    run_id: &str,
    message: String,
) -> (Session, AgentEvent, AgentEvent) {
    let session = emit_session_end(session, scratchpad, tool_descriptors, plugins).await;
    let error = AgentEvent::Error { message };
    let finish = AgentEvent::RunFinish {
        thread_id: session.id.clone(),
        run_id: run_id.to_string(),
        result: None,
        stop_reason: None,
    };
    (session, error, finish)
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
        session: &'a Session,
        tools: Vec<ToolDescriptor>,
    ) -> StepContext<'a> {
        let mut step = StepContext::new(session, tools);
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

    // [2] Session Context
    for ctx in &step.session_context {
        messages.push(Message::system(ctx.clone()));
    }

    // [3+] History from session
    messages.extend(step.session.messages.clone());

    messages
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

/// Replace a placeholder tool message with the real result.
///
/// Finds the tool message matching `tool_call_id` that contains "awaiting approval"
/// and replaces its content with the real tool result message.
fn replace_placeholder_tool_message(session: &mut Session, tool_call_id: &str, real_msg: Message) {
    if let Some(msg) = session.messages.iter_mut().rev().find(|m| {
        m.role == crate::types::Role::Tool
            && m.tool_call_id.as_deref() == Some(tool_call_id)
            && m.content.contains("awaiting approval")
    }) {
        msg.content = real_msg.content;
    }
}

struct AppliedToolResults {
    session: Session,
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
    session: Session,
    results: &[ToolExecutionResult],
    metadata: Option<MessageMetadata>,
    parallel_tools: bool,
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
                vec![tool_response(&r.execution.call.id, &r.execution.result)]
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

    let mut session = session.with_patches(patches).with_messages(tool_messages);

    if let Some(interaction) = pending_interaction.clone() {
        let state = session
            .rebuild_state()
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
        let patch = set_agent_pending_interaction(&state, interaction.clone());
        if !patch.patch().is_empty() {
            state_changed = true;
            session = session.with_patch(patch);
        }
        let state_snapshot = if state_changed {
            Some(
                session
                    .rebuild_state()
                    .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
            )
        } else {
            None
        };
        return Ok(AppliedToolResults {
            session,
            pending_interaction: Some(interaction),
            state_snapshot,
        });
    }

    // If a previous run left a persisted pending interaction, clear it once we successfully
    // complete tool execution without creating a new pending interaction.
    let state = session
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
            session = session.with_patch(patch);
        }
    }

    let state_snapshot = if state_changed {
        Some(
            session
                .rebuild_state()
                .map_err(|e| AgentLoopError::StateError(e.to_string()))?,
        )
    } else {
        None
    };

    Ok(AppliedToolResults {
        session,
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

fn tool_result_metadata_from_session(session: &Session) -> Option<MessageMetadata> {
    let run_id = session
        .runtime
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .or_else(|| {
            session.messages.iter().rev().find_map(|m| {
                m.metadata
                    .as_ref()
                    .and_then(|meta| meta.run_id.as_ref().cloned())
            })
        });

    let step_index = session
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

fn next_step_index(session: &Session) -> u32 {
    session
        .messages
        .iter()
        .filter_map(|m| m.metadata.as_ref().and_then(|meta| meta.step_index))
        .max()
        .map(|v| v.saturating_add(1))
        .unwrap_or(0)
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
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, StreamResult), AgentLoopError> {
    let tool_descriptors = tool_descriptors_for_config(tools, config);
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);

    let phase_block = run_phase_block(
        &session,
        &mut scratchpad,
        &tool_descriptors,
        &config.plugins,
        &[Phase::StepStart, Phase::BeforeInference],
        |_| {},
        |step| {
            let messages = build_messages(step, &config.system_prompt);
            let filtered_tools = step
                .tools
                .iter()
                .map(|td| td.id.clone())
                .collect::<Vec<_>>();
            let skip_inference = step.skip_inference;
            let tracing_span = step.tracing_span.take();
            (messages, filtered_tools, skip_inference, tracing_span)
        },
    )
    .await?;
    let ((messages, filtered_tools, skip_inference, tracing_span), pending) = phase_block;
    let session = apply_pending_patches(session, pending);

    // Skip inference if requested
    if skip_inference {
        return Ok((
            session,
            StreamResult {
                text: String::new(),
                tool_calls: vec![],
                usage: None,
            },
        ));
    }

    // Build request with filtered tools
    let filtered_tool_refs: Vec<&dyn Tool> = tools
        .values()
        .filter(|t| filtered_tools.contains(&t.descriptor().id))
        .map(|t| t.as_ref())
        .collect();
    let request = build_request(&messages, &filtered_tool_refs);

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
                session,
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
        &session,
        &mut scratchpad,
        &tool_descriptors,
        &config.plugins,
        |step| {
            step.response = Some(result.clone());
        },
    )
    .await?;
    let session = apply_pending_patches(session, pending);

    // Add assistant message
    let step_meta = MessageMetadata {
        run_id: session
            .runtime
            .value("run_id")
            .and_then(|v| v.as_str().map(String::from)),
        step_index: Some(next_step_index(&session)),
    };
    let session = if result.tool_calls.is_empty() {
        session.with_message(assistant_message(&result.text).with_metadata(step_meta))
    } else {
        session.with_message(
            assistant_tool_calls(&result.text, result.tool_calls.clone()).with_metadata(step_meta),
        )
    };

    // Phase: StepEnd (with new context)
    let pending = emit_phase_block(
        Phase::StepEnd,
        &session,
        &mut scratchpad,
        &tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await?;
    let session = apply_pending_patches(session, pending);

    Ok((session, result))
}

/// Execute tool calls (simplified version without plugins).
///
/// This is the simpler API for tests and cases where plugins aren't needed.
pub async fn execute_tools(
    session: Session,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<Session, AgentLoopError> {
    execute_tools_with_plugins(session, result, tools, parallel, &[]).await
}

/// Execute tool calls with phase-based plugin hooks.
pub async fn execute_tools_with_config(
    session: Session,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    config: &AgentConfig,
) -> Result<Session, AgentLoopError> {
    execute_tools_with_plugins(
        session,
        result,
        tools,
        config.parallel_tools,
        &config.plugins,
    )
    .await
}

fn runtime_with_tool_caller_context(
    session: &Session,
    state: &Value,
) -> Result<carve_state::Runtime, AgentLoopError> {
    let mut rt = session.runtime.clone();
    if rt.value(TOOL_RUNTIME_CALLER_SESSION_ID_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_SESSION_ID_KEY, session.id.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_RUNTIME_CALLER_STATE_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_STATE_KEY, state.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    if rt.value(TOOL_RUNTIME_CALLER_MESSAGES_KEY).is_none() {
        rt.set(TOOL_RUNTIME_CALLER_MESSAGES_KEY, session.messages.clone())
            .map_err(|e| AgentLoopError::StateError(e.to_string()))?;
    }
    Ok(rt)
}

/// Execute tool calls with plugin hooks (backward compatible).
pub async fn execute_tools_with_plugins(
    session: Session,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
    plugins: &[Arc<dyn AgentPlugin>],
) -> Result<Session, AgentLoopError> {
    if result.tool_calls.is_empty() {
        return Ok(session);
    }

    let state = session
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let scratchpad = initial_scratchpad(plugins);
    let rt_for_tools = runtime_with_tool_caller_context(&session, &state)?;

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
            &session.id,
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
            &session.id,
        )
        .await
    }?;

    let metadata = tool_result_metadata_from_session(&session);
    let applied = apply_tool_results_to_session(session, &results, metadata, parallel)?;
    if let Some(interaction) = applied.pending_interaction {
        return Err(AgentLoopError::PendingInteraction {
            session: Box::new(applied.session),
            interaction: Box::new(interaction),
        });
    }
    Ok(applied.session)
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
    session_id: &str,
) -> Result<Vec<ToolExecutionResult>, AgentLoopError> {
    use futures::future::join_all;

    // Clone runtime for parallel tasks (Runtime is Clone).
    let runtime_owned = runtime.cloned();
    let session_id = session_id.to_string();

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();
        let tool_descriptors = tool_descriptors.to_vec();
        let scratchpad = scratchpad.clone();
        let activity_manager = activity_manager.clone();
        let rt = runtime_owned.clone();
        let sid = session_id.clone();

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
    session_id: &str,
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
            session_id,
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
    session_id: &str,
) -> Result<ToolExecutionResult, AgentLoopError> {
    // Create a session stub so plugins see the real session id and runtime.
    let mut temp_session = Session::with_initial_state(session_id, state.clone());
    if let Some(rt) = runtime {
        temp_session.runtime = rt.clone();
    }

    // Create StepContext for this tool
    let mut step = StepContext::new(&temp_session, tool_descriptors.to_vec());
    step.set_scratchpad_map(scratchpad);
    step.tool = Some(ToolContext::new(call));

    // Phase: BeforeToolExecute
    emit_phase_checked(Phase::BeforeToolExecute, &mut step, plugins).await?;

    // Check if blocked or pending
    let (execution, pending_interaction) = if step.tool_blocked() {
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
        // Execute the tool with context (instrumented with tracing span)
        let tool_span = step.tracing_span.take().unwrap_or_else(tracing::Span::none);
        let ctx = carve_state::Context::new_with_activity_manager(
            state,
            &call.id,
            format!("tool:{}", call.name),
            activity_manager,
        )
        .with_runtime(runtime);
        let result = async {
            match tool.unwrap().execute(call.arguments.clone(), &ctx).await {
                Ok(r) => r,
                Err(e) => ToolResult::error(&call.name, e.to_string()),
            }
        }
        .instrument(tool_span)
        .await;

        let patch = ctx.take_patch();
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
        session: &'a Session,
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
            session,
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
    mut session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, String), AgentLoopError> {
    let mut loop_state = LoopState::new();
    let stop_conditions = effective_stop_conditions(config);
    let mut last_text = String::new();
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);
    let run_id = session
        .runtime
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            let id = uuid_v7();
            let _ = session.runtime.set("run_id", &id);
            id
        });

    let tool_descriptors = tool_descriptors_for_config(tools, config);

    // Phase: SessionStart
    let pending = emit_phase_block(
        Phase::SessionStart,
        &session,
        &mut scratchpad,
        &tool_descriptors,
        &config.plugins,
        |_| {},
    )
    .await?;
    session = apply_pending_patches(session, pending);

    loop {
        // Phase: StepStart and BeforeInference
        let step_prepare = run_phase_block(
            &session,
            &mut scratchpad,
            &tool_descriptors,
            &config.plugins,
            &[Phase::StepStart, Phase::BeforeInference],
            |_| {},
            |step| {
                let messages = build_messages(step, &config.system_prompt);
                let filtered_tools = step
                    .tools
                    .iter()
                    .map(|td| td.id.clone())
                    .collect::<Vec<_>>();
                let skip_inference = step.skip_inference;
                let tracing_span = step.tracing_span.take();
                (messages, filtered_tools, skip_inference, tracing_span)
            },
        )
        .await;
        let ((messages, filtered_tools, skip_inference, tracing_span), pending) = match step_prepare
        {
            Ok(v) => v,
            Err(e) => {
                let _finalized =
                    emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        };
        session = apply_pending_patches(session, pending);

        if skip_inference {
            break;
        }

        // Build request with filtered tools
        let filtered_tool_refs: Vec<&dyn Tool> = tools
            .values()
            .filter(|t| filtered_tools.contains(&t.descriptor().id))
            .map(|t| t.as_ref())
            .collect();
        let request = build_request(&messages, &filtered_tool_refs);

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
                session = match emit_cleanup_phases_and_apply(
                    session.clone(),
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
                            session,
                            &mut scratchpad,
                            &tool_descriptors,
                            &config.plugins,
                        )
                        .await;
                        return Err(phase_error);
                    }
                };
                let _finalized =
                    emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
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
            &session,
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
                session = apply_pending_patches(session, pending);
            }
            Err(e) => {
                let _finalized =
                    emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        }

        // Add assistant message
        let step_meta = MessageMetadata {
            run_id: Some(run_id.clone()),
            step_index: Some(loop_state.rounds as u32),
        };
        session = if result.tool_calls.is_empty() {
            session.with_message(assistant_message(&result.text).with_metadata(step_meta.clone()))
        } else {
            session.with_message(
                assistant_tool_calls(&result.text, result.tool_calls.clone())
                    .with_metadata(step_meta.clone()),
            )
        };

        // Phase: StepEnd
        match emit_phase_block(
            Phase::StepEnd,
            &session,
            &mut scratchpad,
            &tool_descriptors,
            &config.plugins,
            |_| {},
        )
        .await
        {
            Ok(pending) => {
                session = apply_pending_patches(session, pending);
            }
            Err(e) => {
                let _finalized =
                    emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                        .await;
                return Err(e);
            }
        }

        if !result.needs_tools() {
            break;
        }

        // Execute tools with phase hooks (respect config.parallel_tools).
        let state = match session.rebuild_state() {
            Ok(s) => s,
            Err(e) => {
                emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                    .await;
                return Err(AgentLoopError::StateError(e.to_string()));
            }
        };
        let rt_for_tools = match runtime_with_tool_caller_context(&session, &state) {
            Ok(rt) => rt,
            Err(e) => {
                emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                    .await;
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
                &session.id,
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
                &session.id,
            )
            .await
        };

        let results = match results {
            Ok(r) => r,
            Err(e) => {
                let _finalized =
                    emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
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
                        session,
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

        let session_before_apply = session.clone();
        let applied = match apply_tool_results_to_session(
            session,
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
        session = applied.session;

        // Pause if any tool is waiting for client response.
        if let Some(interaction) = applied.pending_interaction {
            session =
                emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                    .await;
            return Err(AgentLoopError::PendingInteraction {
                session: Box::new(session),
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
        let stop_ctx = loop_state.to_check_context(&result, &session);
        if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
            session =
                emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins)
                    .await;
            return Err(AgentLoopError::Stopped {
                session: Box::new(session),
                reason,
            });
        }
    }

    // Phase: SessionEnd
    session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;

    Ok((session, last_text))
}

/// Run the agent loop with streaming output.
///
/// Returns a stream of AgentEvent for real-time updates.
pub fn run_loop_stream(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    run_loop_stream_impl_with_provider(
        Arc::new(client),
        config,
        session,
        tools,
        run_ctx,
        None,
        None,
    )
}

/// A streaming agent run with access to the final `Session`.
///
/// This is primarily intended for transports (HTTP, NATS) that need to persist
/// the updated session after the stream completes.
pub struct StreamWithSession {
    pub events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    pub final_session: tokio::sync::oneshot::Receiver<Session>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCheckpointReason {
    /// An assistant turn was committed to the session (final text and/or tool calls).
    AssistantTurnCommitted,
    /// Tool results were applied to the session (tool messages and patches).
    ToolResultsCommitted,
}

#[derive(Debug, Clone)]
pub struct SessionCheckpoint {
    pub reason: SessionCheckpointReason,
    pub session: Session,
}

/// A streaming agent run with access to intermediate session checkpoints.
///
/// Intended for persistence layers to save at durable boundaries while streaming.
pub struct StreamWithCheckpoints {
    pub events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    pub checkpoints: tokio::sync::mpsc::UnboundedReceiver<SessionCheckpoint>,
    pub final_session: tokio::sync::oneshot::Receiver<Session>,
}

/// Run the agent loop and return a stream of `AgentEvent`s plus the final `Session`.
///
/// The returned `final_session` receiver resolves when the stream finishes. If the
/// stream terminates in a way that consumes the session (rare error paths), the
/// receiver will be closed without a value.
pub fn run_loop_stream_with_session(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> StreamWithSession {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let events = run_loop_stream_impl_with_provider(
        Arc::new(client),
        config,
        session,
        tools,
        run_ctx,
        Some(tx),
        None,
    );
    StreamWithSession {
        events,
        final_session: rx,
    }
}

/// Run the agent loop and return a stream of `AgentEvent`s plus session checkpoints and the final `Session`.
pub fn run_loop_stream_with_checkpoints(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
) -> StreamWithCheckpoints {
    let (final_tx, final_rx) = tokio::sync::oneshot::channel();
    let (checkpoint_tx, checkpoint_rx) = tokio::sync::mpsc::unbounded_channel();
    let events = run_loop_stream_impl_with_provider(
        Arc::new(client),
        config,
        session,
        tools,
        run_ctx,
        Some(final_tx),
        Some(checkpoint_tx),
    );
    StreamWithCheckpoints {
        events,
        checkpoints: checkpoint_rx,
        final_session: final_rx,
    }
}

fn run_loop_stream_impl_with_provider(
    provider: Arc<dyn ChatStreamProvider>,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_ctx: RunContext,
    mut final_session_tx: Option<tokio::sync::oneshot::Sender<Session>>,
    checkpoint_tx: Option<tokio::sync::mpsc::UnboundedSender<SessionCheckpoint>>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut session = session;
    let mut loop_state = LoopState::new();
    let stop_conditions = effective_stop_conditions(&config);
    let cancel_token = run_ctx.cancellation_token.clone();
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

    let tool_descriptors = tool_descriptors_for_config(&tools, &config);
    let mut scratchpad = ScratchpadRuntimeData::new(&config.plugins);

        // Resolve run_id: from runtime if pre-set, otherwise generate.
        // NOTE: runtime is mutated in-place here. This is intentional —
        // runtime is transient (not persisted) and the owned-builder pattern
        // (`with_runtime`) is impractical inside the loop where `session` is
        // borrowed across yield points.
        let run_id = session.runtime.value("run_id")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| {
                let id = uuid_v7();
                // Best-effort: set into runtime (may already be set).
                let _ = session.runtime.set("run_id", &id);
                id
            });
        let parent_run_id = session.runtime.value("parent_run_id")
            .and_then(|v| v.as_str().map(String::from));

        macro_rules! terminate_stream_error {
            ($message:expr) => {{
                let (finalized, error, finish) = prepare_stream_error_termination(
                    session,
                    &mut scratchpad,
                    &tool_descriptors,
                    &config.plugins,
                    &run_id,
                    $message,
                )
                .await;
                session = finalized;
                yield error;
                yield finish;
                if let Some(tx) = final_session_tx.take() {
                    let _ = tx.send(session);
                }
                return;
            }};
        }

        // Phase: SessionStart (use scoped block to manage borrow)
        match emit_phase_block(
            Phase::SessionStart,
            &session,
            &mut scratchpad,
            &tool_descriptors,
            &config.plugins,
            |_| {},
        )
        .await
        {
            Ok(pending) => {
                session = apply_pending_patches(session, pending);
            }
            Err(e) => {
                terminate_stream_error!(e.to_string());
            }
        }

        yield AgentEvent::RunStart {
            thread_id: session.id.clone(),
            run_id: run_id.clone(),
            parent_run_id,
        };

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
                let state = match session.rebuild_state() {
                    Ok(s) => s,
                    Err(e) => {
                        terminate_stream_error!(format!(
                            "failed to rebuild state before replaying tool '{}': {e}",
                            tool_call.id
                        ));
                    }
                };

                let tool = tools.get(&tool_call.name).cloned();
                let rt_for_replay = match runtime_with_tool_caller_context(&session, &state) {
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
                    &session.id,
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

                // Replace the placeholder message with the real tool result.
                let real_msg = tool_response(&tool_call.id, &replay_result.execution.result);
                replace_placeholder_tool_message(&mut session, &tool_call.id, real_msg);

                // Preserve reminder emission semantics for replayed tool calls.
                if !replay_result.reminders.is_empty() {
                    let msgs = replay_result.reminders.iter().map(|reminder| {
                        Message::internal_system(format!(
                            "<system-reminder>{}</system-reminder>",
                            reminder
                        ))
                    });
                    session = session.with_messages(msgs);
                }

                if let Some(patch) = replay_result.execution.patch.clone() {
                    replay_state_changed = true;
                    session = session.with_patch(patch);
                }
                if !replay_result.pending_patches.is_empty() {
                    replay_state_changed = true;
                    session = session.with_patches(replay_result.pending_patches.clone());
                }

                yield AgentEvent::ToolCallDone {
                    id: tool_call.id.clone(),
                    result: replay_result.execution.result,
                    patch: replay_result.execution.patch,
                };
            }

            // Clear pending_interaction state after replaying tools.
            let state = match session.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(format!("failed to rebuild state after replay: {e}"));
                }
            };

            let clear_patch = clear_agent_pending_interaction(&state);
            if !clear_patch.patch().is_empty() {
                replay_state_changed = true;
                session = session.with_patch(clear_patch);
            }

            if replay_state_changed {
                let snapshot = match session.rebuild_state() {
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
            // Check cancellation at the top of each iteration.
            if let Some(ref token) = cancel_token {
                if token.is_cancelled() {
                    session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                    yield AgentEvent::RunFinish {
                        thread_id: session.id.clone(),
                        run_id: run_id.clone(),
                        result: None,
                        stop_reason: Some(StopReason::Cancelled),
                    };
                    if let Some(tx) = final_session_tx.take() {
                        let _ = tx.send(session);
                    }
                    return;
                }
            }

            // Phase: StepStart and BeforeInference (collect messages and tools filter)
            let step_prepare = run_phase_block(
                &session,
                &mut scratchpad,
                &tool_descriptors,
                &config.plugins,
                &[Phase::StepStart, Phase::BeforeInference],
                |_| {},
                |step| {
                    let messages = build_messages(step, &config.system_prompt);
                    let filtered_tools = step.tools.iter().map(|td| td.id.clone()).collect::<Vec<_>>();
                    let skip_inference = step.skip_inference;
                    let tracing_span = step.tracing_span.take();
                    (messages, filtered_tools, skip_inference, tracing_span)
                },
            )
            .await;
            let ((messages, filtered_tools, skip_inference, tracing_span), pending) =
                match step_prepare {
                    Ok(v) => v,
                    Err(e) => {
                        terminate_stream_error!(e.to_string());
                    }
                };
            session = apply_pending_patches(session, pending);

            // Skip inference if requested
            if skip_inference {
                session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                yield AgentEvent::RunFinish {
                    thread_id: session.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: Some(StopReason::PluginRequested),
                };
                if let Some(tx) = final_session_tx.take() {
                    let _ = tx.send(session);
                }
                return;
            }

            // Build request with filtered tools
            let filtered_tool_refs: Vec<&dyn Tool> = tools
                .values()
                .filter(|t| filtered_tools.contains(&t.descriptor().id))
                .map(|t| t.as_ref())
                .collect();
            let request = build_request(&messages, &filtered_tool_refs);

            // Step boundary: starting LLM call
            yield AgentEvent::StepStart;

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
                        session.clone(),
                        &mut scratchpad,
                        &tool_descriptors,
                        &config.plugins,
                        "llm_stream_start_error",
                        e.to_string(),
                    )
                    .await
                    {
                        Ok(next_session) => {
                            session = next_session;
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
                let next_event = if let Some(ref token) = cancel_token {
                    tokio::select! {
                        _ = token.cancelled() => {
                            session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                            yield AgentEvent::RunFinish {
                                thread_id: session.id.clone(),
                                run_id: run_id.clone(),
                                result: None,
                                stop_reason: Some(StopReason::Cancelled),
                            };
                            if let Some(tx) = final_session_tx.take() {
                                let _ = tx.send(session);
                            }
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
                            session.clone(),
                            &mut scratchpad,
                            &tool_descriptors,
                            &config.plugins,
                            "llm_stream_event_error",
                            e.to_string(),
                        )
                        .await
                        {
                            Ok(next_session) => {
                                session = next_session;
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
                &session,
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
                    session = apply_pending_patches(session, pending);
                }
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            }

            // Add assistant message with run/step metadata.
            let step_meta = MessageMetadata {
                run_id: Some(run_id.clone()),
                step_index: Some(loop_state.rounds as u32),
            };
            session = if result.tool_calls.is_empty() {
                session.with_message(assistant_message(&result.text).with_metadata(step_meta.clone()))
            } else {
                session.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()).with_metadata(step_meta.clone()))
            };

            // Phase: StepEnd (with new context) — run plugin cleanup before yielding StepEnd
            match emit_phase_block(
                Phase::StepEnd,
                &session,
                &mut scratchpad,
                &tool_descriptors,
                &config.plugins,
                |_| {},
            )
            .await
            {
                Ok(pending) => {
                    session = apply_pending_patches(session, pending);
                }
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            }

            if let Some(tx) = checkpoint_tx.as_ref() {
                let _ = tx.send(SessionCheckpoint {
                    reason: SessionCheckpointReason::AssistantTurnCommitted,
                    session: session.clone(),
                });
            }

            // Step boundary: finished LLM call
            yield AgentEvent::StepEnd;

            // Check if we need to execute tools
            if !result.needs_tools() {
                session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;

                let result_value = if result.text.is_empty() {
                    None
                } else {
                    Some(serde_json::json!({"response": result.text}))
                };
                yield AgentEvent::RunFinish {
                    thread_id: session.id.clone(),
                    run_id: run_id.clone(),
                    result: result_value,
                    stop_reason: Some(StopReason::NaturalEnd),
                };
                if let Some(tx) = final_session_tx.take() {
                    let _ = tx.send(session);
                }
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
            let state = match session.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };

            let rt_for_tools = match runtime_with_tool_caller_context(&session, &state) {
                Ok(rt) => rt,
                Err(e) => {
                    terminate_stream_error!(e.to_string());
                }
            };
            let sid_for_tools = session.id.clone();
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
                        if let Some(ref token) = cancel_token {
                            token.cancelled().await;
                        } else {
                            futures::future::pending::<()>().await;
                        }
                    } => {
                        session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                        yield AgentEvent::RunFinish {
                            thread_id: session.id.clone(),
                            run_id: run_id.clone(),
                            result: None,
                            stop_reason: Some(StopReason::Cancelled),
                        };
                        if let Some(tx) = final_session_tx.take() {
                            let _ = tx.send(session);
                        }
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
                    yield AgentEvent::Pending {
                        interaction: interaction.clone(),
                    };
                }
            }
            let session_before_apply = session.clone();
            let applied = match apply_tool_results_to_session(
                session,
                &results,
                Some(step_meta),
                config.parallel_tools,
            ) {
                Ok(a) => a,
                Err(e) => {
                    session = session_before_apply;
                    terminate_stream_error!(e.to_string());
                }
            };
            session = applied.session;

            if let Some(tx) = checkpoint_tx.as_ref() {
                let _ = tx.send(SessionCheckpoint {
                    reason: SessionCheckpointReason::ToolResultsCommitted,
                    session: session.clone(),
                });
            }

            // Emit non-pending tool results (pending ones pause the run).
            for exec_result in &results {
                if exec_result.pending_interaction.is_none() {
                    yield AgentEvent::ToolCallDone {
                        id: exec_result.execution.call.id.clone(),
                        result: exec_result.execution.result.clone(),
                        patch: exec_result.execution.patch.clone(),
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
                session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                yield AgentEvent::RunFinish {
                    thread_id: session.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: None, // Pause, not a stop
                };
                if let Some(tx) = final_session_tx.take() {
                    let _ = tx.send(session);
                }
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
            let stop_ctx = loop_state.to_check_context(&result, &session);
            if let Some(reason) = check_stop_conditions(&stop_conditions, &stop_ctx) {
                session = emit_session_end(session, &mut scratchpad, &tool_descriptors, &config.plugins).await;
                yield AgentEvent::RunFinish {
                    thread_id: session.id.clone(),
                    run_id: run_id.clone(),
                    result: None,
                    stop_reason: Some(reason),
                };
                if let Some(tx) = final_session_tx.take() {
                    let _ = tx.send(session);
                }
                return;
            }
        }
    })
}

/// Single round execution for fine-grained control.
///
/// This allows callers to control the loop manually.
#[derive(Debug)]
pub enum RoundResult {
    /// LLM responded with text, no tools needed.
    Done { session: Session, response: String },
    /// LLM requested tool calls, tools have been executed.
    ToolsExecuted {
        session: Session,
        text: String,
        tool_calls: Vec<crate::types::ToolCall>,
    },
}

/// Run a single round of the agent loop.
///
/// This gives you fine-grained control over the loop.
pub async fn run_round(
    client: &Client,
    config: &AgentConfig,
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<RoundResult, AgentLoopError> {
    // Run one step
    let (session, result) = run_step(client, config, session, tools).await?;

    if !result.needs_tools() {
        return Ok(RoundResult::Done {
            session,
            response: result.text,
        });
    }

    // Execute tools
    let session = execute_tools_with_config(session, &result, tools, config).await?;

    Ok(RoundResult::ToolsExecuted {
        session,
        text: result.text,
        tool_calls: result.tool_calls,
    })
}

/// Error type for agent loop operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentLoopError {
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("State error: {0}")]
    StateError(String),
    /// The agent loop terminated normally due to a stop condition.
    ///
    /// This is not an error but a structured stop with a reason. The session
    /// is included so callers can inspect final state.
    #[error("Agent stopped: {reason:?}")]
    Stopped {
        session: Box<Session>,
        reason: StopReason,
    },
    /// Pending user interaction; execution should pause until the client responds.
    ///
    /// The returned `session` includes any patches applied up to the point where the
    /// interaction was requested (including persisting the pending interaction).
    #[error("Pending interaction: {id} ({action})", id = interaction.id, action = interaction.action)]
    PendingInteraction {
        session: Box<Session>,
        interaction: Box<Interaction>,
    },
}

/// Helper to create a tool map from an iterator of tools.
pub fn tool_map<I, T>(tools: I) -> HashMap<String, Arc<dyn Tool>>
where
    I: IntoIterator<Item = T>,
    T: Tool + 'static,
{
    tools
        .into_iter()
        .map(|t| {
            let name = t.descriptor().id.clone();
            (name, Arc::new(t) as Arc<dyn Tool>)
        })
        .collect()
}

/// Helper to create a tool map from Arc<dyn Tool>.
pub fn tool_map_from_arc<I>(tools: I) -> HashMap<String, Arc<dyn Tool>>
where
    I: IntoIterator<Item = Arc<dyn Tool>>,
{
    tools
        .into_iter()
        .map(|t| (t.descriptor().id.clone(), t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::ActivityHub;
    use crate::phase::Phase;
    use crate::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use async_trait::async_trait;
    use carve_state::{ActivityManager, Context, Op, Patch};
    use carve_state_derive::State;
    use genai::chat::{ChatStreamEvent, MessageContent, StreamChunk, StreamEnd, ToolChunk, Usage};
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use tokio::sync::Notify;

    #[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
    struct TestCounterState {
        counter: i64,
    }

    #[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
    struct ActivityProgressState {
        progress: f64,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("echo", "Echo", "Echo the input").with_parameters(json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }))
        }

        async fn execute(&self, args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let msg = args["message"].as_str().unwrap_or("no message");
            Ok(ToolResult::success("echo", json!({ "echoed": msg })))
        }
    }

    struct RuntimeSnapshotTool;

    #[async_trait]
    impl Tool for RuntimeSnapshotTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(
                "runtime_snapshot",
                "Runtime Snapshot",
                "Return tool runtime caller context",
            )
        }

        async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let rt = ctx.runtime_ref().expect("runtime should exist");
            let session_id = rt
                .value(TOOL_RUNTIME_CALLER_SESSION_ID_KEY)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let state = rt
                .value(TOOL_RUNTIME_CALLER_STATE_KEY)
                .cloned()
                .unwrap_or(Value::Null);
            let messages_len = rt
                .value(TOOL_RUNTIME_CALLER_MESSAGES_KEY)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            Ok(ToolResult::success(
                "runtime_snapshot",
                json!({
                    "session_id": session_id,
                    "state": state,
                    "messages_len": messages_len
                }),
            ))
        }
    }

    struct ActivityGateTool {
        id: String,
        stream_id: String,
        ready: Arc<Notify>,
        proceed: Arc<Notify>,
    }

    #[async_trait]
    impl Tool for ActivityGateTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new(&self.id, "Activity Gate", "Emits activity updates")
        }

        async fn execute(&self, _args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let activity = ctx.activity(self.stream_id.clone(), "progress");
            let progress = activity.state::<ActivityProgressState>("");
            progress.set_progress(0.1);
            self.ready.notify_one();
            self.proceed.notified().await;
            progress.set_progress(1.0);
            Ok(ToolResult::success(&self.id, json!({ "ok": true })))
        }
    }

    fn scratchpad_map(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    }

    fn scratchpad_result(call_id: &str, scratchpad: HashMap<String, Value>) -> ToolExecutionResult {
        ToolExecutionResult {
            execution: crate::execute::ToolExecution {
                call: crate::types::ToolCall::new(call_id, "test_tool", json!({})),
                result: ToolResult::success("test_tool", json!({"ok": true})),
                patch: None,
            },
            reminders: Vec::new(),
            pending_interaction: None,
            scratchpad,
            pending_patches: Vec::new(),
        }
    }

    fn skill_activation_result(
        call_id: &str,
        skill_id: &str,
        instruction: Option<&str>,
    ) -> ToolExecutionResult {
        let patch = instruction.map(|text| {
            carve_state::TrackedPatch::new(carve_state::Patch::new().with_op(carve_state::Op::set(
                carve_state::path!("skills", "instructions", skill_id),
                json!(text),
            )))
        });
        let mut result =
            ToolResult::success("skill", json!({ "activated": true, "skill_id": skill_id }));
        if let Some(text) = instruction {
            result = result.with_metadata(
                crate::skills::APPEND_USER_MESSAGES_METADATA_KEY,
                json!([text]),
            );
        }

        ToolExecutionResult {
            execution: crate::execute::ToolExecution {
                call: crate::types::ToolCall::new(call_id, "skill", json!({ "skill": skill_id })),
                result,
                patch,
            },
            reminders: Vec::new(),
            pending_interaction: None,
            scratchpad: HashMap::new(),
            pending_patches: Vec::new(),
        }
    }

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.max_rounds, 10);
        assert!(config.parallel_tools);
        assert_eq!(
            config.scratchpad_merge_policy,
            ScratchpadMergePolicy::DeterministicLww
        );
        assert!(config.system_prompt.is_empty());
    }

    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("gpt-4")
            .with_max_rounds(5)
            .with_parallel_tools(false)
            .with_scratchpad_merge_policy(ScratchpadMergePolicy::Strict)
            .with_system_prompt("You are helpful.");

        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_rounds, 5);
        assert!(!config.parallel_tools);
        assert_eq!(
            config.scratchpad_merge_policy,
            ScratchpadMergePolicy::Strict
        );
        assert_eq!(config.system_prompt, "You are helpful.");
    }

    #[test]
    fn test_tool_map() {
        let tools = tool_map([EchoTool]);

        assert!(tools.contains_key("echo"));
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_tool_map_from_arc() {
        let echo: Arc<dyn Tool> = Arc::new(EchoTool);
        let tools = tool_map_from_arc([echo]);

        assert!(tools.contains_key("echo"));
    }

    #[test]
    fn test_agent_loop_error_display() {
        let err = AgentLoopError::LlmError("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = AgentLoopError::Stopped {
            session: Box::new(Session::new("test")),
            reason: StopReason::MaxRoundsReached,
        };
        assert!(err.to_string().contains("MaxRoundsReached"));
    }

    #[test]
    fn test_execute_tools_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Hello".to_string(),
                tool_calls: vec![],
                usage: None,
            };
            let tools = HashMap::new();

            let session = execute_tools(session, &result, &tools, true).await.unwrap();
            assert_eq!(session.message_count(), 0);
        });
    }

    #[test]
    fn test_execute_tools_with_calls() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Calling tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "hello"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.messages[0].role, crate::types::Role::Tool);
        });
    }

    #[test]
    fn test_execute_tools_injects_caller_runtime_context_for_tools() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("caller-s", json!({"k":"v"}))
                .with_message(crate::Message::user("hello"));
            let result = StreamResult {
                text: "Calling tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "runtime_snapshot",
                    json!({}),
                )],
                usage: None,
            };
            let tools = tool_map([RuntimeSnapshotTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();
            assert_eq!(session.message_count(), 2);
            let tool_msg = session
                .messages
                .last()
                .expect("tool result message should exist");
            let tool_result: ToolResult =
                serde_json::from_str(&tool_msg.content).expect("tool result json");
            assert_eq!(tool_result.status, crate::ToolStatus::Success);
            assert_eq!(tool_result.data["session_id"], json!("caller-s"));
            assert_eq!(tool_result.data["state"]["k"], json!("v"));
            assert_eq!(tool_result.data["messages_len"], json!(1));
        });
    }

    #[tokio::test]
    async fn test_activity_event_emitted_before_tool_completion() {
        use crate::stream::AgentEvent;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(tx));

        let ready = Arc::new(Notify::new());
        let proceed = Arc::new(Notify::new());
        let tool = ActivityGateTool {
            id: "activity_gate".to_string(),
            stream_id: "stream_gate".to_string(),
            ready: ready.clone(),
            proceed: proceed.clone(),
        };

        let call = crate::types::ToolCall::new("call_1", "activity_gate", json!({}));
        let descriptors = vec![tool.descriptor()];
        let plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
        let state = json!({});

        let mut tool_future = Box::pin(execute_single_tool_with_phases(
            Some(&tool),
            &call,
            &state,
            &descriptors,
            &plugins,
            HashMap::new(),
            Some(activity_manager),
            None,
            "test",
        ));

        tokio::select! {
            _ = ready.notified() => {
                let event = rx.recv().await.expect("activity event");
                match event {
                    AgentEvent::ActivitySnapshot { message_id, content, .. } => {
                        assert_eq!(message_id, "stream_gate");
                        assert_eq!(content["progress"], 0.1);
                    }
                    _ => panic!("Expected ActivitySnapshot"),
                }
                proceed.notify_one();
            }
            _res = &mut tool_future => {
                panic!("Tool finished before activity event");
            }
        }

        let result = tool_future.await.expect("tool execution should succeed");
        assert!(result.execution.result.is_success());
    }

    #[tokio::test]
    async fn test_parallel_tools_emit_activity_before_completion() {
        use crate::stream::AgentEvent;
        use std::collections::HashSet;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(tx));

        let ready_a = Arc::new(Notify::new());
        let proceed_a = Arc::new(Notify::new());
        let tool_a = ActivityGateTool {
            id: "activity_gate_a".to_string(),
            stream_id: "stream_gate_a".to_string(),
            ready: ready_a.clone(),
            proceed: proceed_a.clone(),
        };

        let ready_b = Arc::new(Notify::new());
        let proceed_b = Arc::new(Notify::new());
        let tool_b = ActivityGateTool {
            id: "activity_gate_b".to_string(),
            stream_id: "stream_gate_b".to_string(),
            ready: ready_b.clone(),
            proceed: proceed_b.clone(),
        };

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(tool_a.id.clone(), Arc::new(tool_a));
        tools.insert(tool_b.id.clone(), Arc::new(tool_b));

        let calls = vec![
            crate::types::ToolCall::new("call_a", "activity_gate_a", json!({})),
            crate::types::ToolCall::new("call_b", "activity_gate_b", json!({})),
        ];
        let tool_descriptors: Vec<ToolDescriptor> =
            tools.values().map(|t| t.descriptor().clone()).collect();
        let plugins: Vec<Arc<dyn AgentPlugin>> = Vec::new();
        let state = json!({});

        // Spawn the tool execution so it actually starts running while we await activity events.
        let tools_for_task = tools.clone();
        let calls_for_task = calls.clone();
        let tool_descriptors_for_task = tool_descriptors.clone();
        let plugins_for_task = plugins.clone();
        let state_for_task = state.clone();
        let handle = tokio::spawn(async move {
            execute_tools_parallel_with_phases(
                &tools_for_task,
                &calls_for_task,
                &state_for_task,
                &tool_descriptors_for_task,
                &plugins_for_task,
                HashMap::new(),
                Some(activity_manager),
                None,
                "test",
            )
            .await
            .expect("parallel tool execution should succeed")
        });

        let ((), ()) = tokio::join!(ready_a.notified(), ready_b.notified());

        // Both tools have emitted their first activity update; observe both snapshots
        // before unblocking them.
        let mut seen: HashSet<String> = HashSet::new();
        while seen.len() < 2 {
            match rx.recv().await.expect("activity event") {
                AgentEvent::ActivitySnapshot {
                    message_id,
                    content,
                    ..
                } => {
                    assert_eq!(content["progress"], 0.1);
                    seen.insert(message_id);
                }
                other => panic!("Expected ActivitySnapshot, got {:?}", other),
            }
        }
        assert!(seen.contains("stream_gate_a"));
        assert!(seen.contains("stream_gate_b"));

        proceed_a.notify_one();
        proceed_b.notify_one();

        let results = handle.await.expect("task join");
        assert_eq!(results.len(), 2);
        for r in results {
            assert!(r.execution.result.is_success());
        }
    }

    struct CounterTool;

    #[async_trait]
    impl Tool for CounterTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("counter", "Counter", "Increment a counter").with_parameters(
                json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "integer" }
                    }
                }),
            )
        }

        async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let amount = args["amount"].as_i64().unwrap_or(1);

            let state = ctx.state::<TestCounterState>("");
            let current = state.counter().unwrap_or(0);
            let new_value = current + amount;

            state.set_counter(new_value);

            Ok(ToolResult::success(
                "counter",
                json!({ "new_value": new_value }),
            ))
        }
    }

    #[test]
    fn test_execute_tools_with_state_changes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "Incrementing".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "counter",
                    json!({"amount": 5}),
                )],
                usage: None,
            };
            let tools = tool_map([CounterTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.patch_count(), 1);

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }

    #[test]
    fn test_round_result_variants() {
        let session = Session::new("test");

        let done = RoundResult::Done {
            session: session.clone(),
            response: "Hello".to_string(),
        };

        let tools_executed = RoundResult::ToolsExecuted {
            session,
            text: "Calling tools".to_string(),
            tool_calls: vec![],
        };

        match done {
            RoundResult::Done { response, .. } => assert_eq!(response, "Hello"),
            _ => panic!("Expected Done"),
        }

        match tools_executed {
            RoundResult::ToolsExecuted { text, .. } => assert_eq!(text, "Calling tools"),
            _ => panic!("Expected ToolsExecuted"),
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("failing", "Failing Tool", "Always fails")
        }

        async fn execute(&self, _args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            Err(ToolError::ExecutionFailed(
                "Intentional failure".to_string(),
            ))
        }
    }

    #[test]
    fn test_execute_tools_with_failing_tool() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Calling failing tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new("call_1", "failing", json!({}))],
                usage: None,
            };
            let tools = tool_map([FailingTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(msg.content.contains("error") || msg.content.contains("fail"));
        });
    }

    // ============================================================================
    // Phase-based Plugin Tests
    // ============================================================================

    struct TestPhasePlugin {
        id: String,
    }

    impl TestPhasePlugin {
        fn new(id: impl Into<String>) -> Self {
            Self { id: id.into() }
        }
    }

    #[async_trait]
    impl AgentPlugin for TestPhasePlugin {
        fn id(&self) -> &str {
            &self.id
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            match phase {
                Phase::StepStart => {
                    step.system("Test system context");
                }
                Phase::BeforeInference => {
                    step.session("Test session context");
                }
                Phase::AfterToolExecute => {
                    if step.tool_name() == Some("echo") {
                        step.reminder("Check the echo result");
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_agent_config_with_phase_plugin() {
        let plugin: Arc<dyn AgentPlugin> = Arc::new(TestPhasePlugin::new("test"));
        let config = AgentConfig::new("gpt-4").with_plugin(plugin);

        assert!(config.has_plugins());
        assert_eq!(config.plugins.len(), 1);
    }

    struct BlockingPhasePlugin;

    #[async_trait]
    impl AgentPlugin for BlockingPhasePlugin {
        fn id(&self) -> &str {
            "blocker"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeToolExecute && step.tool_name() == Some("echo") {
                step.block("Echo tool is blocked");
            }
        }
    }

    #[test]
    fn test_execute_tools_with_blocking_phase_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Blocked".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(BlockingPhasePlugin)];

            let session = execute_tools_with_plugins(session, &result, &tools, true, &plugins)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("blocked") || msg.content.contains("Error"),
                "Expected blocked/error in message, got: {}",
                msg.content
            );
        });
    }

    struct InvalidAfterToolMutationPlugin;

    #[async_trait]
    impl AgentPlugin for InvalidAfterToolMutationPlugin {
        fn id(&self) -> &str {
            "invalid_after_tool_mutation"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::AfterToolExecute {
                step.block("too late");
            }
        }
    }

    #[test]
    fn test_execute_tools_rejects_tool_gate_mutation_outside_before_tool_execute() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "invalid".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(InvalidAfterToolMutationPlugin)];

            let err = execute_tools_with_plugins(session, &result, &tools, true, &plugins)
                .await
                .expect_err("phase mutation outside BeforeToolExecute should fail");

            assert!(
                matches!(
                    err,
                    AgentLoopError::StateError(ref message)
                    if message.contains("mutated tool gate outside BeforeToolExecute")
                ),
                "unexpected error: {err:?}"
            );
        });
    }

    struct ReminderPhasePlugin;

    #[async_trait]
    impl AgentPlugin for ReminderPhasePlugin {
        fn id(&self) -> &str {
            "reminder"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::AfterToolExecute {
                step.reminder("Tool execution completed");
            }
        }
    }

    #[test]
    fn test_execute_tools_with_reminder_phase_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "With reminder".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ReminderPhasePlugin)];

            let session = execute_tools_with_plugins(session, &result, &tools, true, &plugins)
                .await
                .unwrap();

            // Should have tool response + reminder message
            assert_eq!(session.message_count(), 2);
            assert!(session.messages[1].content.contains("system-reminder"));
            assert!(session.messages[1]
                .content
                .contains("Tool execution completed"));
        });
    }

    #[test]
    fn test_build_messages_with_context() {
        let session = Session::new("test").with_message(Message::user("Hello"));
        let tool_descriptors = vec![ToolDescriptor::new("test", "Test", "Test tool")];
        let mut step = StepContext::new(&session, tool_descriptors);

        step.system("System context 1");
        step.system("System context 2");
        step.session("Session context");

        let messages = build_messages(&step, "Base system prompt");

        assert_eq!(messages.len(), 3);
        assert!(messages[0].content.contains("Base system prompt"));
        assert!(messages[0].content.contains("System context 1"));
        assert!(messages[0].content.contains("System context 2"));
        assert_eq!(messages[1].content, "Session context");
        assert_eq!(messages[2].content, "Hello");
    }

    #[test]
    fn test_build_messages_empty_system() {
        let session = Session::new("test").with_message(Message::user("Hello"));
        let step = StepContext::new(&session, vec![]);

        let messages = build_messages(&step, "");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello");
    }

    struct ToolFilterPlugin;

    #[async_trait]
    impl AgentPlugin for ToolFilterPlugin {
        fn id(&self) -> &str {
            "filter"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeInference {
                step.exclude("dangerous_tool");
            }
        }
    }

    #[test]
    fn test_tool_filtering_via_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let tool_descriptors = vec![
                ToolDescriptor::new("safe_tool", "Safe", "Safe tool"),
                ToolDescriptor::new("dangerous_tool", "Dangerous", "Dangerous tool"),
            ];
            let mut step = StepContext::new(&session, tool_descriptors);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ToolFilterPlugin)];

            emit_phase_checked(Phase::BeforeInference, &mut step, &plugins)
                .await
                .expect("BeforeInference should not fail");

            assert_eq!(step.tools.len(), 1);
            assert_eq!(step.tools[0].id, "safe_tool");
        });
    }

    #[test]
    fn test_scratchpad_initialization() {
        struct DataPlugin;

        #[async_trait]
        impl AgentPlugin for DataPlugin {
            fn id(&self) -> &str {
                "data"
            }

            async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("plugin_config", json!({"enabled": true})))
            }
        }

        let session = Session::new("test");
        let mut step = StepContext::new(&session, vec![]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(DataPlugin)];
        let runtime_data = ScratchpadRuntimeData::new(&plugins);
        step.set_scratchpad_map(runtime_data.data);

        let config: Option<serde_json::Map<String, Value>> = step.scratchpad_get("plugin_config");
        assert!(config.is_some());
        assert_eq!(config.unwrap()["enabled"], true);
    }

    #[tokio::test]
    async fn test_plugin_initial_data_available_in_before_tool_execute() {
        struct GuardedPlugin;

        #[async_trait]
        impl AgentPlugin for GuardedPlugin {
            fn id(&self) -> &str {
                "guarded"
            }

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("allow_exec", json!(true)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeToolExecute
                    && step.scratchpad_get::<bool>("allow_exec") != Some(true)
                {
                    step.block("missing plugin initial data");
                }
            }
        }

        let tool = EchoTool;
        let call = crate::types::ToolCall::new("call_1", "echo", json!({ "message": "hello" }));
        let state = json!({});
        let tool_descriptors = vec![tool.descriptor()];
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(GuardedPlugin)];

        let result = execute_single_tool_with_phases(
            Some(&tool),
            &call,
            &state,
            &tool_descriptors,
            &plugins,
            initial_scratchpad(&plugins),
            None,
            None,
            "test",
        )
        .await
        .expect("tool execution should succeed");

        assert!(result.execution.result.is_success());
    }

    #[tokio::test]
    async fn test_plugin_initial_scratchpad_available_in_before_tool_execute() {
        struct GuardedScratchpadPlugin;

        #[async_trait]
        impl AgentPlugin for GuardedScratchpadPlugin {
            fn id(&self) -> &str {
                "guarded_scratchpad"
            }

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("allow_exec_v2", json!(true)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeToolExecute
                    && step.scratchpad_get::<bool>("allow_exec_v2") != Some(true)
                {
                    step.block("missing plugin initial scratchpad");
                }
            }
        }

        let tool = EchoTool;
        let call = crate::types::ToolCall::new("call_1", "echo", json!({ "message": "hello" }));
        let state = json!({});
        let tool_descriptors = vec![tool.descriptor()];
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(GuardedScratchpadPlugin)];

        let result = execute_single_tool_with_phases(
            Some(&tool),
            &call,
            &state,
            &tool_descriptors,
            &plugins,
            initial_scratchpad(&plugins),
            None,
            None,
            "test",
        )
        .await
        .expect("tool execution should succeed");

        assert!(result.execution.result.is_success());
    }

    #[tokio::test]
    async fn test_plugin_sees_real_session_id_and_runtime_in_tool_phase() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static VERIFIED: AtomicBool = AtomicBool::new(false);

        struct SessionCheckPlugin;

        #[async_trait]
        impl AgentPlugin for SessionCheckPlugin {
            fn id(&self) -> &str {
                "session_check"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeToolExecute {
                    assert_eq!(step.session.id, "real-session-42");
                    assert_eq!(step.session.runtime.value("user_id"), Some(&json!("u-abc")),);
                    VERIFIED.store(true, Ordering::SeqCst);
                }
            }
        }

        VERIFIED.store(false, Ordering::SeqCst);

        let tool = EchoTool;
        let call = crate::types::ToolCall::new("call_1", "echo", json!({ "message": "hi" }));
        let state = json!({});
        let tool_descriptors = vec![tool.descriptor()];
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(SessionCheckPlugin)];

        let mut rt = carve_state::Runtime::new();
        rt.set("user_id", "u-abc").unwrap();

        let result = execute_single_tool_with_phases(
            Some(&tool),
            &call,
            &state,
            &tool_descriptors,
            &plugins,
            HashMap::new(),
            None,
            Some(&rt),
            "real-session-42",
        )
        .await
        .expect("tool execution should succeed");

        assert!(result.execution.result.is_success());
        assert!(VERIFIED.load(Ordering::SeqCst), "plugin did not run");
    }

    #[tokio::test]
    async fn test_scratchpad_persists_across_phase_contexts() {
        struct LifecyclePlugin;

        #[async_trait]
        impl AgentPlugin for LifecyclePlugin {
            fn id(&self) -> &str {
                "lifecycle"
            }

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("seed", json!(1)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::SessionStart => {
                        step.scratchpad_set("seed", 2);
                    }
                    Phase::StepStart => {
                        if step.scratchpad_get::<i64>("seed") == Some(2) {
                            step.system("seed_visible");
                        } else {
                            step.system("seed_missing");
                        }
                    }
                    _ => {}
                }
            }
        }

        let session = Session::new("test");
        let tools = vec![];
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(LifecyclePlugin)];
        let mut scratchpad = ScratchpadRuntimeData::new(&plugins);

        let mut session_step = scratchpad.new_step_context(&session, tools.clone());
        emit_phase_checked(Phase::SessionStart, &mut session_step, &plugins)
            .await
            .expect("SessionStart should not fail");
        scratchpad.sync_from_step(&session_step);

        let mut step = scratchpad.new_step_context(&session, tools);
        emit_phase_checked(Phase::StepStart, &mut step, &plugins)
            .await
            .expect("StepStart should not fail");

        assert_eq!(step.system_context, vec!["seed_visible".to_string()]);
    }

    #[tokio::test]
    async fn test_run_phase_block_executes_phases_extracts_output_and_commits_side_effects() {
        struct PhaseBlockPlugin {
            phases: Arc<Mutex<Vec<Phase>>>,
        }

        #[async_trait]
        impl AgentPlugin for PhaseBlockPlugin {
            fn id(&self) -> &str {
                "phase_block"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                self.phases.lock().unwrap().push(phase);
                match phase {
                    Phase::StepStart => {
                        step.system("from_step_start");
                        step.scratchpad_set("phase_block_seen", true);
                    }
                    Phase::BeforeInference => {
                        step.skip_inference = true;
                        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                            carve_state::path!("debug", "phase_block"),
                            json!(true),
                        )))
                        .with_source("test:phase_block");
                        step.pending_patches.push(patch);
                    }
                    _ => {}
                }
            }
        }

        let session = Session::with_initial_state("test", json!({}));
        let tool_descriptors = vec![ToolDescriptor::new("echo", "Echo", "Echo")];
        let phases = Arc::new(Mutex::new(Vec::new()));
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PhaseBlockPlugin {
            phases: phases.clone(),
        })];
        let mut scratchpad = ScratchpadRuntimeData::new(&plugins);

        let (extracted, pending) = run_phase_block(
            &session,
            &mut scratchpad,
            &tool_descriptors,
            &plugins,
            &[Phase::StepStart, Phase::BeforeInference],
            |_| {},
            |step| {
                (
                    step.system_context.clone(),
                    step.skip_inference,
                    step.scratchpad_get::<bool>("phase_block_seen"),
                )
            },
        )
        .await
        .expect("phase block should succeed");

        assert_eq!(
            phases.lock().unwrap().as_slice(),
            &[Phase::StepStart, Phase::BeforeInference]
        );
        assert_eq!(extracted.0, vec!["from_step_start".to_string()]);
        assert!(extracted.1);
        assert_eq!(extracted.2, Some(true));
        assert_eq!(pending.len(), 1);

        let updated = apply_pending_patches(session, pending);
        let state = updated
            .rebuild_state()
            .expect("state rebuild should succeed");
        assert_eq!(state["debug"]["phase_block"], true);
    }

    #[tokio::test]
    async fn test_emit_cleanup_phases_and_apply_runs_after_inference_and_step_end() {
        struct CleanupPlugin {
            phases: Arc<Mutex<Vec<Phase>>>,
        }

        #[async_trait]
        impl AgentPlugin for CleanupPlugin {
            fn id(&self) -> &str {
                "cleanup_plugin"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                self.phases.lock().unwrap().push(phase);
                match phase {
                    Phase::AfterInference => {
                        let err =
                            step.scratchpad_get::<serde_json::Value>("llmmetry.inference_error");
                        assert_eq!(
                            err.as_ref()
                                .and_then(|v| v.get("type"))
                                .and_then(|v| v.as_str()),
                            Some("llm_stream_start_error")
                        );
                    }
                    Phase::StepEnd => {
                        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                            carve_state::path!("debug", "cleanup_ran"),
                            json!(true),
                        )))
                        .with_source("test:cleanup");
                        step.pending_patches.push(patch);
                    }
                    _ => {}
                }
            }
        }

        let session = Session::with_initial_state("test", json!({}));
        let tool_descriptors = vec![ToolDescriptor::new("echo", "Echo", "Echo")];
        let phases = Arc::new(Mutex::new(Vec::new()));
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(CleanupPlugin {
            phases: phases.clone(),
        })];
        let mut scratchpad = ScratchpadRuntimeData::new(&plugins);

        let updated = emit_cleanup_phases_and_apply(
            session,
            &mut scratchpad,
            &tool_descriptors,
            &plugins,
            "llm_stream_start_error",
            "boom".to_string(),
        )
        .await
        .expect("cleanup phases should succeed");

        assert_eq!(
            phases.lock().unwrap().as_slice(),
            &[Phase::AfterInference, Phase::StepEnd]
        );
        let state = updated
            .rebuild_state()
            .expect("state rebuild should succeed");
        assert_eq!(state["debug"]["cleanup_ran"], true);
    }

    #[tokio::test]
    async fn test_scratchpad_is_run_scoped_and_writable() {
        struct RunScopedScratchpadPlugin;

        #[async_trait]
        impl AgentPlugin for RunScopedScratchpadPlugin {
            fn id(&self) -> &str {
                "run_scoped_scratchpad"
            }

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("counter", json!(0)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::SessionStart | Phase::StepStart => {
                        let next = step.scratchpad_get::<i64>("counter").unwrap_or(0) + 1;
                        step.scratchpad_set("counter", next);
                    }
                    Phase::SessionEnd => {
                        let Ok(state) = step.session.rebuild_state() else {
                            return;
                        };
                        let run_count = state
                            .get("debug")
                            .and_then(|d| d.get("run_count"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            + 1;
                        let counter = step.scratchpad_get::<i64>("counter").unwrap_or(-1);
                        let patch = TrackedPatch::new(
                            Patch::new()
                                .with_op(Op::set(
                                    carve_state::path!("debug", "run_count"),
                                    json!(run_count),
                                ))
                                .with_op(Op::set(
                                    carve_state::path!("debug", "last_scratchpad_counter"),
                                    json!(counter),
                                )),
                        )
                        .with_source("test:run_scoped_scratchpad");
                        step.pending_patches.push(patch);
                    }
                    _ => {}
                }
            }
        }

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(RunScopedScratchpadPlugin) as Arc<dyn AgentPlugin>);
        let tools = HashMap::new();
        let session = Session::with_initial_state("test", json!({}));

        let (_, first_session) = run_mock_stream_with_final_session(
            MockStreamProvider::new(vec![MockResponse::text("done")]),
            config.clone(),
            session,
            tools.clone(),
        )
        .await;
        let first_state = first_session.rebuild_state().unwrap();
        assert_eq!(first_state["debug"]["run_count"], 1);
        assert_eq!(first_state["debug"]["last_scratchpad_counter"], 2);
        assert!(first_state.get("counter").is_none());

        let (_, second_session) = run_mock_stream_with_final_session(
            MockStreamProvider::new(vec![MockResponse::text("done")]),
            config,
            first_session,
            tools,
        )
        .await;
        let second_state = second_session.rebuild_state().unwrap();
        assert_eq!(second_state["debug"]["run_count"], 2);
        assert_eq!(
            second_state["debug"]["last_scratchpad_counter"], 2,
            "scratchpad must reset on each run instead of accumulating"
        );
        assert!(second_state.get("counter").is_none());
    }

    // ============================================================================
    // Additional Coverage Tests
    // ============================================================================

    #[test]
    fn test_agent_config_debug() {
        let config = AgentConfig::new("gpt-4").with_system_prompt("You are helpful.");

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("AgentDefinition"));
        assert!(debug_str.contains("gpt-4"));
        // Check that system_prompt is shown as length indicator
        assert!(debug_str.contains("chars]"));
    }

    #[test]
    fn test_agent_config_with_chat_options() {
        let chat_options = ChatOptions::default();
        let config = AgentConfig::new("gpt-4").with_chat_options(chat_options);
        assert!(config.chat_options.is_some());
    }

    #[test]
    fn test_agent_config_with_plugins() {
        struct DummyPlugin;

        #[async_trait]
        impl AgentPlugin for DummyPlugin {
            fn id(&self) -> &str {
                "dummy"
            }
            async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
        }

        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(DummyPlugin), Arc::new(DummyPlugin)];
        let config = AgentConfig::new("gpt-4").with_plugins(plugins);
        assert_eq!(config.plugins.len(), 2);
    }

    struct PendingPhasePlugin;

    #[async_trait]
    impl AgentPlugin for PendingPhasePlugin {
        fn id(&self) -> &str {
            "pending"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeToolExecute && step.tool_name() == Some("echo") {
                use crate::state_types::Interaction;
                step.pending(
                    Interaction::new("confirm_1", "confirm").with_message("Execute echo?"),
                );
            }
        }
    }

    #[test]
    fn test_execute_tools_with_pending_phase_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Pending".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PendingPhasePlugin)];

            let err = execute_tools_with_plugins(session, &result, &tools, true, &plugins)
                .await
                .unwrap_err();

            let (session, interaction) = match err {
                AgentLoopError::PendingInteraction {
                    session,
                    interaction,
                } => (session, interaction),
                other => panic!("Expected PendingInteraction error, got: {:?}", other),
            };

            assert_eq!(interaction.id, "confirm_1");
            assert_eq!(interaction.action, "confirm");

            // Pending tool gets a placeholder tool result to keep message sequence valid.
            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert_eq!(msg.role, crate::types::Role::Tool);
            assert!(msg.content.contains("awaiting approval"));

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["agent"]["pending_interaction"]["id"], "confirm_1");
        });
    }

    #[test]
    fn test_apply_tool_results_rejects_multiple_pending_interactions() {
        let session = Session::new("test");

        let mut first = scratchpad_result("call_1", scratchpad_map(vec![]));
        first.pending_interaction =
            Some(Interaction::new("confirm_1", "confirm").with_message("approve first tool"));

        let mut second = scratchpad_result("call_2", scratchpad_map(vec![]));
        second.pending_interaction =
            Some(Interaction::new("confirm_2", "confirm").with_message("approve second tool"));

        let result = apply_tool_results_to_session(session, &[first, second], None, false);
        assert!(
            matches!(result, Err(AgentLoopError::StateError(_))),
            "expected StateError when multiple pending interactions exist"
        );
    }

    #[test]
    fn test_apply_tool_results_appends_skill_instruction_as_user_message() {
        let session = Session::with_initial_state("test", json!({}));
        let result = skill_activation_result("call_1", "docx", Some("## DOCX\nUse docx-js."));

        let applied = apply_tool_results_to_session(session, &[result], None, false)
            .expect("apply_tool_results_to_session should succeed");

        assert_eq!(applied.session.message_count(), 2);
        assert_eq!(applied.session.messages[0].role, crate::types::Role::Tool);
        assert_eq!(applied.session.messages[1].role, crate::types::Role::User);
        assert_eq!(applied.session.messages[1].content, "## DOCX\nUse docx-js.");
    }

    #[test]
    fn test_apply_tool_results_skill_instruction_user_message_attaches_metadata() {
        let session = Session::with_initial_state("test", json!({}));
        let result = skill_activation_result("call_1", "docx", Some("Use docx-js."));
        let meta = MessageMetadata {
            run_id: Some("run-1".to_string()),
            step_index: Some(3),
        };

        let applied = apply_tool_results_to_session(session, &[result], Some(meta.clone()), false)
            .expect("apply_tool_results_to_session should succeed");

        assert_eq!(applied.session.message_count(), 2);
        let user_msg = &applied.session.messages[1];
        assert_eq!(user_msg.role, crate::types::Role::User);
        assert_eq!(user_msg.metadata.as_ref(), Some(&meta));
    }

    #[test]
    fn test_apply_tool_results_skill_without_instruction_does_not_append_user_message() {
        let session = Session::with_initial_state("test", json!({}));
        let result = skill_activation_result("call_1", "docx", None);

        let applied = apply_tool_results_to_session(session, &[result], None, false)
            .expect("apply_tool_results_to_session should succeed");

        assert_eq!(applied.session.message_count(), 1);
        assert_eq!(applied.session.messages[0].role, crate::types::Role::Tool);
    }

    #[test]
    fn test_apply_tool_results_appends_user_messages_from_tool_result_metadata() {
        let session = Session::with_initial_state("test", json!({}));
        let result = ToolExecutionResult {
            execution: crate::execute::ToolExecution {
                call: crate::types::ToolCall::new("call_1", "any_tool", json!({})),
                result: ToolResult::success("any_tool", json!({"ok": true})).with_metadata(
                    crate::skills::APPEND_USER_MESSAGES_METADATA_KEY,
                    json!(["first", "second"]),
                ),
                patch: None,
            },
            reminders: Vec::new(),
            pending_interaction: None,
            scratchpad: HashMap::new(),
            pending_patches: Vec::new(),
        };

        let applied = apply_tool_results_to_session(session, &[result], None, false)
            .expect("apply should succeed");

        assert_eq!(applied.session.message_count(), 3);
        assert_eq!(applied.session.messages[0].role, crate::types::Role::Tool);
        assert_eq!(applied.session.messages[1].role, crate::types::Role::User);
        assert_eq!(applied.session.messages[1].content, "first");
        assert_eq!(applied.session.messages[2].role, crate::types::Role::User);
        assert_eq!(applied.session.messages[2].content, "second");
    }

    #[test]
    fn test_apply_tool_results_ignores_invalid_append_user_messages_metadata() {
        let session = Session::with_initial_state("test", json!({}));
        let result = ToolExecutionResult {
            execution: crate::execute::ToolExecution {
                call: crate::types::ToolCall::new("call_1", "any_tool", json!({})),
                result: ToolResult::success("any_tool", json!({"ok": true})).with_metadata(
                    crate::skills::APPEND_USER_MESSAGES_METADATA_KEY,
                    json!({"unexpected": true}),
                ),
                patch: None,
            },
            reminders: Vec::new(),
            pending_interaction: None,
            scratchpad: HashMap::new(),
            pending_patches: Vec::new(),
        };

        let applied = apply_tool_results_to_session(session, &[result], None, false)
            .expect("apply should succeed");

        assert_eq!(applied.session.message_count(), 1);
        assert_eq!(applied.session.messages[0].role, crate::types::Role::Tool);
    }

    #[test]
    fn test_apply_tool_results_keeps_tool_and_appended_user_message_order_stable() {
        let session = Session::with_initial_state("test", json!({}));
        let first = skill_activation_result("call_2", "beta", Some("Instruction B"));
        let second = skill_activation_result("call_1", "alpha", Some("Instruction A"));

        let applied =
            apply_tool_results_to_session(session, &[first, second], None, true).expect("apply");
        let messages = &applied.session.messages;

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, crate::types::Role::Tool);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_2"));
        assert_eq!(messages[1].role, crate::types::Role::User);
        assert_eq!(messages[1].content, "Instruction B");
        assert_eq!(messages[2].role, crate::types::Role::Tool);
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(messages[3].role, crate::types::Role::User);
        assert_eq!(messages[3].content, "Instruction A");
    }

    #[test]
    fn test_agent_loop_error_state_error() {
        let err = AgentLoopError::StateError("invalid state".to_string());
        assert!(err.to_string().contains("State error"));
        assert!(err.to_string().contains("invalid state"));
    }

    #[test]
    fn test_execute_tools_missing_tool() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Calling unknown tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "unknown_tool",
                    json!({}),
                )],
                usage: None,
            };
            let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new(); // Empty tools

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("not found") || msg.content.contains("Error"),
                "Expected 'not found' error in message, got: {}",
                msg.content
            );
        });
    }

    #[test]
    fn test_execute_tools_with_config_empty_calls() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "No tools".to_string(),
                tool_calls: vec![],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4");

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            // No messages should be added when there are no tool calls
            assert_eq!(session.message_count(), 0);
        });
    }

    #[test]
    fn test_execute_tools_with_config_basic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Calling tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4");

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.messages[0].role, crate::types::Role::Tool);
        });
    }

    #[test]
    fn test_execute_tools_with_config_attaches_runtime_run_metadata() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut session = Session::new("test").with_message(
                Message::assistant_with_tool_calls(
                    "calling tool",
                    vec![crate::types::ToolCall::new(
                        "call_1",
                        "echo",
                        json!({"message": "test"}),
                    )],
                )
                .with_metadata(crate::types::MessageMetadata {
                    run_id: Some("run-meta-1".to_string()),
                    step_index: Some(7),
                }),
            );
            session.runtime.set("run_id", "run-meta-1").unwrap();

            let result = StreamResult {
                text: "Calling tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4");

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 2);
            let tool_msg = session.messages.last().expect("tool message should exist");
            assert_eq!(tool_msg.role, crate::types::Role::Tool);
            let meta = tool_msg
                .metadata
                .as_ref()
                .expect("tool message metadata should be attached");
            assert_eq!(meta.run_id.as_deref(), Some("run-meta-1"));
            assert_eq!(meta.step_index, Some(7));
        });
    }

    #[test]
    fn test_execute_tools_with_config_with_blocking_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Blocked".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4")
                .with_plugin(Arc::new(BlockingPhasePlugin) as Arc<dyn AgentPlugin>);

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("blocked") || msg.content.contains("Error"),
                "Expected blocked error in message, got: {}",
                msg.content
            );
        });
    }

    #[test]
    fn test_execute_tools_with_config_with_pending_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Pending".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4")
                .with_plugin(Arc::new(PendingPhasePlugin) as Arc<dyn AgentPlugin>);

            let err = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap_err();

            let (session, interaction) = match err {
                AgentLoopError::PendingInteraction {
                    session,
                    interaction,
                } => (session, interaction),
                other => panic!("Expected PendingInteraction error, got: {:?}", other),
            };

            assert_eq!(interaction.id, "confirm_1");
            assert_eq!(interaction.action, "confirm");

            // Pending tool gets a placeholder tool result to keep message sequence valid.
            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert_eq!(msg.role, crate::types::Role::Tool);
            assert!(msg.content.contains("awaiting approval"));

            // Pending interaction should be persisted via AgentState.
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["agent"]["pending_interaction"]["id"], "confirm_1");
        });
    }

    #[test]
    fn test_execute_tools_with_config_with_reminder_plugin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "With reminder".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4")
                .with_plugin(Arc::new(ReminderPhasePlugin) as Arc<dyn AgentPlugin>);

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            // Should have tool response + reminder message
            assert_eq!(session.message_count(), 2);
            assert!(session.messages[1].content.contains("system-reminder"));
        });
    }

    #[test]
    fn test_execute_tools_with_config_clears_persisted_pending_interaction_on_success() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Seed a session with a previously persisted pending interaction.
            let base_state = json!({});
            let pending_patch = set_agent_pending_interaction(
                &base_state,
                Interaction::new("confirm_1", "confirm").with_message("ok"),
            );
            let session = Session::with_initial_state("test", base_state).with_patch(pending_patch);

            let result = StreamResult {
                text: "Calling tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let config = AgentConfig::new("gpt-4");

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            let state = session.rebuild_state().unwrap();
            let pending = state
                .get("agent")
                .and_then(|a| a.get("pending_interaction"));
            assert!(
                pending.is_none() || pending.is_some_and(|v| v.is_null()),
                "expected pending_interaction to be cleared, got: {pending:?}"
            );
        });
    }

    #[test]
    fn test_execute_tools_sequential_propagates_intermediate_state_apply_errors() {
        struct FirstCallIntermediatePatchPlugin;

        #[async_trait]
        impl AgentPlugin for FirstCallIntermediatePatchPlugin {
            fn id(&self) -> &str {
                "first_call_intermediate_patch"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase != Phase::AfterToolExecute || step.tool_call_id() != Some("call_1") {
                    return;
                }

                // This increment fails when applied between call_1 and call_2 because
                // `counter` doesn't exist yet. Swallowing that failure hides a broken
                // intermediate state transition.
                let patch = TrackedPatch::new(
                    Patch::new().with_op(Op::increment(carve_state::path!("counter"), 1_i64)),
                )
                .with_source("test:intermediate_apply_error");
                step.pending_patches.push(patch);
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Call tools".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "hello"})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 5})),
                ],
                usage: None,
            };

            let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool));
            tools.insert("counter".to_string(), Arc::new(CounterTool));
            let plugins: Vec<Arc<dyn AgentPlugin>> =
                vec![Arc::new(FirstCallIntermediatePatchPlugin)];

            let err = execute_tools_with_plugins(session, &result, &tools, false, &plugins)
                .await
                .expect_err("sequential apply errors should surface");
            assert!(matches!(err, AgentLoopError::StateError(_)));
        });
    }

    // ========================================================================
    // Phase lifecycle helpers & tests for run_loop_stream
    // ========================================================================

    /// Plugin that records phases AND skips inference.
    struct RecordAndSkipPlugin {
        phases: Arc<Mutex<Vec<Phase>>>,
    }

    impl RecordAndSkipPlugin {
        fn new() -> (Self, Arc<Mutex<Vec<Phase>>>) {
            let phases = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    phases: phases.clone(),
                },
                phases,
            )
        }
    }

    #[async_trait]
    impl AgentPlugin for RecordAndSkipPlugin {
        fn id(&self) -> &str {
            "record_and_skip"
        }
        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            self.phases.lock().unwrap().push(phase);
            if phase == Phase::BeforeInference {
                step.skip_inference = true;
            }
        }
    }

    /// Collect all events from a stream.
    async fn collect_stream_events(
        stream: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    ) -> Vec<AgentEvent> {
        use futures::StreamExt;
        let mut events = Vec::new();
        let mut stream = stream;
        while let Some(event) = stream.next().await {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn test_stream_skip_inference_emits_session_end_phase() {
        let (recorder, phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();

        let stream = run_loop_stream(
            Client::default(),
            config,
            session,
            tools,
            RunContext::default(),
        );
        let events = collect_stream_events(stream).await;

        // Verify events include RunStart and RunFinish
        assert!(
            matches!(events.first(), Some(AgentEvent::RunStart { .. })),
            "Expected RunStart as first event, got: {:?}",
            events.first()
        );
        assert!(
            matches!(events.last(), Some(AgentEvent::RunFinish { .. })),
            "Expected RunFinish as last event, got: {:?}",
            events.last()
        );

        // Verify phase lifecycle: SessionStart → StepStart → BeforeInference → SessionEnd
        let recorded = phases.lock().unwrap().clone();
        assert!(
            recorded.contains(&Phase::SessionStart),
            "Missing SessionStart phase"
        );
        assert!(
            recorded.contains(&Phase::SessionEnd),
            "Missing SessionEnd phase"
        );

        // SessionEnd must be last
        assert_eq!(
            recorded.last(),
            Some(&Phase::SessionEnd),
            "SessionEnd should be last phase, got: {:?}",
            recorded
        );
    }

    #[tokio::test]
    async fn test_stream_skip_inference_emits_run_start_and_finish() {
        // Verify the complete event sequence on skip_inference path
        let (recorder, _phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();

        let stream = run_loop_stream(
            Client::default(),
            config,
            session,
            tools,
            RunContext::default(),
        );
        let events = collect_stream_events(stream).await;

        let event_names: Vec<&str> = events
            .iter()
            .map(|e| match e {
                AgentEvent::RunStart { .. } => "RunStart",
                AgentEvent::RunFinish { .. } => "RunFinish",
                AgentEvent::Error { .. } => "Error",
                _ => "Other",
            })
            .collect();
        assert_eq!(event_names, vec!["RunStart", "RunFinish"]);
    }

    #[tokio::test]
    async fn test_run_loop_skip_inference_emits_session_end_phase() {
        let (recorder, phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();
        let client = Client::default();

        let result = run_loop(&client, &config, session, &tools).await;
        assert!(result.is_ok());

        let recorded = phases.lock().unwrap().clone();
        assert!(
            recorded.contains(&Phase::SessionStart),
            "Missing SessionStart phase"
        );
        assert!(
            recorded.contains(&Phase::SessionEnd),
            "Missing SessionEnd phase"
        );
        assert_eq!(
            recorded.last(),
            Some(&Phase::SessionEnd),
            "SessionEnd should be last phase, got: {:?}",
            recorded
        );
    }

    #[tokio::test]
    async fn test_run_loop_auto_generated_run_id_is_rfc4122_uuid_v7() {
        let (recorder, _phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();
        let client = Client::default();

        let (final_session, _response) = run_loop(&client, &config, session, &tools)
            .await
            .expect("run_loop should succeed");
        let run_id = final_session
            .runtime
            .value("run_id")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("run_loop must populate runtime run_id"));

        let parsed = uuid::Uuid::parse_str(run_id)
            .unwrap_or_else(|_| panic!("run_id must be parseable UUID, got: {run_id}"));
        assert_eq!(
            parsed.get_variant(),
            uuid::Variant::RFC4122,
            "run_id must be RFC4122 UUID, got: {run_id}"
        );
        assert_eq!(
            parsed.get_version_num(),
            7,
            "run_id must be version 7 UUID, got: {run_id}"
        );
    }

    #[tokio::test]
    async fn test_run_loop_phase_sequence_on_skip_inference() {
        // Verify the full phase sequence: SessionStart → StepStart → BeforeInference → SessionEnd
        let (recorder, phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();
        let client = Client::default();

        let result = run_loop(&client, &config, session, &tools).await;
        assert!(result.is_ok());

        let recorded = phases.lock().unwrap().clone();
        assert_eq!(
            recorded,
            vec![
                Phase::SessionStart,
                Phase::StepStart,
                Phase::BeforeInference,
                Phase::SessionEnd,
            ],
            "Unexpected phase sequence: {:?}",
            recorded
        );
    }

    #[tokio::test]
    async fn test_run_loop_rejects_skip_inference_mutation_outside_before_inference() {
        struct InvalidStepStartSkipPlugin;

        #[async_trait]
        impl AgentPlugin for InvalidStepStartSkipPlugin {
            fn id(&self) -> &str {
                "invalid_step_start_skip"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::StepStart {
                    step.skip_inference = true;
                }
            }
        }

        let config = AgentConfig::new("gpt-4o-mini")
            .with_plugin(Arc::new(InvalidStepStartSkipPlugin) as Arc<dyn AgentPlugin>);
        let session = Session::new("test").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();
        let client = Client::default();

        let result = run_loop(&client, &config, session, &tools).await;
        assert!(
            matches!(
                result,
                Err(AgentLoopError::StateError(ref message))
                if message.contains("mutated skip_inference outside BeforeInference")
            ),
            "expected phase mutation state error, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_stream_rejects_skip_inference_mutation_outside_before_inference() {
        struct InvalidStepStartSkipPlugin;

        #[async_trait]
        impl AgentPlugin for InvalidStepStartSkipPlugin {
            fn id(&self) -> &str {
                "invalid_step_start_skip"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::StepStart {
                    step.skip_inference = true;
                }
            }
        }

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(InvalidStepStartSkipPlugin) as Arc<dyn AgentPlugin>);
        let session = Session::new("test").with_message(Message::user("hi"));
        let tools = HashMap::new();

        let events = run_mock_stream(MockStreamProvider::new(vec![]), config, session, tools).await;

        assert!(
            events.iter().any(|event| matches!(
                event,
                AgentEvent::Error { message }
                if message.contains("mutated skip_inference outside BeforeInference")
            )),
            "expected mutation error event, got: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(AgentEvent::RunFinish { .. })),
            "expected stream termination after mutation error, got: {events:?}"
        );
    }

    #[tokio::test]
    async fn test_stream_run_finish_has_matching_thread_id() {
        let (recorder, _phases) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("gpt-4o-mini").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);

        let session = Session::new("my-session").with_message(crate::types::Message::user("hello"));
        let tools = HashMap::new();

        let stream = run_loop_stream(
            Client::default(),
            config,
            session,
            tools,
            RunContext::default(),
        );
        let events = collect_stream_events(stream).await;

        // Extract thread_id from RunStart and RunFinish
        let start_tid = events.iter().find_map(|e| match e {
            AgentEvent::RunStart { thread_id, .. } => Some(thread_id.clone()),
            _ => None,
        });
        let finish_tid = events.iter().find_map(|e| match e {
            AgentEvent::RunFinish { thread_id, .. } => Some(thread_id.clone()),
            _ => None,
        });

        assert_eq!(
            start_tid, finish_tid,
            "RunStart and RunFinish thread_ids must match"
        );
        assert_eq!(start_tid.as_deref(), Some("my-session"));
    }

    // ========================================================================
    // RunContext tests
    // ========================================================================

    #[test]
    fn test_run_context_default() {
        let ctx = RunContext::default();
        assert!(ctx.cancellation_token.is_none());
    }

    #[test]
    fn test_run_context_with_cancellation() {
        let ctx = RunContext {
            cancellation_token: Some(CancellationToken::new()),
        };
        assert!(ctx.cancellation_token.is_some());
    }

    #[test]
    fn test_run_context_clone() {
        let ctx = RunContext {
            cancellation_token: None,
        };
        let cloned = ctx.clone();
        assert!(cloned.cancellation_token.is_none());
    }

    #[test]
    fn test_runtime_run_id_in_session() {
        let mut session = Session::new("test");
        session.runtime.set("run_id", "my-run").unwrap();
        session.runtime.set("parent_run_id", "parent-run").unwrap();
        assert_eq!(
            session.runtime.value("run_id").and_then(|v| v.as_str()),
            Some("my-run")
        );
        assert_eq!(
            session
                .runtime
                .value("parent_run_id")
                .and_then(|v| v.as_str()),
            Some("parent-run")
        );
    }

    // ========================================================================
    // Mock ChatStreamProvider for stop condition integration tests
    // ========================================================================

    /// A single mock LLM response: text and optional tool calls.
    #[derive(Clone)]
    struct MockResponse {
        text: String,
        tool_calls: Vec<genai::chat::ToolCall>,
        usage: Option<Usage>,
    }

    impl MockResponse {
        fn text(s: &str) -> Self {
            Self {
                text: s.to_string(),
                tool_calls: Vec::new(),
                usage: None,
            }
        }

        fn with_tool_call(mut self, call_id: &str, name: &str, args: Value) -> Self {
            self.tool_calls.push(genai::chat::ToolCall {
                call_id: call_id.to_string(),
                fn_name: name.to_string(),
                fn_arguments: Value::String(args.to_string()),
                thought_signatures: None,
            });
            self
        }

        fn with_usage(mut self, input: i32, output: i32) -> Self {
            self.usage = Some(Usage {
                prompt_tokens: Some(input),
                prompt_tokens_details: None,
                completion_tokens: Some(output),
                completion_tokens_details: None,
                total_tokens: Some(input + output),
            });
            self
        }
    }

    /// Mock provider that returns pre-configured responses in order.
    /// After all responses are consumed, returns text-only (triggering NaturalEnd).
    struct MockStreamProvider {
        responses: Mutex<Vec<MockResponse>>,
    }

    impl MockStreamProvider {
        fn new(responses: Vec<MockResponse>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait]
    impl ChatStreamProvider for MockStreamProvider {
        async fn exec_chat_stream_events(
            &self,
            _model: &str,
            _chat_req: genai::chat::ChatRequest,
            _options: Option<&ChatOptions>,
        ) -> genai::Result<Pin<Box<dyn Stream<Item = genai::Result<ChatStreamEvent>> + Send>>>
        {
            let resp = {
                let mut responses = self.responses.lock().unwrap();
                if responses.is_empty() {
                    MockResponse::text("done")
                } else {
                    responses.remove(0)
                }
            };

            let mut events: Vec<genai::Result<ChatStreamEvent>> = Vec::new();
            events.push(Ok(ChatStreamEvent::Start));

            if !resp.text.is_empty() {
                events.push(Ok(ChatStreamEvent::Chunk(StreamChunk {
                    content: resp.text.clone(),
                })));
            }

            for tc in &resp.tool_calls {
                events.push(Ok(ChatStreamEvent::ToolCallChunk(ToolChunk {
                    tool_call: tc.clone(),
                })));
            }

            let end = StreamEnd {
                captured_content: if resp.tool_calls.is_empty() {
                    None
                } else {
                    Some(MessageContent::from_tool_calls(resp.tool_calls))
                },
                captured_usage: resp.usage,
                ..Default::default()
            };
            events.push(Ok(ChatStreamEvent::End(end)));

            Ok(Box::pin(futures::stream::iter(events)))
        }
    }

    /// Helper: run a mock stream and collect events.
    async fn run_mock_stream(
        provider: MockStreamProvider,
        config: AgentConfig,
        session: Session,
        tools: HashMap<String, Arc<dyn Tool>>,
    ) -> Vec<AgentEvent> {
        let stream = run_loop_stream_impl_with_provider(
            Arc::new(provider),
            config,
            session,
            tools,
            RunContext::default(),
            None,
            None,
        );
        collect_stream_events(stream).await
    }

    /// Helper: run a mock stream and collect events plus final session.
    async fn run_mock_stream_with_final_session(
        provider: MockStreamProvider,
        config: AgentConfig,
        session: Session,
        tools: HashMap<String, Arc<dyn Tool>>,
    ) -> (Vec<AgentEvent>, Session) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let stream = run_loop_stream_impl_with_provider(
            Arc::new(provider),
            config,
            session,
            tools,
            RunContext::default(),
            Some(tx),
            None,
        );
        let events = collect_stream_events(stream).await;
        let final_session = rx.await.expect("final session should be available");
        (events, final_session)
    }

    /// Extract the stop_reason from the RunFinish event.
    fn extract_stop_reason(events: &[AgentEvent]) -> Option<StopReason> {
        events.iter().find_map(|e| match e {
            AgentEvent::RunFinish { stop_reason, .. } => stop_reason.clone(),
            _ => None,
        })
    }

    #[tokio::test]
    async fn test_stream_replay_invalid_payload_emits_error_and_finish() {
        struct InvalidReplayPayloadPlugin;

        #[async_trait]
        impl AgentPlugin for InvalidReplayPayloadPlugin {
            fn id(&self) -> &str {
                "invalid_replay_payload"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::SessionStart {
                    step.scratchpad_set("__replay_tool_calls", json!({"bad": "payload"}));
                }
            }
        }

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(InvalidReplayPayloadPlugin) as Arc<dyn AgentPlugin>);
        let session = Session::new("test").with_message(Message::user("resume"));
        let tools = tool_map([EchoTool]);

        let events = run_mock_stream(
            MockStreamProvider::new(vec![MockResponse::text("should not run")]),
            config,
            session,
            tools,
        )
        .await;

        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Error { message }
                if message.contains("__replay_tool_calls")
            )),
            "expected replay payload parse error, got events: {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(AgentEvent::RunFinish {
                    stop_reason: None,
                    ..
                })
            ),
            "expected terminal RunFinish after replay parse error, got: {:?}",
            events.last()
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::TextDelta { .. })),
            "stream should terminate before inference when replay payload is invalid"
        );
    }

    #[tokio::test]
    async fn test_stream_replay_rebuild_state_failure_emits_error() {
        struct ReplayPlugin;

        #[async_trait]
        impl AgentPlugin for ReplayPlugin {
            fn id(&self) -> &str {
                "replay_state_failure"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::SessionStart {
                    step.scratchpad_set(
                        "__replay_tool_calls",
                        vec![crate::types::ToolCall::new(
                            "replay_call_1",
                            "echo",
                            json!({"message": "resume"}),
                        )],
                    );
                }
            }
        }

        let broken_patch = carve_state::TrackedPatch::new(
            Patch::new().with_op(Op::increment(carve_state::path!("missing_counter"), 1_i64)),
        )
        .with_source("test:broken_state");
        let session = Session::with_initial_state("test", json!({}))
            .with_message(Message::user("resume"))
            .with_patch(broken_patch);

        let config =
            AgentConfig::new("mock").with_plugin(Arc::new(ReplayPlugin) as Arc<dyn AgentPlugin>);
        let tools = tool_map([EchoTool]);

        let events = run_mock_stream(
            MockStreamProvider::new(vec![MockResponse::text("should not run")]),
            config,
            session,
            tools,
        )
        .await;

        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::Error { message } if message.contains("replay"))),
            "expected replay state rebuild error, got events: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::ToolCallDone { .. })),
            "replay tool must not execute when state rebuild fails"
        );
    }

    #[tokio::test]
    async fn test_stream_replay_tool_exec_respects_tool_phases() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static BEFORE_TOOL_EXECUTED: AtomicBool = AtomicBool::new(false);

        struct ReplayBlockingPlugin;

        #[async_trait]
        impl AgentPlugin for ReplayBlockingPlugin {
            fn id(&self) -> &str {
                "replay_blocking"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::SessionStart => {
                        step.scratchpad_set(
                            "__replay_tool_calls",
                            vec![crate::types::ToolCall::new(
                                "replay_call_1",
                                "echo",
                                json!({"message": "resume"}),
                            )],
                        );
                    }
                    Phase::BeforeToolExecute if step.tool_call_id() == Some("replay_call_1") => {
                        BEFORE_TOOL_EXECUTED.store(true, Ordering::SeqCst);
                        step.block("blocked in replay");
                    }
                    _ => {}
                }
            }
        }

        BEFORE_TOOL_EXECUTED.store(false, Ordering::SeqCst);

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(ReplayBlockingPlugin) as Arc<dyn AgentPlugin>);
        let session = Session::new("test").with_message(Message::user("resume"));
        let tools = tool_map([EchoTool]);

        let events = run_mock_stream(
            MockStreamProvider::new(vec![MockResponse::text("done")]),
            config,
            session,
            tools,
        )
        .await;

        let replay_done = events.iter().find(|e| {
            matches!(
                e,
                AgentEvent::ToolCallDone { id, .. } if id == "replay_call_1"
            )
        });

        let replay_result = match replay_done {
            Some(AgentEvent::ToolCallDone { result, .. }) => result,
            _ => panic!("expected replay ToolCallDone event, got: {events:?}"),
        };

        assert!(
            BEFORE_TOOL_EXECUTED.load(Ordering::SeqCst),
            "BeforeToolExecute should run for replayed tool calls"
        );
        assert!(
            replay_result.is_error(),
            "blocked replay should produce an error tool result"
        );
    }

    #[tokio::test]
    async fn test_stream_apply_error_still_runs_session_end_phase() {
        use std::sync::atomic::{AtomicBool, Ordering};

        static SESSION_END_RAN: AtomicBool = AtomicBool::new(false);

        struct PendingAndSessionEndPlugin;

        #[async_trait]
        impl AgentPlugin for PendingAndSessionEndPlugin {
            fn id(&self) -> &str {
                "pending_and_session_end"
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::BeforeToolExecute => {
                        if let Some(call_id) = step.tool_call_id() {
                            step.pending(
                                Interaction::new(format!("confirm_{call_id}"), "confirm")
                                    .with_message("needs confirmation"),
                            );
                        }
                    }
                    Phase::SessionEnd => {
                        SESSION_END_RAN.store(true, Ordering::SeqCst);
                    }
                    _ => {}
                }
            }
        }

        SESSION_END_RAN.store(false, Ordering::SeqCst);

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(PendingAndSessionEndPlugin) as Arc<dyn AgentPlugin>)
            .with_parallel_tools(true);
        let session = Session::new("test").with_message(Message::user("run tools"));
        let responses = vec![MockResponse::text("run both")
            .with_tool_call("call_1", "echo", json!({"message": "a"}))
            .with_tool_call("call_2", "echo", json!({"message": "b"}))];
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;

        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Error { message }
                if message.contains("multiple pending interactions")
            )),
            "expected apply error when multiple pending interactions exist: {events:?}"
        );
        assert!(
            SESSION_END_RAN.load(Ordering::SeqCst),
            "SessionEnd phase must run on apply_tool_results failure"
        );
    }

    // ========================================================================
    // Stop condition integration tests
    // ========================================================================

    #[tokio::test]
    async fn test_stop_max_rounds_via_stop_condition() {
        // Configure MaxRounds(2) as explicit stop condition.
        // Provider returns tool calls forever → should stop after 2 rounds.
        let responses: Vec<MockResponse> = (0..10)
            .map(|i| {
                MockResponse::text("calling echo").with_tool_call(
                    &format!("c{i}"),
                    "echo",
                    json!({"message": "hi"}),
                )
            })
            .collect();

        let config = AgentConfig::new("mock").with_stop_condition(crate::stop::MaxRounds(2));
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[tokio::test]
    async fn test_stop_natural_end_no_tools() {
        // LLM returns text only → NaturalEnd.
        let provider = MockStreamProvider::new(vec![MockResponse::text("Hello!")]);
        let config = AgentConfig::new("mock");
        let session = Session::new("test").with_message(Message::user("hi"));
        let tools = HashMap::new();

        let events = run_mock_stream(provider, config, session, tools).await;
        assert_eq!(extract_stop_reason(&events), Some(StopReason::NaturalEnd));
    }

    #[tokio::test]
    async fn test_parallel_tool_scratchpad_merges_across_calls() {
        struct ParallelScratchpadRecorder;

        #[async_trait]
        impl AgentPlugin for ParallelScratchpadRecorder {
            fn id(&self) -> &str {
                "parallel_scratchpad_recorder"
            }

            fn initial_scratchpad(&self) -> Option<(&'static str, Value)> {
                Some(("seed", json!(true)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::BeforeToolExecute => {
                        if let Some(call_id) = step.tool_call_id() {
                            step.scratchpad_set(&format!("seen_{call_id}"), true);
                        }
                    }
                    Phase::BeforeInference => {
                        let seen_count = step
                            .scratchpad_snapshot()
                            .keys()
                            .filter(|k| k.starts_with("seen_"))
                            .count();

                        let patch = TrackedPatch::new(Patch::new().with_op(Op::set(
                            carve_state::path!("debug", "seen_parallel_count"),
                            json!(seen_count),
                        )))
                        .with_source("test:parallel_scratchpad");
                        step.pending_patches.push(patch);
                    }
                    _ => {}
                }
            }
        }

        let responses = vec![
            MockResponse::text("run tools")
                .with_tool_call("call_a", "echo", json!({"message": "a"}))
                .with_tool_call("call_b", "counter", json!({"amount": 1})),
            MockResponse::text("done"),
        ];

        let config = AgentConfig::new("mock")
            .with_plugin(Arc::new(ParallelScratchpadRecorder) as Arc<dyn AgentPlugin>)
            .with_parallel_tools(true);
        let session = Session::new("test").with_message(Message::user("go"));

        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert("echo".to_string(), Arc::new(EchoTool));
        tools.insert("counter".to_string(), Arc::new(CounterTool));

        let (_events, final_session) = run_mock_stream_with_final_session(
            MockStreamProvider::new(responses),
            config,
            session,
            tools,
        )
        .await;

        let state = final_session.rebuild_state().unwrap();
        assert_eq!(state["debug"]["seen_parallel_count"], 2);
    }

    #[test]
    fn test_merge_parallel_scratchpad_strict_rejects_conflicts() {
        let base = scratchpad_map(vec![("shared", json!(0)), ("stable", json!(true))]);
        let results = vec![
            scratchpad_result(
                "call_a",
                scratchpad_map(vec![("shared", json!(1)), ("stable", json!(true))]),
            ),
            scratchpad_result(
                "call_b",
                scratchpad_map(vec![("shared", json!(2)), ("stable", json!(true))]),
            ),
        ];

        let err = merge_parallel_scratchpad(&base, &results, ScratchpadMergePolicy::Strict)
            .expect_err("strict policy should fail on conflicting key updates");

        match err {
            AgentLoopError::StateError(message) => {
                assert!(message.contains("shared"), "unexpected message: {message}");
            }
            other => panic!("expected state error, got: {other:?}"),
        }
    }

    #[test]
    fn test_merge_parallel_scratchpad_deterministic_lww_uses_tool_call_order() {
        let base = scratchpad_map(vec![("shared", json!(0)), ("stable", json!(true))]);
        let results = vec![
            scratchpad_result(
                "call_a",
                scratchpad_map(vec![("shared", json!(1)), ("stable", json!(true))]),
            ),
            scratchpad_result(
                "call_b",
                scratchpad_map(vec![("shared", json!(2)), ("stable", json!(true))]),
            ),
        ];

        let merged =
            merge_parallel_scratchpad(&base, &results, ScratchpadMergePolicy::DeterministicLww)
                .expect("deterministic lww should resolve conflicts");
        assert_eq!(merged.get("shared"), Some(&json!(2)));
    }

    #[test]
    fn test_merge_parallel_scratchpad_deterministic_lww_disjoint_commutative() {
        let permutations = [
            [0usize, 1, 2],
            [0usize, 2, 1],
            [1usize, 0, 2],
            [1usize, 2, 0],
            [2usize, 0, 1],
            [2usize, 1, 0],
        ];

        for alpha in [1_i64, 7_i64] {
            for beta in ["x", "y"] {
                for gamma in [true, false] {
                    let base = scratchpad_map(vec![("shared.root", json!("root"))]);
                    let build_result = |idx: usize| match idx {
                        0 => scratchpad_result(
                            "call_a",
                            scratchpad_map(vec![
                                ("shared.root", json!("root")),
                                ("ns.alpha", json!(alpha)),
                            ]),
                        ),
                        1 => scratchpad_result(
                            "call_b",
                            scratchpad_map(vec![
                                ("shared.root", json!("root")),
                                ("ns.beta", json!(beta)),
                            ]),
                        ),
                        2 => scratchpad_result(
                            "call_c",
                            scratchpad_map(vec![
                                ("shared.root", json!("root")),
                                ("ns.gamma", json!(gamma)),
                            ]),
                        ),
                        _ => panic!("invalid index"),
                    };

                    let first_order: Vec<ToolExecutionResult> = permutations[0]
                        .iter()
                        .map(|idx| build_result(*idx))
                        .collect();
                    let expected = merge_parallel_scratchpad(
                        &base,
                        &first_order,
                        ScratchpadMergePolicy::DeterministicLww,
                    )
                    .expect("base merge should succeed");

                    for order in permutations.iter().skip(1) {
                        let ordered: Vec<ToolExecutionResult> =
                            order.iter().map(|idx| build_result(*idx)).collect();
                        let merged = merge_parallel_scratchpad(
                            &base,
                            &ordered,
                            ScratchpadMergePolicy::DeterministicLww,
                        )
                        .expect("merge should succeed for disjoint keys");
                        assert_eq!(merged, expected);
                    }
                }
            }
        }
    }

    #[test]
    fn test_merge_parallel_scratchpad_deterministic_lww_deterministic_and_idempotent() {
        let base = scratchpad_map(vec![("seed", json!(true)), ("shared", json!(0))]);

        let build_results = || {
            vec![
                scratchpad_result(
                    "call_1",
                    scratchpad_map(vec![
                        ("seed", json!(true)),
                        ("shared", json!(1)),
                        ("ns.alpha", json!("a")),
                    ]),
                ),
                scratchpad_result(
                    "call_2",
                    scratchpad_map(vec![
                        ("seed", json!(true)),
                        ("shared", json!(2)),
                        ("ns.beta", json!("b")),
                    ]),
                ),
                scratchpad_result(
                    "call_3",
                    scratchpad_map(vec![
                        ("seed", json!(true)),
                        ("shared", json!(3)),
                        ("ns.gamma", json!("c")),
                    ]),
                ),
            ]
        };

        let expected = merge_parallel_scratchpad(
            &base,
            &build_results(),
            ScratchpadMergePolicy::DeterministicLww,
        )
        .expect("merge should succeed");

        for _ in 0..8 {
            let merged = merge_parallel_scratchpad(
                &base,
                &build_results(),
                ScratchpadMergePolicy::DeterministicLww,
            )
            .expect("repeated merge should succeed");
            assert_eq!(merged, expected);
        }

        let mut repeated = build_results();
        repeated.extend(build_results());
        let idempotent =
            merge_parallel_scratchpad(&base, &repeated, ScratchpadMergePolicy::DeterministicLww)
                .expect("merge should stay stable when the same result set is replayed");
        assert_eq!(idempotent, expected);
    }

    #[tokio::test]
    async fn test_stop_plugin_requested() {
        // SkipInferencePlugin → PluginRequested.
        let (recorder, _) = RecordAndSkipPlugin::new();
        let config =
            AgentConfig::new("mock").with_plugin(Arc::new(recorder) as Arc<dyn AgentPlugin>);
        let session = Session::new("test").with_message(Message::user("hi"));
        let tools = HashMap::new();

        let provider = MockStreamProvider::new(vec![]);
        let events = run_mock_stream(provider, config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::PluginRequested)
        );
    }

    #[tokio::test]
    async fn test_stop_on_tool_condition() {
        // StopOnTool("finish") → first round calls echo, second calls finish.
        let responses = vec![
            MockResponse::text("step 1").with_tool_call("c1", "echo", json!({"message": "a"})),
            MockResponse::text("step 2").with_tool_call("c2", "finish_tool", json!({})),
        ];

        struct FinishTool;
        #[async_trait]
        impl Tool for FinishTool {
            fn descriptor(&self) -> ToolDescriptor {
                ToolDescriptor::new("finish_tool", "Finish", "Finishes the run")
            }
            async fn execute(
                &self,
                _args: Value,
                _ctx: &Context<'_>,
            ) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success("finish_tool", json!({"done": true})))
            }
        }

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::StopOnTool("finish_tool".to_string()));
        let session = Session::new("test").with_message(Message::user("go"));

        let mut tools = tool_map([EchoTool]);
        let ft: Arc<dyn Tool> = Arc::new(FinishTool);
        tools.insert("finish_tool".to_string(), ft);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::ToolCalled("finish_tool".to_string()))
        );
    }

    #[tokio::test]
    async fn test_stop_content_match_condition() {
        // ContentMatch("FINAL_ANSWER") → second response has it in the text.
        let responses = vec![
            MockResponse::text("thinking...").with_tool_call("c1", "echo", json!({"message": "a"})),
            MockResponse::text("here is the FINAL_ANSWER: 42").with_tool_call(
                "c2",
                "echo",
                json!({"message": "b"}),
            ),
        ];

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::ContentMatch("FINAL_ANSWER".to_string()))
            .with_stop_condition(crate::stop::MaxRounds(10));
        let session = Session::new("test").with_message(Message::user("solve"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::ContentMatched("FINAL_ANSWER".to_string()))
        );
    }

    #[tokio::test]
    async fn test_stop_token_budget_condition() {
        // TokenBudget with max_total=500 → second round pushes over budget.
        let responses = vec![
            MockResponse::text("step 1")
                .with_tool_call("c1", "echo", json!({"message": "a"}))
                .with_usage(200, 100),
            MockResponse::text("step 2")
                .with_tool_call("c2", "echo", json!({"message": "b"}))
                .with_usage(200, 100),
        ];

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::TokenBudget { max_total: 500 })
            .with_stop_condition(crate::stop::MaxRounds(10));
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::TokenBudgetExceeded)
        );
    }

    #[tokio::test]
    async fn test_stop_consecutive_errors_condition() {
        // ConsecutiveErrors(2) → all tool calls fail each round.
        let responses: Vec<MockResponse> = (0..5)
            .map(|i| {
                MockResponse::text(&format!("round {i}")).with_tool_call(
                    &format!("c{i}"),
                    "failing",
                    json!({}),
                )
            })
            .collect();

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::ConsecutiveErrors(2))
            .with_stop_condition(crate::stop::MaxRounds(10));
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([FailingTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::ConsecutiveErrorsExceeded)
        );
    }

    #[tokio::test]
    async fn test_stop_loop_detection_condition() {
        // LoopDetection(window=3) → same tool called repeatedly.
        let responses: Vec<MockResponse> = (0..5)
            .map(|i| {
                MockResponse::text(&format!("round {i}")).with_tool_call(
                    &format!("c{i}"),
                    "echo",
                    json!({"message": "same"}),
                )
            })
            .collect();

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::LoopDetection { window: 3 })
            .with_stop_condition(crate::stop::MaxRounds(10));
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(extract_stop_reason(&events), Some(StopReason::LoopDetected));
    }

    #[tokio::test]
    async fn test_stop_cancellation_token() {
        // Cancel before first inference.
        let token = CancellationToken::new();
        token.cancel();

        let provider = MockStreamProvider::new(vec![MockResponse::text("never")]);
        let config = AgentConfig::new("mock");
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = HashMap::new();

        let stream = run_loop_stream_impl_with_provider(
            Arc::new(provider),
            config,
            session,
            tools,
            RunContext {
                cancellation_token: Some(token),
            },
            None,
            None,
        );
        let events = collect_stream_events(stream).await;
        assert_eq!(extract_stop_reason(&events), Some(StopReason::Cancelled));
    }

    #[tokio::test]
    async fn test_stop_cancellation_token_during_inference_stream() {
        struct HangingStreamProvider;

        #[async_trait]
        impl ChatStreamProvider for HangingStreamProvider {
            async fn exec_chat_stream_events(
                &self,
                _model: &str,
                _chat_req: genai::chat::ChatRequest,
                _options: Option<&ChatOptions>,
            ) -> genai::Result<
                Pin<Box<dyn Stream<Item = genai::Result<genai::chat::ChatStreamEvent>> + Send>>,
            > {
                let stream = async_stream::stream! {
                    yield Ok(ChatStreamEvent::Start);
                    yield Ok(ChatStreamEvent::Chunk(StreamChunk {
                        content: "partial".to_string(),
                    }));
                    // Simulate a provider stream that hangs after emitting a partial response.
                    let _: () = futures::future::pending().await;
                };
                Ok(Box::pin(stream))
            }
        }

        let token = CancellationToken::new();
        let stream = run_loop_stream_impl_with_provider(
            Arc::new(HangingStreamProvider),
            AgentConfig::new("mock"),
            Session::new("test").with_message(Message::user("go")),
            HashMap::new(),
            RunContext {
                cancellation_token: Some(token.clone()),
            },
            None,
            None,
        );

        let collect_task = tokio::spawn(async move { collect_stream_events(stream).await });
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        token.cancel();

        let events = tokio::time::timeout(std::time::Duration::from_millis(250), collect_task)
            .await
            .expect("stream should stop shortly after cancellation")
            .expect("collector task should not panic");

        assert_eq!(extract_stop_reason(&events), Some(StopReason::Cancelled));
    }

    #[tokio::test]
    async fn test_stop_first_condition_wins() {
        // Both MaxRounds(1) and TokenBudget(50) should trigger after round 1.
        // MaxRounds is first in the list → it wins.
        let responses = vec![MockResponse::text("r1")
            .with_tool_call("c1", "echo", json!({"message": "a"}))
            .with_usage(100, 100)];

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::MaxRounds(1))
            .with_stop_condition(crate::stop::TokenBudget { max_total: 50 });
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        // MaxRounds listed first → wins
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[tokio::test]
    async fn test_stop_default_max_rounds_from_config() {
        // No explicit stop_conditions → auto-creates MaxRounds from config.max_rounds.
        let responses: Vec<MockResponse> = (0..5)
            .map(|i| {
                MockResponse::text(&format!("r{i}")).with_tool_call(
                    &format!("c{i}"),
                    "echo",
                    json!({"message": "a"}),
                )
            })
            .collect();

        let config = AgentConfig::new("mock").with_max_rounds(2);
        let session = Session::new("test").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[tokio::test]
    async fn test_stop_reason_in_run_finish_event() {
        // Verify RunFinish event structure when stop condition triggers.
        let responses =
            vec![MockResponse::text("r1").with_tool_call("c1", "echo", json!({"message": "a"}))];

        let config = AgentConfig::new("mock").with_stop_condition(crate::stop::MaxRounds(1));
        let session = Session::new("test-thread").with_message(Message::user("go"));
        let tools = tool_map([EchoTool]);

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;

        let finish = events
            .iter()
            .find(|e| matches!(e, AgentEvent::RunFinish { .. }));
        assert!(finish.is_some());
        if let Some(AgentEvent::RunFinish {
            thread_id,
            stop_reason,
            ..
        }) = finish
        {
            assert_eq!(thread_id, "test-thread");
            assert_eq!(*stop_reason, Some(StopReason::MaxRoundsReached));
        }
    }

    #[tokio::test]
    async fn test_consecutive_errors_resets_on_success() {
        // Round 1: failing tool (consecutive_errors=1)
        // Round 2: echo succeeds (consecutive_errors=0)
        // Round 3: failing tool (consecutive_errors=1)
        // ConsecutiveErrors(2) should NOT trigger — never reaches 2.
        let responses = vec![
            MockResponse::text("r1").with_tool_call("c1", "failing", json!({})),
            MockResponse::text("r2").with_tool_call("c2", "echo", json!({"message": "ok"})),
            MockResponse::text("r3").with_tool_call("c3", "failing", json!({})),
        ];

        let mut tools = tool_map([EchoTool]);
        let ft: Arc<dyn Tool> = Arc::new(FailingTool);
        tools.insert("failing".to_string(), ft);

        let config = AgentConfig::new("mock")
            .with_stop_condition(crate::stop::ConsecutiveErrors(2))
            .with_stop_condition(crate::stop::MaxRounds(3));
        let session = Session::new("test").with_message(Message::user("go"));

        let events =
            run_mock_stream(MockStreamProvider::new(responses), config, session, tools).await;
        // Should hit MaxRounds(3), not ConsecutiveErrors
        assert_eq!(
            extract_stop_reason(&events),
            Some(StopReason::MaxRoundsReached)
        );
    }

    #[tokio::test]
    async fn test_loop_state_tracks_rounds() {
        let mut state = LoopState::new();
        assert_eq!(state.rounds, 0);

        let tool_calls = vec![crate::types::ToolCall::new("c1", "echo", json!({}))];
        state.record_tool_round(&tool_calls, 0);
        state.rounds += 1;
        assert_eq!(state.rounds, 1);
        assert_eq!(state.consecutive_errors, 0);
        assert_eq!(state.tool_call_history.len(), 1);
    }

    #[tokio::test]
    async fn test_loop_state_tracks_token_usage() {
        let mut state = LoopState::new();
        let result = StreamResult {
            text: "hello".to_string(),
            tool_calls: vec![],
            usage: Some(Usage {
                prompt_tokens: Some(100),
                prompt_tokens_details: None,
                completion_tokens: Some(50),
                completion_tokens_details: None,
                total_tokens: Some(150),
            }),
        };
        state.update_from_response(&result);
        assert_eq!(state.total_input_tokens, 100);
        assert_eq!(state.total_output_tokens, 50);

        state.update_from_response(&result);
        assert_eq!(state.total_input_tokens, 200);
        assert_eq!(state.total_output_tokens, 100);
    }

    #[tokio::test]
    async fn test_loop_state_caps_history_at_20() {
        let mut state = LoopState::new();
        for i in 0..25 {
            let tool_calls = vec![crate::types::ToolCall::new(
                &format!("c{i}"),
                &format!("tool_{i}"),
                json!({}),
            )];
            state.record_tool_round(&tool_calls, 0);
        }
        assert_eq!(state.tool_call_history.len(), 20);
    }

    #[tokio::test]
    async fn test_effective_stop_conditions_empty_uses_max_rounds() {
        let config = AgentConfig::new("mock").with_max_rounds(5);
        let conditions = effective_stop_conditions(&config);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].id(), "max_rounds");
    }

    #[tokio::test]
    async fn test_effective_stop_conditions_explicit_overrides() {
        let config = AgentConfig::new("mock")
            .with_max_rounds(5) // ignored when explicit conditions set
            .with_stop_condition(crate::stop::Timeout(std::time::Duration::from_secs(30)));
        let conditions = effective_stop_conditions(&config);
        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].id(), "timeout");
    }

    // ========================================================================
    // Parallel Tool Execution: Partial Failure Tests
    // ========================================================================

    #[test]
    fn test_parallel_tools_partial_failure() {
        // When running tools in parallel, a failing tool should produce an error
        // message, while the successful tool should still complete.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Call both".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_echo", "echo", json!({"message": "ok"})),
                    crate::types::ToolCall::new("call_fail", "failing", json!({})),
                ],
                usage: None,
            };

            let mut tools = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>);
            tools.insert(
                "failing".to_string(),
                Arc::new(FailingTool) as Arc<dyn Tool>,
            );

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Both tools produce messages.
            assert_eq!(
                session.message_count(),
                2,
                "Both tools should produce a message"
            );

            // One should be success, one should be error.
            let contents: Vec<&str> = session
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect();
            let has_success = contents.iter().any(|c| c.contains("echoed"));
            let has_error = contents
                .iter()
                .any(|c| c.to_lowercase().contains("error") || c.to_lowercase().contains("fail"));
            assert!(has_success, "Echo tool should succeed: {:?}", contents);
            assert!(
                has_error,
                "Failing tool should produce error: {:?}",
                contents
            );
        });
    }

    #[test]
    fn test_parallel_tools_conflicting_state_patches_return_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "conflicting calls".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "counter", json!({"amount": 1})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 2})),
                ],
                usage: None,
            };
            let tools = tool_map([CounterTool]);

            let err = execute_tools(session, &result, &tools, true)
                .await
                .expect_err("parallel conflicting patches should fail");
            assert!(
                matches!(err, AgentLoopError::StateError(ref msg) if msg.contains("conflict")),
                "expected conflict state error, got: {err:?}"
            );
        });
    }

    #[test]
    fn test_sequential_tools_partial_failure() {
        // Same test but with sequential execution.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Call both".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_echo", "echo", json!({"message": "ok"})),
                    crate::types::ToolCall::new("call_fail", "failing", json!({})),
                ],
                usage: None,
            };

            let mut tools = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool) as Arc<dyn Tool>);
            tools.insert(
                "failing".to_string(),
                Arc::new(FailingTool) as Arc<dyn Tool>,
            );

            let session = execute_tools(session, &result, &tools, false)
                .await
                .unwrap();

            assert_eq!(
                session.message_count(),
                2,
                "Both tools should produce a message"
            );
            let contents: Vec<&str> = session
                .messages
                .iter()
                .map(|m| m.content.as_str())
                .collect();
            let has_success = contents.iter().any(|c| c.contains("echoed"));
            let has_error = contents
                .iter()
                .any(|c| c.to_lowercase().contains("error") || c.to_lowercase().contains("fail"));
            assert!(has_success, "Echo tool should succeed: {:?}", contents);
            assert!(
                has_error,
                "Failing tool should produce error: {:?}",
                contents
            );
        });
    }

    // ========================================================================
    // Plugin Execution Order Tests
    // ========================================================================

    /// Plugin that records when it runs (appends to a shared Vec).
    struct OrderTrackingPlugin {
        id: &'static str,
        order_log: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl AgentPlugin for OrderTrackingPlugin {
        fn id(&self) -> &str {
            self.id
        }

        async fn on_phase(&self, phase: Phase, _step: &mut StepContext<'_>) {
            self.order_log
                .lock()
                .unwrap()
                .push(format!("{}:{:?}", self.id, phase));
        }
    }

    #[test]
    fn test_plugin_execution_order_preserved() {
        // Plugins should execute in the order they are provided.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let log = Arc::new(std::sync::Mutex::new(Vec::new()));

            let plugin_a = OrderTrackingPlugin {
                id: "plugin_a",
                order_log: Arc::clone(&log),
            };
            let plugin_b = OrderTrackingPlugin {
                id: "plugin_b",
                order_log: Arc::clone(&log),
            };

            let session = Session::new("test");
            let result = StreamResult {
                text: "Test".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(plugin_a), Arc::new(plugin_b)];

            let _ = execute_tools_with_plugins(session, &result, &tools, false, &plugins).await;

            let entries = log.lock().unwrap().clone();

            // For each phase, plugin_a should appear before plugin_b.
            let before_a = entries
                .iter()
                .position(|e| e.starts_with("plugin_a:BeforeToolExecute"));
            let before_b = entries
                .iter()
                .position(|e| e.starts_with("plugin_b:BeforeToolExecute"));
            if let (Some(a), Some(b)) = (before_a, before_b) {
                assert!(
                    a < b,
                    "plugin_a should run before plugin_b in BeforeToolExecute phase"
                );
            }

            let after_a = entries
                .iter()
                .position(|e| e.starts_with("plugin_a:AfterToolExecute"));
            let after_b = entries
                .iter()
                .position(|e| e.starts_with("plugin_b:AfterToolExecute"));
            if let (Some(a), Some(b)) = (after_a, after_b) {
                assert!(
                    a < b,
                    "plugin_a should run before plugin_b in AfterToolExecute phase"
                );
            }
        });
    }

    /// Plugin that blocks if it runs after another plugin has already set pending.
    struct ConditionalBlockPlugin;

    #[async_trait]
    impl AgentPlugin for ConditionalBlockPlugin {
        fn id(&self) -> &str {
            "conditional_block"
        }

        async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            if phase == Phase::BeforeToolExecute && step.tool_pending() {
                step.block("Blocked because tool was pending".to_string());
            }
        }
    }

    #[test]
    fn test_plugin_order_affects_outcome() {
        // When PendingPhasePlugin runs FIRST, it sets pending. Then
        // ConditionalBlockPlugin sees pending and blocks. Net result: blocked.
        // When reversed, ConditionalBlockPlugin sees no pending (does nothing),
        // then PendingPhasePlugin sets pending. Net result: pending (not blocked).
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Test".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
                usage: None,
            };
            let tools = tool_map([EchoTool]);

            // Order 1: PendingPhasePlugin first → ConditionalBlockPlugin blocks.
            let plugins_order1: Vec<Arc<dyn AgentPlugin>> = vec![
                Arc::new(PendingPhasePlugin),
                Arc::new(ConditionalBlockPlugin),
            ];
            let r1 = execute_tools_with_plugins(
                session.clone(),
                &result,
                &tools,
                false,
                &plugins_order1,
            )
            .await;
            // When pending+blocked, the blocked result takes priority.
            let s1 = r1.unwrap();
            assert_eq!(s1.message_count(), 1);
            assert!(
                s1.messages[0].content.to_lowercase().contains("blocked")
                    || s1.messages[0].content.to_lowercase().contains("pending"),
                "Order 1 should block or produce error: {}",
                s1.messages[0].content
            );

            // Order 2: ConditionalBlockPlugin first → sees no pending → PendingPhasePlugin sets pending.
            let plugins_order2: Vec<Arc<dyn AgentPlugin>> = vec![
                Arc::new(ConditionalBlockPlugin),
                Arc::new(PendingPhasePlugin),
            ];
            let r2 =
                execute_tools_with_plugins(session, &result, &tools, false, &plugins_order2).await;
            // Should be PendingInteraction (not blocked).
            assert!(r2.is_err(), "Order 2 should result in PendingInteraction");
            match r2.unwrap_err() {
                AgentLoopError::PendingInteraction { .. } => {}
                other => panic!("Expected PendingInteraction, got: {:?}", other),
            }
        });
    }
}
