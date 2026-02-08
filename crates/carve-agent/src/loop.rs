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
use crate::convert::{assistant_message, assistant_tool_calls, build_request, tool_response};
use crate::execute::{collect_patches, execute_single_tool, ToolExecution};
use crate::phase::{Phase, StepContext, ToolContext};
use crate::plugin::AgentPlugin;
use crate::session::Session;
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::types::Message;
use async_stream::stream;
use carve_state::ActivityManager;
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::Instrument;

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
    /// Chat options for the LLM.
    pub chat_options: Option<ChatOptions>,
    /// Plugins to run during the agent loop.
    pub plugins: Vec<Arc<dyn AgentPlugin>>,
    /// Tool whitelist (None = all tools available).
    pub allowed_tools: Option<Vec<String>>,
    /// Tool blacklist.
    pub excluded_tools: Option<Vec<String>>,
}

/// Backwards-compatible alias.
pub type AgentConfig = AgentDefinition;

impl Default for AgentDefinition {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            parallel_tools: true,
            chat_options: Some(ChatOptions::default().with_capture_usage(true)),
            plugins: Vec::new(),
            allowed_tools: None,
            excluded_tools: None,
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
            .field("chat_options", &self.chat_options)
            .field("plugins", &format!("[{} plugins]", self.plugins.len()))
            .field("allowed_tools", &self.allowed_tools)
            .field("excluded_tools", &self.excluded_tools)
            .finish()
    }
}

impl AgentDefinition {
    /// Create a new definition with id and model.
    /// Create a new definition with id and model.
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

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty()
    }
}

/// Emit a phase to all plugins.
async fn emit_phase(phase: Phase, step: &mut StepContext<'_>, plugins: &[Arc<dyn AgentPlugin>]) {
    for plugin in plugins {
        plugin.on_phase(phase, step).await;
    }
}

/// Build initial plugin data map.
fn initial_plugin_data(plugins: &[Arc<dyn AgentPlugin>]) -> HashMap<String, Value> {
    let mut data = HashMap::new();
    for plugin in plugins {
        if let Some((key, value)) = plugin.initial_data() {
            data.insert(key.to_string(), value);
        }
    }
    data
}

/// Runtime plugin data shared across phase contexts.
#[derive(Debug, Clone, Default)]
struct PluginRuntimeData {
    data: HashMap<String, Value>,
}

impl PluginRuntimeData {
    fn new(plugins: &[Arc<dyn AgentPlugin>]) -> Self {
        Self {
            data: initial_plugin_data(plugins),
        }
    }

    fn new_step_context<'a>(
        &self,
        session: &'a Session,
        tools: Vec<ToolDescriptor>,
    ) -> StepContext<'a> {
        let mut step = StepContext::new(session, tools);
        step.set_data_map(self.data.clone());
        step
    }

    fn sync_from_step(&mut self, step: &StepContext<'_>) {
        self.data = step.data_snapshot();
    }
}

/// Build messages from StepContext for LLM request.
fn build_messages(step: &mut StepContext<'_>, system_prompt: &str) {
    step.messages.clear();

    // [1] System Prompt + Context
    let system = if step.system_context.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, step.system_context.join("\n"))
    };

    if !system.is_empty() {
        step.messages.push(Message::system(system));
    }

    // [2] Session Context
    for ctx in &step.session_context {
        step.messages.push(Message::system(ctx.clone()));
    }

    // [3+] History from session
    step.messages.extend(step.session.messages.clone());
}

/// Run one step of the agent loop (non-streaming).
///
/// A step consists of:
/// 1. Emit StepStart phase
/// 2. Emit BeforeInference phase
/// 3. Send messages to LLM
/// 4. Emit AfterInference phase
/// 5. If LLM returns tool calls, execute them with BeforeToolExecute/AfterToolExecute phases
/// 6. Emit StepEnd phase
/// 7. Return the result
pub async fn run_step(
    client: &Client,
    config: &AgentConfig,
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, StreamResult), AgentLoopError> {
    // Get tool descriptors, applying definition-level filtering
    let tool_descriptors: Vec<ToolDescriptor> = tools
        .values()
        .map(|t| t.descriptor().clone())
        .filter(|td| {
            if let Some(ref allowed) = config.allowed_tools {
                if !allowed.contains(&td.id) {
                    return false;
                }
            }
            if let Some(ref excluded) = config.excluded_tools {
                if excluded.contains(&td.id) {
                    return false;
                }
            }
            true
        })
        .collect();
    let mut plugin_data = PluginRuntimeData::new(&config.plugins);

    // Create StepContext - use scoped blocks to manage borrows
    let (messages, filtered_tools, skip_inference, tracing_span) = {
        let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());

        // Phase 1: StepStart
        emit_phase(Phase::StepStart, &mut step, &config.plugins).await;

        // Phase 2: BeforeInference
        emit_phase(Phase::BeforeInference, &mut step, &config.plugins).await;

        // Build messages
        build_messages(&mut step, &config.system_prompt);

        // Get data before dropping step
        let skip = step.skip_inference;
        let msgs = step.messages.clone();
        let tools_filter: Vec<String> = step.tools.iter().map(|td| td.id.clone()).collect();
        let tracing_span = step.tracing_span.take();
        plugin_data.sync_from_step(&step);

        (msgs, tools_filter, skip, tracing_span)
    };

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
    let response = async {
        client
            .exec_chat(&config.model, request, config.chat_options.as_ref())
            .await
    }
    .instrument(inference_span)
    .await
    .map_err(|e| AgentLoopError::LlmError(e.to_string()))?;

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

    let result = StreamResult { text, tool_calls, usage: None };

    // Phase 3: AfterInference (with new context)
    {
        let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());
        step.response = Some(result.clone());
        emit_phase(Phase::AfterInference, &mut step, &config.plugins).await;
        plugin_data.sync_from_step(&step);
    }

    // Add assistant message
    let session = if result.tool_calls.is_empty() {
        session.with_message(assistant_message(&result.text))
    } else {
        session.with_message(assistant_tool_calls(
            &result.text,
            result.tool_calls.clone(),
        ))
    };

    // Phase 6: StepEnd (with new context)
    {
        let mut step = plugin_data.new_step_context(&session, tool_descriptors);
        emit_phase(Phase::StepEnd, &mut step, &config.plugins).await;
        plugin_data.sync_from_step(&step);
    }

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
    if result.tool_calls.is_empty() {
        return Ok(session);
    }

    // Get current state
    let state = session
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    // Get tool descriptors
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let mut plugin_data = PluginRuntimeData::new(&config.plugins);

    // Execute each tool with phase hooks
    let mut executions = Vec::new();

    for call in &result.tool_calls {
        // Create StepContext for this tool
        let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());
        step.tool = Some(ToolContext::new(call));

        // Phase 4: BeforeToolExecute
        emit_phase(Phase::BeforeToolExecute, &mut step, &config.plugins).await;

        // Check if blocked
        let execution = if step.tool_blocked() {
            let reason = step
                .tool
                .as_ref()
                .and_then(|t| t.block_reason.clone())
                .unwrap_or_else(|| "Blocked by plugin".to_string());
            ToolExecution {
                call: call.clone(),
                result: ToolResult::error(&call.name, reason),
                patch: None,
            }
        } else if step.tool_pending() {
            ToolExecution {
                call: call.clone(),
                result: ToolResult::pending(&call.name, "Waiting for user confirmation"),
                patch: None,
            }
        } else {
            // Execute the tool
            let tool = tools.get(&call.name).cloned();
            execute_single_tool(tool.as_deref(), call, &state).await
        };

        // Set tool result in context
        step.set_tool_result(execution.result.clone());

        // Phase 5: AfterToolExecute
        emit_phase(Phase::AfterToolExecute, &mut step, &config.plugins).await;
        plugin_data.sync_from_step(&step);

        executions.push((execution, step.system_reminders.clone()));
    }

    // Collect patches and tool response messages
    let patches = collect_patches(
        &executions
            .iter()
            .map(|(e, _)| e.clone())
            .collect::<Vec<_>>(),
    );
    let tool_messages: Vec<Message> = executions
        .iter()
        .flat_map(|(e, reminders)| {
            let mut msgs = vec![tool_response(&e.call.id, &e.result)];
            for r in reminders {
                msgs.push(Message::system(format!(
                    "<system-reminder>{}</system-reminder>",
                    r
                )));
            }
            msgs
        })
        .collect();

    // Update session
    let session = session.with_patches(patches).with_messages(tool_messages);

    Ok(session)
}

/// Execute tool calls (simple version without config).
pub async fn execute_tools_simple(
    session: Session,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<Session, AgentLoopError> {
    execute_tools_with_plugins(session, result, tools, parallel, &[]).await
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

    // Get current state
    let state = session
        .rebuild_state()
        .map_err(|e| AgentLoopError::StateError(e.to_string()))?;

    // Get tool descriptors
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();
    let plugin_data = initial_plugin_data(plugins);

    // Execute tools
    let results = if parallel {
        execute_tools_parallel_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            plugins,
            plugin_data.clone(),
            None,
        )
        .await
    } else {
        execute_tools_sequential_with_phases(
            tools,
            &result.tool_calls,
            &state,
            &tool_descriptors,
            plugins,
            plugin_data.clone(),
            None,
        )
        .await
    };

    // Collect patches and tool response messages
    let patches = collect_patches(
        &results
            .iter()
            .map(|r| r.execution.clone())
            .collect::<Vec<_>>(),
    );
    let tool_messages: Vec<Message> = results
        .iter()
        .flat_map(|r| {
            let mut msgs = vec![tool_response(&r.execution.call.id, &r.execution.result)];
            for reminder in &r.reminders {
                msgs.push(Message::system(format!(
                    "<system-reminder>{}</system-reminder>",
                    reminder
                )));
            }
            msgs
        })
        .collect();

    // Update session
    let session = session.with_patches(patches).with_messages(tool_messages);

    Ok(session)
}

/// Execute tools in parallel with phase hooks.
async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::types::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    plugin_data: HashMap<String, Value>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
) -> Vec<ToolExecutionResult> {
    use futures::future::join_all;

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();
        let tool_descriptors = tool_descriptors.to_vec();
        let plugin_data = plugin_data.clone();
        let activity_manager = activity_manager.clone();

        async move {
            execute_single_tool_with_phases(
                tool.as_deref(),
                &call,
                &state,
                &tool_descriptors,
                &plugins,
                plugin_data,
                activity_manager,
            )
            .await
        }
    });

    join_all(futures).await
}

/// Execute tools sequentially with phase hooks.
async fn execute_tools_sequential_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::types::ToolCall],
    initial_state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    mut plugin_data: HashMap<String, Value>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
) -> Vec<ToolExecutionResult> {
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
            plugin_data.clone(),
            activity_manager.clone(),
        )
        .await;

        // Apply patch to state for next tool
        if let Some(ref patch) = result.execution.patch {
            if let Ok(new_state) = apply_patch(&state, patch.patch()) {
                state = new_state;
            }
        }

        plugin_data = result.plugin_data.clone();
        results.push(result);
    }

    results
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
    pub plugin_data: HashMap<String, Value>,
}

/// Execute a single tool with phase hooks.
async fn execute_single_tool_with_phases(
    tool: Option<&dyn Tool>,
    call: &crate::types::ToolCall,
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
    plugin_data: HashMap<String, Value>,
    activity_manager: Option<Arc<dyn ActivityManager>>,
) -> ToolExecutionResult {
    // Create a minimal session for StepContext
    let temp_session = Session::with_initial_state("temp", state.clone());

    // Create StepContext for this tool
    let mut step = StepContext::new(&temp_session, tool_descriptors.to_vec());
    step.set_data_map(plugin_data);
    step.tool = Some(ToolContext::new(call));

    // Phase: BeforeToolExecute
    emit_phase(Phase::BeforeToolExecute, &mut step, plugins).await;

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
        );
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
    emit_phase(Phase::AfterToolExecute, &mut step, plugins).await;

    ToolExecutionResult {
        execution,
        reminders: step.system_reminders.clone(),
        pending_interaction,
        plugin_data: step.data_snapshot(),
    }
}

/// Run the full agent loop until completion or max rounds.
///
/// Returns the final session and the last response text.
pub async fn run_loop(
    client: &Client,
    config: &AgentConfig,
    mut session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, String), AgentLoopError> {
    let mut rounds = 0;
    let mut last_text;
    let mut plugin_data = PluginRuntimeData::new(&config.plugins);

    // Get tool descriptors for SessionStart
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();

    // Create StepContext for session lifecycle
    let mut session_step = plugin_data.new_step_context(&session, tool_descriptors.clone());

    // Phase: SessionStart
    emit_phase(Phase::SessionStart, &mut session_step, &config.plugins).await;
    plugin_data.sync_from_step(&session_step);

    loop {
        // Run one step
        let (new_session, result) = run_step(client, config, session, tools).await?;
        session = new_session;
        last_text = result.text.clone();

        // Check if we need to execute tools
        if !result.needs_tools() {
            // No tools - we're done
            break;
        }

        // Execute tools
        session = execute_tools_with_config(session, &result, tools, config).await?;

        rounds += 1;
        if rounds >= config.max_rounds {
            return Err(AgentLoopError::MaxRoundsExceeded(config.max_rounds));
        }
    }

    // Phase: SessionEnd
    let mut end_step = plugin_data.new_step_context(&session, tool_descriptors);
    emit_phase(Phase::SessionEnd, &mut end_step, &config.plugins).await;

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
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream! {
    let mut session = session;
    let mut rounds = 0;
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel();
        let activity_manager: Arc<dyn ActivityManager> = Arc::new(ActivityHub::new(activity_tx));

    // Get tool descriptors, applying definition-level filtering
    let tool_descriptors: Vec<ToolDescriptor> = tools
        .values()
        .map(|t| t.descriptor().clone())
        .filter(|td| {
            if let Some(ref allowed) = config.allowed_tools {
                if !allowed.contains(&td.id) {
                    return false;
                }
            }
            if let Some(ref excluded) = config.excluded_tools {
                if excluded.contains(&td.id) {
                    return false;
                }
            }
            true
        })
        .collect();
    let mut plugin_data = PluginRuntimeData::new(&config.plugins);

        // Phase: SessionStart (use scoped block to manage borrow)
        {
            let mut session_step = plugin_data.new_step_context(&session, tool_descriptors.clone());
            emit_phase(Phase::SessionStart, &mut session_step, &config.plugins).await;
            plugin_data.sync_from_step(&session_step);
        }

        loop {
            // Phase: StepStart and BeforeInference (collect messages and tools filter)
            let (messages, filtered_tools, skip_inference, tracing_span) = {
                let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());

                emit_phase(Phase::StepStart, &mut step, &config.plugins).await;
                emit_phase(Phase::BeforeInference, &mut step, &config.plugins).await;

                build_messages(&mut step, &config.system_prompt);

                let skip = step.skip_inference;
                let msgs = step.messages.clone();
                let tools_filter: Vec<String> = step.tools.iter().map(|td| td.id.clone()).collect();
                let tracing_span = step.tracing_span.take();
                plugin_data.sync_from_step(&step);

                (msgs, tools_filter, skip, tracing_span)
            };

            // Skip inference if requested
            if skip_inference {
                yield AgentEvent::Done { response: String::new() };
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
                client
                    .exec_chat_stream(&config.model, request, config.chat_options.as_ref())
                    .await
            }
            .instrument(inference_span)
            .await;

            let chat_stream_response = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            };

            // Collect streaming response
            let inference_start = std::time::Instant::now();
            let mut collector = StreamCollector::new();
            let mut chat_stream = chat_stream_response.stream;

            while let Some(event_result) = chat_stream.next().await {
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
                        yield AgentEvent::Error { message: e.to_string() };
                        return;
                    }
                }
            }

            let result = collector.finish();
            let inference_duration_ms = inference_start.elapsed().as_millis() as u64;

            yield AgentEvent::InferenceComplete {
                model: config.model.clone(),
                usage: result.usage.clone(),
                duration_ms: inference_duration_ms,
            };

            // Phase: AfterInference (with new context)
            {
                let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());
                step.response = Some(result.clone());
                emit_phase(Phase::AfterInference, &mut step, &config.plugins).await;
                plugin_data.sync_from_step(&step);
            }

            // Step boundary: finished LLM call
            yield AgentEvent::StepEnd;

            // Add assistant message
            session = if result.tool_calls.is_empty() {
                session.with_message(assistant_message(&result.text))
            } else {
                session.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()))
            };

            // Phase: StepEnd (with new context)
            {
                let mut step = plugin_data.new_step_context(&session, tool_descriptors.clone());
                emit_phase(Phase::StepEnd, &mut step, &config.plugins).await;
                plugin_data.sync_from_step(&step);
            }

            // Check if we need to execute tools
            if !result.needs_tools() {
                // Phase: SessionEnd
                let mut end_step = plugin_data.new_step_context(&session, tool_descriptors.clone());
                emit_phase(Phase::SessionEnd, &mut end_step, &config.plugins).await;
                plugin_data.sync_from_step(&end_step);

                yield AgentEvent::Done { response: result.text };
                return;
            }

            // Execute tools with phase hooks
            let state = match session.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            };

            let mut tool_future: Pin<Box<dyn Future<Output = Vec<ToolExecutionResult>> + Send>> =
                if config.parallel_tools {
                    Box::pin(execute_tools_parallel_with_phases(
                        &tools,
                        &result.tool_calls,
                        &state,
                        &tool_descriptors,
                        &config.plugins,
                        plugin_data.data.clone(),
                        Some(activity_manager.clone()),
                    ))
                } else {
                    Box::pin(execute_tools_sequential_with_phases(
                        &tools,
                        &result.tool_calls,
                        &state,
                        &tool_descriptors,
                        &config.plugins,
                        plugin_data.data.clone(),
                        Some(activity_manager.clone()),
                    ))
                };
            let mut activity_closed = false;
            let results = loop {
                tokio::select! {
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

            if !config.parallel_tools {
                if let Some(last) = results.last() {
                    plugin_data.data = last.plugin_data.clone();
                }
            }

            // Check if any tool has pending interaction
            let mut has_pending = false;
            for exec_result in &results {
                if let Some(ref interaction) = exec_result.pending_interaction {
                    // Emit pending interaction event
                    yield AgentEvent::Pending {
                        interaction: interaction.clone(),
                    };
                    has_pending = true;
                }
            }

            // If there are pending interactions, pause the loop
            // Client must respond and start a new run to continue
            if has_pending {
                // Still emit non-pending tool results
                for exec_result in &results {
                    if exec_result.pending_interaction.is_none() {
                        yield AgentEvent::ToolCallDone {
                            id: exec_result.execution.call.id.clone(),
                            result: exec_result.execution.result.clone(),
                            patch: exec_result.execution.patch.clone(),
                        };
                    }
                }
                // End with Pending state (not Done, not Error)
                return;
            }

            // Emit tool results and collect patches/messages
            let patches = collect_patches(&results.iter().map(|r| r.execution.clone()).collect::<Vec<_>>());
            let mut tool_messages = Vec::new();

            for exec_result in &results {
                yield AgentEvent::ToolCallDone {
                    id: exec_result.execution.call.id.clone(),
                    result: exec_result.execution.result.clone(),
                    patch: exec_result.execution.patch.clone(),
                };
                tool_messages.push(tool_response(&exec_result.execution.call.id, &exec_result.execution.result));
                for r in &exec_result.reminders {
                    tool_messages.push(Message::system(format!(
                        "<system-reminder>{}</system-reminder>",
                        r
                    )));
                }
            }

            session = session.with_patches(patches).with_messages(tool_messages);

            rounds += 1;
            if rounds >= config.max_rounds {
                yield AgentEvent::Error { message: format!("Max rounds ({}) exceeded", config.max_rounds) };
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
#[derive(Debug, Clone)]
pub enum AgentLoopError {
    /// LLM API error.
    LlmError(String),
    /// State rebuild error.
    StateError(String),
    /// Max rounds exceeded.
    MaxRoundsExceeded(usize),
}

impl std::fmt::Display for AgentLoopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LlmError(e) => write!(f, "LLM error: {}", e),
            Self::StateError(e) => write!(f, "State error: {}", e),
            Self::MaxRoundsExceeded(n) => write!(f, "Max rounds ({}) exceeded", n),
        }
    }
}

impl std::error::Error for AgentLoopError {}

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
    use carve_state::{ActivityManager, Context};
    use carve_state_derive::State;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};
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

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();
        assert_eq!(config.max_rounds, 10);
        assert!(config.parallel_tools);
        assert!(config.system_prompt.is_empty());
    }

    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("gpt-4")
            .with_max_rounds(5)
            .with_parallel_tools(false)
            .with_system_prompt("You are helpful.");

        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_rounds, 5);
        assert!(!config.parallel_tools);
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

        let err = AgentLoopError::MaxRoundsExceeded(10);
        assert!(err.to_string().contains("10"));
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

            let session = execute_tools_simple(session, &result, &tools, true)
                .await
                .unwrap();
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

            let session = execute_tools_simple(session, &result, &tools, true)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.messages[0].role, crate::types::Role::Tool);
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

        let result = tool_future.await;
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
            )
            .await
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

            let session = execute_tools_simple(session, &result, &tools, true)
                .await
                .unwrap();

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

            let session = execute_tools_simple(session, &result, &tools, true)
                .await
                .unwrap();

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
                    if step.tool_id() == Some("echo") {
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
            if phase == Phase::BeforeToolExecute {
                if step.tool_id() == Some("echo") {
                    step.block("Echo tool is blocked");
                }
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

        build_messages(&mut step, "Base system prompt");

        assert_eq!(step.messages.len(), 3);
        // First message should be system prompt + context
        assert!(step.messages[0].content.contains("Base system prompt"));
        assert!(step.messages[0].content.contains("System context 1"));
        assert!(step.messages[0].content.contains("System context 2"));
        // Second message should be session context
        assert_eq!(step.messages[1].content, "Session context");
        // Third message should be user message
        assert_eq!(step.messages[2].content, "Hello");
    }

    #[test]
    fn test_build_messages_empty_system() {
        let session = Session::new("test").with_message(Message::user("Hello"));
        let mut step = StepContext::new(&session, vec![]);

        build_messages(&mut step, "");

        // Only user message
        assert_eq!(step.messages.len(), 1);
        assert_eq!(step.messages[0].content, "Hello");
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

            emit_phase(Phase::BeforeInference, &mut step, &plugins).await;

            assert_eq!(step.tools.len(), 1);
            assert_eq!(step.tools[0].id, "safe_tool");
        });
    }

    #[test]
    fn test_plugin_data_initialization() {
        struct DataPlugin;

        #[async_trait]
        impl AgentPlugin for DataPlugin {
            fn id(&self) -> &str {
                "data"
            }

            async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}

            fn initial_data(&self) -> Option<(&'static str, Value)> {
                Some(("plugin_config", json!({"enabled": true})))
            }
        }

        let session = Session::new("test");
        let mut step = StepContext::new(&session, vec![]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(DataPlugin)];
        let runtime_data = PluginRuntimeData::new(&plugins);
        step.set_data_map(runtime_data.data);

        let config: Option<serde_json::Map<String, Value>> = step.get("plugin_config");
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

            fn initial_data(&self) -> Option<(&'static str, Value)> {
                Some(("allow_exec", json!(true)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                if phase == Phase::BeforeToolExecute && step.get::<bool>("allow_exec") != Some(true)
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
            initial_plugin_data(&plugins),
            None,
        )
        .await;

        assert!(result.execution.result.is_success());
    }

    #[tokio::test]
    async fn test_plugin_data_persists_across_phase_contexts() {
        struct LifecyclePlugin;

        #[async_trait]
        impl AgentPlugin for LifecyclePlugin {
            fn id(&self) -> &str {
                "lifecycle"
            }

            fn initial_data(&self) -> Option<(&'static str, Value)> {
                Some(("seed", json!(1)))
            }

            async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
                match phase {
                    Phase::SessionStart => {
                        step.set("seed", 2);
                    }
                    Phase::StepStart => {
                        if step.get::<i64>("seed") == Some(2) {
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
        let mut plugin_data = PluginRuntimeData::new(&plugins);

        let mut session_step = plugin_data.new_step_context(&session, tools.clone());
        emit_phase(Phase::SessionStart, &mut session_step, &plugins).await;
        plugin_data.sync_from_step(&session_step);

        let mut step = plugin_data.new_step_context(&session, tools);
        emit_phase(Phase::StepStart, &mut step, &plugins).await;

        assert_eq!(step.system_context, vec!["seed_visible".to_string()]);
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
            if phase == Phase::BeforeToolExecute {
                if step.tool_id() == Some("echo") {
                    use crate::state_types::Interaction;
                    step.pending(
                        Interaction::new("confirm_1", "confirm").with_message("Execute echo?"),
                    );
                }
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

            let session = execute_tools_with_plugins(session, &result, &tools, true, &plugins)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            // Pending tool should return pending status
            assert!(
                msg.content.contains("pending")
                    || msg.content.contains("Pending")
                    || msg.content.contains("Waiting"),
                "Expected pending in message, got: {}",
                msg.content
            );
        });
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

            let session = execute_tools_simple(session, &result, &tools, true)
                .await
                .unwrap();

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

            let session = execute_tools_with_config(session, &result, &tools, &config)
                .await
                .unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("pending")
                    || msg.content.contains("Pending")
                    || msg.content.contains("Waiting"),
                "Expected pending in message, got: {}",
                msg.content
            );
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
}
