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
//! │      TurnStart          │ ← plugins can inject system context
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
//! │       TurnEnd           │
//! └─────────────────────────┘
//!     │
//!     ▼
//! SessionEnd (once)
//! ```

use crate::convert::{assistant_message, assistant_tool_calls, build_request, tool_response};
use crate::execute::{collect_patches, execute_single_tool, ToolExecution};
use crate::phase::{Phase, ToolContext, TurnContext};
use crate::plugin::AgentPlugin;
use crate::session::Session;
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::traits::tool::{Tool, ToolDescriptor, ToolResult};
use crate::types::Message;
use async_stream::stream;
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use serde_json::Value;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// Configuration for the agent loop.
#[derive(Clone)]
pub struct AgentConfig {
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
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o-mini".to_string(),
            system_prompt: String::new(),
            max_rounds: 10,
            parallel_tools: true,
            chat_options: None,
            plugins: Vec::new(),
        }
    }
}

impl std::fmt::Debug for AgentConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentConfig")
            .field("model", &self.model)
            .field("system_prompt", &format!("[{} chars]", self.system_prompt.len()))
            .field("max_rounds", &self.max_rounds)
            .field("parallel_tools", &self.parallel_tools)
            .field("chat_options", &self.chat_options)
            .field("plugins", &format!("[{} plugins]", self.plugins.len()))
            .finish()
    }
}

impl AgentConfig {
    /// Create a new config with the specified model.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
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

    /// Check if any plugins are configured.
    pub fn has_plugins(&self) -> bool {
        !self.plugins.is_empty()
    }
}

/// Emit a phase to all plugins.
async fn emit_phase(phase: Phase, turn: &mut TurnContext<'_>, plugins: &[Arc<dyn AgentPlugin>]) {
    for plugin in plugins {
        plugin.on_phase(phase, turn).await;
    }
}

/// Initialize plugin data in TurnContext.
fn init_plugin_data(turn: &mut TurnContext<'_>, plugins: &[Arc<dyn AgentPlugin>]) {
    for plugin in plugins {
        if let Some((key, value)) = plugin.initial_data() {
            turn.set(key, value);
        }
    }
}

/// Build messages from TurnContext for LLM request.
fn build_messages(turn: &mut TurnContext<'_>, system_prompt: &str) {
    turn.messages.clear();

    // [1] System Prompt + Context
    let system = if turn.system_context.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, turn.system_context.join("\n"))
    };

    if !system.is_empty() {
        turn.messages.push(Message::system(system));
    }

    // [2] Session Context
    for ctx in &turn.session_context {
        turn.messages.push(Message::system(ctx.clone()));
    }

    // [3+] History from session
    turn.messages.extend(turn.session.messages.clone());
}

/// Run one turn of the agent loop (non-streaming).
///
/// A turn consists of:
/// 1. Emit TurnStart phase
/// 2. Emit BeforeInference phase
/// 3. Send messages to LLM
/// 4. Emit AfterInference phase
/// 5. If LLM returns tool calls, execute them with BeforeToolExecute/AfterToolExecute phases
/// 6. Emit TurnEnd phase
/// 7. Return the result
pub async fn run_turn(
    client: &Client,
    config: &AgentConfig,
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, StreamResult), AgentLoopError> {
    // Get tool descriptors
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();

    // Create TurnContext - use scoped blocks to manage borrows
    let (messages, filtered_tools, skip_inference) = {
        let mut turn = TurnContext::new(&session, tool_descriptors.clone());

        // Phase 1: TurnStart
        emit_phase(Phase::TurnStart, &mut turn, &config.plugins).await;

        // Phase 2: BeforeInference
        emit_phase(Phase::BeforeInference, &mut turn, &config.plugins).await;

        // Build messages
        build_messages(&mut turn, &config.system_prompt);

        // Get data before dropping turn
        let skip = turn.skip_inference;
        let msgs = turn.messages.clone();
        let tools_filter: Vec<String> = turn.tools.iter().map(|td| td.id.clone()).collect();

        (msgs, tools_filter, skip)
    };

    // Skip inference if requested
    if skip_inference {
        return Ok((
            session,
            StreamResult {
                text: String::new(),
                tool_calls: vec![],
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

    // Call LLM
    let response = client
        .exec_chat(&config.model, request, config.chat_options.as_ref())
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

    let result = StreamResult { text, tool_calls };

    // Phase 3: AfterInference (with new context)
    {
        let mut turn = TurnContext::new(&session, tool_descriptors.clone());
        turn.response = Some(result.clone());
        emit_phase(Phase::AfterInference, &mut turn, &config.plugins).await;
    }

    // Add assistant message
    let session = if result.tool_calls.is_empty() {
        session.with_message(assistant_message(&result.text))
    } else {
        session.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()))
    };

    // Phase 6: TurnEnd (with new context)
    {
        let mut turn = TurnContext::new(&session, tool_descriptors);
        emit_phase(Phase::TurnEnd, &mut turn, &config.plugins).await;
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

    // Execute each tool with phase hooks
    let mut executions = Vec::new();

    for call in &result.tool_calls {
        // Create TurnContext for this tool
        let mut turn = TurnContext::new(&session, tool_descriptors.clone());
        turn.tool = Some(ToolContext::new(call));

        // Phase 4: BeforeToolExecute
        emit_phase(Phase::BeforeToolExecute, &mut turn, &config.plugins).await;

        // Check if blocked
        let execution = if turn.tool_blocked() {
            let reason = turn
                .tool
                .as_ref()
                .and_then(|t| t.block_reason.clone())
                .unwrap_or_else(|| "Blocked by plugin".to_string());
            ToolExecution {
                call: call.clone(),
                result: ToolResult::error(&call.name, reason),
                patch: None,
            }
        } else if turn.tool_pending() {
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
        turn.set_tool_result(execution.result.clone());

        // Phase 5: AfterToolExecute
        emit_phase(Phase::AfterToolExecute, &mut turn, &config.plugins).await;

        executions.push((execution, turn.system_reminders.clone()));
    }

    // Collect patches and tool response messages
    let patches = collect_patches(&executions.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>());
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

    // Execute tools
    let executions = if parallel {
        execute_tools_parallel_with_phases(tools, &result.tool_calls, &state, &tool_descriptors, plugins).await
    } else {
        execute_tools_sequential_with_phases(tools, &result.tool_calls, &state, &tool_descriptors, plugins).await
    };

    // Collect patches and tool response messages
    let patches = collect_patches(&executions.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>());
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

/// Execute tools in parallel with phase hooks.
async fn execute_tools_parallel_with_phases(
    tools: &HashMap<String, Arc<dyn Tool>>,
    calls: &[crate::types::ToolCall],
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
) -> Vec<(ToolExecution, Vec<String>)> {
    use futures::future::join_all;

    let futures = calls.iter().map(|call| {
        let tool = tools.get(&call.name).cloned();
        let state = state.clone();
        let call = call.clone();
        let plugins = plugins.to_vec();
        let tool_descriptors = tool_descriptors.to_vec();

        async move {
            execute_single_tool_with_phases(tool.as_deref(), &call, &state, &tool_descriptors, &plugins).await
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
) -> Vec<(ToolExecution, Vec<String>)> {
    use carve_state::apply_patch;

    let mut state = initial_state.clone();
    let mut executions = Vec::with_capacity(calls.len());

    for call in calls {
        let tool = tools.get(&call.name).cloned();
        let (exec, reminders) =
            execute_single_tool_with_phases(tool.as_deref(), call, &state, tool_descriptors, plugins).await;

        // Apply patch to state for next tool
        if let Some(ref patch) = exec.patch {
            if let Ok(new_state) = apply_patch(&state, patch.patch()) {
                state = new_state;
            }
        }

        executions.push((exec, reminders));
    }

    executions
}

/// Execute a single tool with phase hooks.
async fn execute_single_tool_with_phases(
    tool: Option<&dyn Tool>,
    call: &crate::types::ToolCall,
    state: &Value,
    tool_descriptors: &[ToolDescriptor],
    plugins: &[Arc<dyn AgentPlugin>],
) -> (ToolExecution, Vec<String>) {
    // Create a minimal session for TurnContext
    let temp_session = Session::with_initial_state("temp", state.clone());

    // Create TurnContext for this tool
    let mut turn = TurnContext::new(&temp_session, tool_descriptors.to_vec());
    turn.tool = Some(ToolContext::new(call));

    // Phase: BeforeToolExecute
    emit_phase(Phase::BeforeToolExecute, &mut turn, plugins).await;

    // Check if blocked
    let execution = if turn.tool_blocked() {
        let reason = turn
            .tool
            .as_ref()
            .and_then(|t| t.block_reason.clone())
            .unwrap_or_else(|| "Blocked by plugin".to_string());
        ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, reason),
            patch: None,
        }
    } else if turn.tool_pending() {
        ToolExecution {
            call: call.clone(),
            result: ToolResult::pending(&call.name, "Waiting for user confirmation"),
            patch: None,
        }
    } else if tool.is_none() {
        ToolExecution {
            call: call.clone(),
            result: ToolResult::error(&call.name, format!("Tool '{}' not found", call.name)),
            patch: None,
        }
    } else {
        // Execute the tool
        execute_single_tool(tool, call, state).await
    };

    // Set tool result in context
    turn.set_tool_result(execution.result.clone());

    // Phase: AfterToolExecute
    emit_phase(Phase::AfterToolExecute, &mut turn, plugins).await;

    (execution, turn.system_reminders.clone())
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

    // Get tool descriptors for SessionStart
    let tool_descriptors: Vec<ToolDescriptor> =
        tools.values().map(|t| t.descriptor().clone()).collect();

    // Create TurnContext for session lifecycle
    let mut session_turn = TurnContext::new(&session, tool_descriptors.clone());
    init_plugin_data(&mut session_turn, &config.plugins);

    // Phase: SessionStart
    emit_phase(Phase::SessionStart, &mut session_turn, &config.plugins).await;

    loop {
        // Run one turn
        let (new_session, result) = run_turn(client, config, session, tools).await?;
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
    let mut end_turn = TurnContext::new(&session, tool_descriptors);
    emit_phase(Phase::SessionEnd, &mut end_turn, &config.plugins).await;

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

        // Get tool descriptors
        let tool_descriptors: Vec<ToolDescriptor> =
            tools.values().map(|t| t.descriptor().clone()).collect();

        // Phase: SessionStart (use scoped block to manage borrow)
        {
            let mut session_turn = TurnContext::new(&session, tool_descriptors.clone());
            init_plugin_data(&mut session_turn, &config.plugins);
            emit_phase(Phase::SessionStart, &mut session_turn, &config.plugins).await;
        }

        loop {
            // Phase: TurnStart and BeforeInference (collect messages and tools filter)
            let (messages, filtered_tools, skip_inference) = {
                let mut turn = TurnContext::new(&session, tool_descriptors.clone());

                emit_phase(Phase::TurnStart, &mut turn, &config.plugins).await;
                emit_phase(Phase::BeforeInference, &mut turn, &config.plugins).await;

                build_messages(&mut turn, &config.system_prompt);

                let skip = turn.skip_inference;
                let msgs = turn.messages.clone();
                let tools_filter: Vec<String> = turn.tools.iter().map(|td| td.id.clone()).collect();

                (msgs, tools_filter, skip)
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

            // Stream LLM response
            let stream_result = client
                .exec_chat_stream(&config.model, request, config.chat_options.as_ref())
                .await;

            let chat_stream_response = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            };

            // Collect streaming response
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

            // Phase: AfterInference (with new context)
            {
                let mut turn = TurnContext::new(&session, tool_descriptors.clone());
                turn.response = Some(result.clone());
                emit_phase(Phase::AfterInference, &mut turn, &config.plugins).await;
            }

            // Emit turn done
            yield AgentEvent::TurnDone {
                text: result.text.clone(),
                tool_calls: result.tool_calls.clone(),
            };

            // Add assistant message
            session = if result.tool_calls.is_empty() {
                session.with_message(assistant_message(&result.text))
            } else {
                session.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()))
            };

            // Phase: TurnEnd (with new context)
            {
                let mut turn = TurnContext::new(&session, tool_descriptors.clone());
                emit_phase(Phase::TurnEnd, &mut turn, &config.plugins).await;
            }

            // Check if we need to execute tools
            if !result.needs_tools() {
                // Phase: SessionEnd
                let mut end_turn = TurnContext::new(&session, tool_descriptors.clone());
                emit_phase(Phase::SessionEnd, &mut end_turn, &config.plugins).await;

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

            let executions = if config.parallel_tools {
                execute_tools_parallel_with_phases(&tools, &result.tool_calls, &state, &tool_descriptors, &config.plugins).await
            } else {
                execute_tools_sequential_with_phases(&tools, &result.tool_calls, &state, &tool_descriptors, &config.plugins).await
            };

            // Emit tool results and collect patches/messages
            let patches = collect_patches(&executions.iter().map(|(e, _)| e.clone()).collect::<Vec<_>>());
            let mut tool_messages = Vec::new();

            for (exec, reminders) in &executions {
                yield AgentEvent::ToolCallDone {
                    id: exec.call.id.clone(),
                    result: exec.result.clone(),
                    patch: exec.patch.clone(),
                };
                tool_messages.push(tool_response(&exec.call.id, &exec.result));
                for r in reminders {
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

/// Single step execution for fine-grained control.
///
/// This allows callers to control the loop manually.
#[derive(Debug)]
pub enum StepResult {
    /// LLM responded with text, no tools needed.
    Done {
        session: Session,
        response: String,
    },
    /// LLM requested tool calls, tools have been executed.
    ToolsExecuted {
        session: Session,
        text: String,
        tool_calls: Vec<crate::types::ToolCall>,
    },
}

/// Run a single step of the agent loop.
///
/// This gives you fine-grained control over the loop.
pub async fn run_step(
    client: &Client,
    config: &AgentConfig,
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<StepResult, AgentLoopError> {
    // Run one turn
    let (session, result) = run_turn(client, config, session, tools).await?;

    if !result.needs_tools() {
        return Ok(StepResult::Done {
            session,
            response: result.text,
        });
    }

    // Execute tools
    let session = execute_tools_with_config(session, &result, tools, config).await?;

    Ok(StepResult::ToolsExecuted {
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
    use crate::phase::Phase;
    use crate::traits::tool::{ToolDescriptor, ToolError, ToolResult};
    use async_trait::async_trait;
    use carve_state::Context;
    use carve_state_derive::State;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, Value};

    #[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
    struct TestCounterState {
        counter: i64,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("echo", "Echo", "Echo the input")
                .with_parameters(json!({
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
            };
            let tools = HashMap::new();

            let session = execute_tools_simple(session, &result, &tools, true).await.unwrap();
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
            };
            let tools = tool_map([EchoTool]);

            let session = execute_tools_simple(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.messages[0].role, crate::types::Role::Tool);
        });
    }

    struct CounterTool;

    #[async_trait]
    impl Tool for CounterTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("counter", "Counter", "Increment a counter")
                .with_parameters(json!({
                    "type": "object",
                    "properties": {
                        "amount": { "type": "integer" }
                    }
                }))
        }

        async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let amount = args["amount"].as_i64().unwrap_or(1);

            let state = ctx.state::<TestCounterState>("");
            let current = state.counter().unwrap_or(0);
            let new_value = current + amount;

            state.set_counter(new_value);

            Ok(ToolResult::success("counter", json!({ "new_value": new_value })))
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
            };
            let tools = tool_map([CounterTool]);

            let session = execute_tools_simple(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            assert_eq!(session.patch_count(), 1);

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }

    #[test]
    fn test_step_result_variants() {
        let session = Session::new("test");

        let done = StepResult::Done {
            session: session.clone(),
            response: "Hello".to_string(),
        };

        let tools_executed = StepResult::ToolsExecuted {
            session,
            text: "Calling tools".to_string(),
            tool_calls: vec![],
        };

        match done {
            StepResult::Done { response, .. } => assert_eq!(response, "Hello"),
            _ => panic!("Expected Done"),
        }

        match tools_executed {
            StepResult::ToolsExecuted { text, .. } => assert_eq!(text, "Calling tools"),
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
            Err(ToolError::ExecutionFailed("Intentional failure".to_string()))
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
            };
            let tools = tool_map([FailingTool]);

            let session = execute_tools_simple(session, &result, &tools, true).await.unwrap();

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

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            match phase {
                Phase::TurnStart => {
                    turn.system("Test system context");
                }
                Phase::BeforeInference => {
                    turn.session("Test session context");
                }
                Phase::AfterToolExecute => {
                    if turn.tool_id() == Some("echo") {
                        turn.reminder("Check the echo result");
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

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase == Phase::BeforeToolExecute {
                if turn.tool_id() == Some("echo") {
                    turn.block("Echo tool is blocked");
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
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(BlockingPhasePlugin)];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("blocked") || msg.content.contains("Error"),
                "Expected blocked/error in message, got: {}", msg.content
            );
        });
    }

    struct ReminderPhasePlugin;

    #[async_trait]
    impl AgentPlugin for ReminderPhasePlugin {
        fn id(&self) -> &str {
            "reminder"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase == Phase::AfterToolExecute {
                turn.reminder("Tool execution completed");
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
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ReminderPhasePlugin)];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            // Should have tool response + reminder message
            assert_eq!(session.message_count(), 2);
            assert!(session.messages[1].content.contains("system-reminder"));
            assert!(session.messages[1].content.contains("Tool execution completed"));
        });
    }

    #[test]
    fn test_build_messages_with_context() {
        let session = Session::new("test").with_message(Message::user("Hello"));
        let tool_descriptors = vec![ToolDescriptor::new("test", "Test", "Test tool")];
        let mut turn = TurnContext::new(&session, tool_descriptors);

        turn.system("System context 1");
        turn.system("System context 2");
        turn.session("Session context");

        build_messages(&mut turn, "Base system prompt");

        assert_eq!(turn.messages.len(), 3);
        // First message should be system prompt + context
        assert!(turn.messages[0].content.contains("Base system prompt"));
        assert!(turn.messages[0].content.contains("System context 1"));
        assert!(turn.messages[0].content.contains("System context 2"));
        // Second message should be session context
        assert_eq!(turn.messages[1].content, "Session context");
        // Third message should be user message
        assert_eq!(turn.messages[2].content, "Hello");
    }

    #[test]
    fn test_build_messages_empty_system() {
        let session = Session::new("test").with_message(Message::user("Hello"));
        let mut turn = TurnContext::new(&session, vec![]);

        build_messages(&mut turn, "");

        // Only user message
        assert_eq!(turn.messages.len(), 1);
        assert_eq!(turn.messages[0].content, "Hello");
    }

    struct ToolFilterPlugin;

    #[async_trait]
    impl AgentPlugin for ToolFilterPlugin {
        fn id(&self) -> &str {
            "filter"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase == Phase::BeforeInference {
                turn.exclude("dangerous_tool");
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
            let mut turn = TurnContext::new(&session, tool_descriptors);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(ToolFilterPlugin)];

            emit_phase(Phase::BeforeInference, &mut turn, &plugins).await;

            assert_eq!(turn.tools.len(), 1);
            assert_eq!(turn.tools[0].id, "safe_tool");
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

            async fn on_phase(&self, _phase: Phase, _turn: &mut TurnContext<'_>) {}

            fn initial_data(&self) -> Option<(&'static str, Value)> {
                Some(("plugin_config", json!({"enabled": true})))
            }
        }

        let session = Session::new("test");
        let mut turn = TurnContext::new(&session, vec![]);
        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(DataPlugin)];

        init_plugin_data(&mut turn, &plugins);

        let config: Option<serde_json::Map<String, Value>> = turn.get("plugin_config");
        assert!(config.is_some());
        assert_eq!(config.unwrap()["enabled"], true);
    }

    // ============================================================================
    // Additional Coverage Tests
    // ============================================================================

    #[test]
    fn test_agent_config_debug() {
        let config = AgentConfig::new("gpt-4")
            .with_system_prompt("You are helpful.");

        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("AgentConfig"));
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
            async fn on_phase(&self, _phase: Phase, _turn: &mut TurnContext<'_>) {}
        }

        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![
            Arc::new(DummyPlugin),
            Arc::new(DummyPlugin),
        ];
        let config = AgentConfig::new("gpt-4").with_plugins(plugins);
        assert_eq!(config.plugins.len(), 2);
    }

    struct PendingPhasePlugin;

    #[async_trait]
    impl AgentPlugin for PendingPhasePlugin {
        fn id(&self) -> &str {
            "pending"
        }

        async fn on_phase(&self, phase: Phase, turn: &mut TurnContext<'_>) {
            if phase == Phase::BeforeToolExecute {
                if turn.tool_id() == Some("echo") {
                    use crate::state_types::{Interaction, InteractionType};
                    turn.pending(Interaction {
                        id: "confirm_1".to_string(),
                        interaction_type: InteractionType::Confirm,
                        message: "Execute echo?".to_string(),
                        choices: vec![],
                        metadata: Default::default(),
                    });
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
            };
            let tools = tool_map([EchoTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(PendingPhasePlugin)];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            // Pending tool should return pending status
            assert!(
                msg.content.contains("pending") || msg.content.contains("Pending") || msg.content.contains("Waiting"),
                "Expected pending in message, got: {}", msg.content
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
            };
            let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new(); // Empty tools

            let session = execute_tools_simple(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(
                msg.content.contains("not found") || msg.content.contains("Error"),
                "Expected 'not found' error in message, got: {}", msg.content
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
                msg.content.contains("pending") || msg.content.contains("Pending") || msg.content.contains("Waiting"),
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
