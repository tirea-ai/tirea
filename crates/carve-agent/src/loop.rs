//! Agent loop implementation.
//!
//! The agent loop orchestrates the conversation between user, LLM, and tools:
//!
//! ```text
//! User Input → LLM → Tool Calls? → Execute Tools → LLM → ... → Final Response
//! ```

use crate::convert::{assistant_message, assistant_tool_calls, build_request, tool_response};
use crate::execute::{
    collect_patches, execute_tools_parallel, execute_tools_parallel_with_plugins,
    execute_tools_sequential_with_plugins,
};
use crate::plugin::AgentPlugin;
use crate::session::Session;
use crate::stream::{AgentEvent, StreamCollector, StreamResult};
use crate::traits::tool::Tool;
use crate::types::Message;
use async_stream::stream;
use futures::{Stream, StreamExt};
use genai::chat::ChatOptions;
use genai::Client;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// Configuration for the agent loop.
#[derive(Clone)]
pub struct AgentConfig {
    /// Model identifier (e.g., "gpt-4", "claude-3-opus").
    pub model: String,
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

/// Run one turn of the agent loop (non-streaming).
///
/// A turn consists of:
/// 1. Send messages to LLM
/// 2. If LLM returns tool calls, execute them
/// 3. Return the result
///
/// This function does NOT loop - it's a single LLM call + optional tool execution.
/// Use `run_loop` for the full agent loop.
pub async fn run_turn(
    client: &Client,
    config: &AgentConfig,
    session: Session,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<(Session, StreamResult), AgentLoopError> {
    // Build request
    let tool_refs: Vec<&dyn Tool> = tools.values().map(|t| t.as_ref()).collect();
    let request = build_request(&session.messages, &tool_refs);

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

    // Add assistant message
    let session = if result.tool_calls.is_empty() {
        session.with_message(assistant_message(&result.text))
    } else {
        session.with_message(assistant_tool_calls(&result.text, result.tool_calls.clone()))
    };

    Ok((session, result))
}

/// Execute tool calls and update session.
pub async fn execute_tools(
    session: Session,
    result: &StreamResult,
    tools: &HashMap<String, Arc<dyn Tool>>,
    parallel: bool,
) -> Result<Session, AgentLoopError> {
    execute_tools_with_plugins(session, result, tools, parallel, &[]).await
}

/// Execute tool calls with plugin hooks and update session.
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

    // Execute tools with or without plugins
    let executions = if plugins.is_empty() {
        if parallel {
            execute_tools_parallel(tools, &result.tool_calls, &state).await
        } else {
            let (_, execs) =
                crate::execute::execute_tools_sequential(tools, &result.tool_calls, &state).await;
            execs
        }
    } else if parallel {
        execute_tools_parallel_with_plugins(tools, &result.tool_calls, &state, plugins).await
    } else {
        let (_, execs) =
            execute_tools_sequential_with_plugins(tools, &result.tool_calls, &state, plugins).await;
        execs
    };

    // Collect patches and tool response messages
    let patches = collect_patches(&executions);
    let tool_messages: Vec<Message> = executions
        .iter()
        .map(|e| tool_response(&e.call.id, &e.result))
        .collect();

    // Update session
    let session = session.with_patches(patches).with_messages(tool_messages);

    Ok(session)
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

        // Execute tools with plugins
        session = execute_tools_with_plugins(
            session,
            &result,
            tools,
            config.parallel_tools,
            &config.plugins,
        )
        .await?;

        rounds += 1;
        if rounds >= config.max_rounds {
            return Err(AgentLoopError::MaxRoundsExceeded(config.max_rounds));
        }
    }

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

        loop {
            // Build request
            let tool_refs: Vec<&dyn Tool> = tools.values().map(|t| t.as_ref()).collect();
            let request = build_request(&session.messages, &tool_refs);

            // Stream LLM response
            let stream_result = client
                .exec_chat_stream(&config.model, request, config.chat_options.as_ref())
                .await;

            let chat_stream_response = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error(e.to_string());
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
                                    yield AgentEvent::TextDelta(delta);
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
                        yield AgentEvent::Error(e.to_string());
                        return;
                    }
                }
            }

            let result = collector.finish();

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

            // Check if we need to execute tools
            if !result.needs_tools() {
                yield AgentEvent::Done { response: result.text };
                return;
            }

            // Execute tools with plugins
            let state = match session.rebuild_state() {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error(e.to_string());
                    return;
                }
            };

            let executions = if config.plugins.is_empty() {
                if config.parallel_tools {
                    execute_tools_parallel(&tools, &result.tool_calls, &state).await
                } else {
                    let (_, execs) = crate::execute::execute_tools_sequential(&tools, &result.tool_calls, &state).await;
                    execs
                }
            } else if config.parallel_tools {
                execute_tools_parallel_with_plugins(&tools, &result.tool_calls, &state, &config.plugins).await
            } else {
                let (_, execs) = execute_tools_sequential_with_plugins(&tools, &result.tool_calls, &state, &config.plugins).await;
                execs
            };

            // Emit tool results and collect patches/messages
            let patches = collect_patches(&executions);
            let mut tool_messages = Vec::new();

            for exec in &executions {
                yield AgentEvent::ToolCallDone {
                    id: exec.call.id.clone(),
                    result: exec.result.clone(),
                    patch: exec.patch.clone(),
                };
                tool_messages.push(tool_response(&exec.call.id, &exec.result));
            }

            session = session.with_patches(patches).with_messages(tool_messages);

            rounds += 1;
            if rounds >= config.max_rounds {
                yield AgentEvent::Error(format!("Max rounds ({}) exceeded", config.max_rounds));
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

    // Execute tools with plugins
    let session = execute_tools_with_plugins(
        session,
        &result,
        tools,
        config.parallel_tools,
        &config.plugins,
    )
    .await?;

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
    }

    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("gpt-4")
            .with_max_rounds(5)
            .with_parallel_tools(false);

        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_rounds, 5);
        assert!(!config.parallel_tools);
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

    // Integration tests require actual LLM access, so we test the pure parts here
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

            let session = execute_tools(session, &result, &tools, true).await.unwrap();
            assert_eq!(session.message_count(), 0); // No tool messages added
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

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should have tool response message
            assert_eq!(session.message_count(), 1);
            assert_eq!(session.messages[0].role, crate::types::Role::Tool);
        });
    }

    // Tool that modifies state
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

            // Access typed state
            let state = ctx.state::<TestCounterState>("");
            let current = state.counter().unwrap_or(0);
            let new_value = current + amount;

            // Write new value
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

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should have tool response message
            assert_eq!(session.message_count(), 1);

            // Should have patch
            assert_eq!(session.patch_count(), 1);

            // Rebuild state should show new value
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }

    #[test]
    fn test_execute_tools_multiple() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "Multiple tools".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "hello"})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 3})),
                    crate::types::ToolCall::new("call_3", "echo", json!({"message": "world"})),
                ],
            };

            let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool));
            tools.insert("counter".to_string(), Arc::new(CounterTool));

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should have 3 tool response messages
            assert_eq!(session.message_count(), 3);

            // Should have 1 patch (only counter modifies state)
            assert_eq!(session.patch_count(), 1);
        });
    }

    #[test]
    fn test_execute_tools_sequential() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "Sequential".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "counter", json!({"amount": 1})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 2})),
                ],
            };

            let tools = tool_map([CounterTool]);

            // Sequential execution - each tool sees previous state
            let session = execute_tools(session, &result, &tools, false).await.unwrap();

            assert_eq!(session.message_count(), 2);
            // Both tools produce patches
            assert_eq!(session.patch_count(), 2);
        });
    }

    #[test]
    fn test_execute_tools_parallel() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "Parallel".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "counter", json!({"amount": 1})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 2})),
                ],
            };

            let tools = tool_map([CounterTool]);

            // Parallel execution - both tools see same initial state
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 2);
            // Both tools produce patches (but they both see counter=0)
            assert_eq!(session.patch_count(), 2);
        });
    }

    #[test]
    fn test_execute_tools_not_found() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Unknown tool".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "nonexistent",
                    json!({}),
                )],
            };
            let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should still have tool response message (with error)
            assert_eq!(session.message_count(), 1);

            // Response should contain error info
            let msg = &session.messages[0];
            assert!(msg.content.contains("not found") || msg.content.contains("error"));
        });
    }

    #[test]
    fn test_session_with_messages_and_patches() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Simulate a conversation with tool usage
            let session = Session::with_initial_state("test", json!({"counter": 0}))
                .with_message(crate::types::Message::user("Increment counter by 5"));

            // Simulate LLM response with tool call
            let result = StreamResult {
                text: "I'll increment the counter".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "counter",
                    json!({"amount": 5}),
                )],
            };

            // Add assistant message with tool calls
            let session = session.with_message(crate::convert::assistant_tool_calls(
                &result.text,
                result.tool_calls.clone(),
            ));

            let tools = tool_map([CounterTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should have: user message, assistant message, tool response
            assert_eq!(session.message_count(), 3);

            // Should have patch from counter tool
            assert_eq!(session.patch_count(), 1);

            // State should be updated
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }

    #[test]
    fn test_agent_loop_error_state_error() {
        let err = AgentLoopError::StateError("invalid patch".to_string());
        assert!(err.to_string().contains("State error"));
        assert!(err.to_string().contains("invalid patch"));
    }

    #[test]
    fn test_step_result_variants() {
        // Test that StepResult can be constructed
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

        // Just verify they can be constructed (pattern matching)
        match done {
            StepResult::Done { response, .. } => assert_eq!(response, "Hello"),
            _ => panic!("Expected Done"),
        }

        match tools_executed {
            StepResult::ToolsExecuted { text, .. } => assert_eq!(text, "Calling tools"),
            _ => panic!("Expected ToolsExecuted"),
        }
    }

    // Tool that always fails
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

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should have tool response (with error)
            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            assert!(msg.content.contains("error") || msg.content.contains("fail"));
        });
    }

    #[test]
    fn test_execute_tools_mixed_success_and_failure() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Mixed tools".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "hello"})),
                    crate::types::ToolCall::new("call_2", "failing", json!({})),
                    crate::types::ToolCall::new("call_3", "echo", json!({"message": "world"})),
                ],
            };

            let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool));
            tools.insert("failing".to_string(), Arc::new(FailingTool));

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // All 3 tools should have responses
            assert_eq!(session.message_count(), 3);
        });
    }

    #[test]
    fn test_session_multiple_tool_rounds() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Simulate multiple rounds of tool execution
            let mut session = Session::with_initial_state("test", json!({"counter": 0}));

            // Round 1
            let result1 = StreamResult {
                text: "Round 1".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "counter",
                    json!({"amount": 5}),
                )],
            };
            session = session.with_message(crate::convert::assistant_tool_calls(
                &result1.text,
                result1.tool_calls.clone(),
            ));
            let tools = tool_map([CounterTool]);
            session = execute_tools(session, &result1, &tools, true).await.unwrap();

            // Round 2
            let result2 = StreamResult {
                text: "Round 2".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_2",
                    "counter",
                    json!({"amount": 3}),
                )],
            };
            session = session.with_message(crate::convert::assistant_tool_calls(
                &result2.text,
                result2.tool_calls.clone(),
            ));
            session = execute_tools(session, &result2, &tools, true).await.unwrap();

            // Should have 4 messages: 2 assistant + 2 tool responses
            assert_eq!(session.message_count(), 4);

            // Should have 2 patches
            assert_eq!(session.patch_count(), 2);

            // Final state should be 5 + 3 = 8
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 8);
        });
    }

    #[test]
    fn test_session_snapshot_after_tools() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));

            let result = StreamResult {
                text: "Incrementing".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "counter", json!({"amount": 10})),
                ],
            };
            let tools = tool_map([CounterTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();
            assert_eq!(session.patch_count(), 1);

            // Snapshot should collapse patches
            let session = session.snapshot().unwrap();
            assert_eq!(session.patch_count(), 0);
            assert_eq!(session.state["counter"], 10);
        });
    }

    #[test]
    fn test_agent_config_with_chat_options() {
        use genai::chat::ChatOptions;

        let options = ChatOptions::default();
        let config = AgentConfig::new("gpt-4").with_chat_options(options);

        assert!(config.chat_options.is_some());
    }

    #[test]
    fn test_tool_map_empty() {
        let tools: HashMap<String, Arc<dyn Tool>> = tool_map(std::iter::empty::<EchoTool>());
        assert!(tools.is_empty());
    }

    #[test]
    fn test_tool_map_multiple_same_name() {
        // If multiple tools have the same name, later ones overwrite
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        assert!(tools.is_empty());

        // tool_map creates from descriptor id
        let mut tools = tool_map([EchoTool]);
        tools.insert("echo".to_string(), Arc::new(EchoTool));
        assert_eq!(tools.len(), 1); // Still 1, not 2
    }

    // Tool with complex state changes
    struct MultiFieldTool;

    #[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
    struct MultiFieldState {
        field_a: i64,
        field_b: String,
        field_c: bool,
    }

    #[async_trait]
    impl Tool for MultiFieldTool {
        fn descriptor(&self) -> ToolDescriptor {
            ToolDescriptor::new("multi", "Multi Field", "Updates multiple fields")
        }

        async fn execute(&self, args: Value, ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
            let state = ctx.state::<MultiFieldState>("");

            if let Some(a) = args.get("a").and_then(|v| v.as_i64()) {
                state.set_field_a(a);
            }
            if let Some(b) = args.get("b").and_then(|v| v.as_str()) {
                state.set_field_b(b.to_string());
            }
            if let Some(c) = args.get("c").and_then(|v| v.as_bool()) {
                state.set_field_c(c);
            }

            Ok(ToolResult::success("multi", json!({"updated": true})))
        }
    }

    #[test]
    fn test_execute_tools_with_complex_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state(
                "test",
                json!({"field_a": 0, "field_b": "", "field_c": false}),
            );
            let result = StreamResult {
                text: "Updating".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "multi",
                    json!({"a": 42, "b": "hello", "c": true}),
                )],
            };
            let tools = tool_map([MultiFieldTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["field_a"], 42);
            assert_eq!(state["field_b"], "hello");
            assert_eq!(state["field_c"], true);
        });
    }

    #[test]
    fn test_execute_tools_partial_state_update() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state(
                "test",
                json!({"field_a": 100, "field_b": "original", "field_c": true}),
            );
            let result = StreamResult {
                text: "Partial update".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "multi",
                    json!({"a": 200}), // Only update field_a
                )],
            };
            let tools = tool_map([MultiFieldTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["field_a"], 200);
            assert_eq!(state["field_b"], "original"); // Unchanged
            assert_eq!(state["field_c"], true); // Unchanged
        });
    }

    #[test]
    fn test_stream_result_methods() {
        let result = StreamResult {
            text: "Hello world".to_string(),
            tool_calls: vec![],
        };
        assert!(!result.needs_tools());
        assert_eq!(result.text, "Hello world");

        let result_with_tools = StreamResult {
            text: "Calling tools".to_string(),
            tool_calls: vec![
                crate::types::ToolCall::new("1", "tool1", json!({})),
                crate::types::ToolCall::new("2", "tool2", json!({})),
            ],
        };
        assert!(result_with_tools.needs_tools());
        assert_eq!(result_with_tools.tool_calls.len(), 2);
    }

    #[test]
    fn test_execute_tools_preserves_message_order() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test")
                .with_message(crate::types::Message::user("First"))
                .with_message(crate::types::Message::assistant("Second"));

            let result = StreamResult {
                text: "Tools".to_string(),
                tool_calls: vec![crate::types::ToolCall::new("call_1", "echo", json!({}))],
            };
            let tools = tool_map([EchoTool]);

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Messages should be in order
            assert_eq!(session.messages.len(), 3);
            assert_eq!(session.messages[0].role, crate::types::Role::User);
            assert_eq!(session.messages[1].role, crate::types::Role::Assistant);
            assert_eq!(session.messages[2].role, crate::types::Role::Tool);
        });
    }

    // ============================================================================
    // Error Recovery Tests
    // ============================================================================

    #[test]
    fn test_partial_tool_failure_continues() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // First tool succeeds, second fails, third succeeds
            let session = Session::new("test");
            let result = StreamResult {
                text: "Running tools".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "first"})),
                    crate::types::ToolCall::new("call_2", "failing", json!({})),
                    crate::types::ToolCall::new("call_3", "echo", json!({"message": "third"})),
                ],
            };

            let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
            tools.insert("echo".to_string(), Arc::new(EchoTool));
            tools.insert("failing".to_string(), Arc::new(FailingTool));

            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // All tools should have responses, even the failed one
            assert_eq!(session.message_count(), 3);
        });
    }

    #[test]
    fn test_all_tools_fail_still_returns_session() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "All fail".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "failing", json!({})),
                    crate::types::ToolCall::new("call_2", "failing", json!({})),
                ],
            };

            let tools = tool_map([FailingTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should still have error responses
            assert_eq!(session.message_count(), 2);
        });
    }

    #[test]
    fn test_session_rebuild_with_invalid_patch() {
        // Test that rebuild handles edge cases
        let session = Session::with_initial_state("test", json!({"value": 1}));

        // Empty patches should work fine
        let state = session.rebuild_state().unwrap();
        assert_eq!(state["value"], 1);
    }

    #[test]
    fn test_empty_tool_calls_list() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "No tools".to_string(),
                tool_calls: vec![],
            };

            let tools = tool_map([EchoTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // No messages added
            assert_eq!(session.message_count(), 0);
        });
    }

    // ============================================================================
    // Concurrency and Edge Case Tests
    // ============================================================================

    #[test]
    fn test_many_parallel_tool_executions() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));

            // Create many tool calls
            let tool_calls: Vec<crate::types::ToolCall> = (0..20)
                .map(|i| {
                    crate::types::ToolCall::new(
                        format!("call_{}", i),
                        "echo",
                        json!({"message": format!("msg_{}", i)}),
                    )
                })
                .collect();

            let result = StreamResult {
                text: "Many tools".to_string(),
                tool_calls,
            };

            let tools = tool_map([EchoTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 20);
        });
    }

    #[test]
    fn test_large_session_state() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Create a large state
            let mut items: Vec<serde_json::Value> = Vec::new();
            for i in 0..1000 {
                items.push(json!({
                    "id": i,
                    "name": format!("Item {}", i),
                    "data": "x".repeat(100)
                }));
            }

            let session = Session::with_initial_state("test", json!({"items": items}));

            // Should handle large state
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["items"].as_array().unwrap().len(), 1000);
        });
    }

    #[test]
    fn test_many_patches_accumulation() {
        use carve_state::{path, Op, Patch, TrackedPatch};

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut session = Session::with_initial_state("test", json!({"counter": 0}));

            // Add many patches
            for i in 0..50 {
                session = session.with_patch(TrackedPatch::new(
                    Patch::new().with_op(Op::set(path!("counter"), json!(i))),
                ));
            }

            assert_eq!(session.patch_count(), 50);
            assert!(session.needs_snapshot(30));

            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 49); // Last patch value
        });
    }

    #[test]
    fn test_tool_with_null_arguments() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Null args".to_string(),
                tool_calls: vec![crate::types::ToolCall::new("call_1", "echo", json!(null))],
            };

            let tools = tool_map([EchoTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Should handle null arguments
            assert_eq!(session.message_count(), 1);
        });
    }

    #[test]
    fn test_tool_with_empty_object_arguments() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Empty args".to_string(),
                tool_calls: vec![crate::types::ToolCall::new("call_1", "echo", json!({}))],
            };

            let tools = tool_map([EchoTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            assert_eq!(session.message_count(), 1);
        });
    }

    // ============================================================================
    // Mock LLM Tests (testing without actual LLM)
    // ============================================================================

    #[test]
    fn test_simulate_conversation_flow() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Simulate: User asks -> LLM calls tool -> Tool responds -> LLM answers
            let mut session = Session::with_initial_state("conv", json!({"counter": 0}))
                .with_message(crate::types::Message::user("Increment counter by 5"));

            // Simulate LLM response with tool call
            let llm_response = StreamResult {
                text: "I'll increment the counter for you.".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_inc",
                    "counter",
                    json!({"amount": 5}),
                )],
            };

            // Add assistant message
            session = session.with_message(crate::convert::assistant_tool_calls(
                &llm_response.text,
                llm_response.tool_calls.clone(),
            ));

            // Execute tools
            let tools = tool_map([CounterTool]);
            session = execute_tools(session, &llm_response, &tools, true)
                .await
                .unwrap();

            // Simulate final LLM response
            session = session.with_message(crate::types::Message::assistant(
                "Done! The counter is now 5.",
            ));

            // Verify conversation
            assert_eq!(session.message_count(), 4);
            assert_eq!(session.messages[0].role, crate::types::Role::User);
            assert_eq!(session.messages[1].role, crate::types::Role::Assistant);
            assert_eq!(session.messages[2].role, crate::types::Role::Tool);
            assert_eq!(session.messages[3].role, crate::types::Role::Assistant);

            // Verify state
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }

    #[test]
    fn test_simulate_multi_turn_with_tools() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut session = Session::with_initial_state("multi", json!({"counter": 0}));
            let tools = tool_map([CounterTool]);

            // Turn 1
            session = session.with_message(crate::types::Message::user("Add 3"));
            let result1 = StreamResult {
                text: "Adding 3".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "c1",
                    "counter",
                    json!({"amount": 3}),
                )],
            };
            session = session.with_message(crate::convert::assistant_tool_calls(
                &result1.text,
                result1.tool_calls.clone(),
            ));
            session = execute_tools(session, &result1, &tools, true).await.unwrap();
            session = session.with_message(crate::types::Message::assistant("Counter is 3"));

            // Turn 2
            session = session.with_message(crate::types::Message::user("Add 7 more"));
            let result2 = StreamResult {
                text: "Adding 7".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "c2",
                    "counter",
                    json!({"amount": 7}),
                )],
            };
            session = session.with_message(crate::convert::assistant_tool_calls(
                &result2.text,
                result2.tool_calls.clone(),
            ));
            session = execute_tools(session, &result2, &tools, true).await.unwrap();
            session = session.with_message(crate::types::Message::assistant("Counter is now 10"));

            // Verify
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 10);
            assert_eq!(session.message_count(), 8); // 2 turns * (user + assistant_tool + tool + assistant)
        });
    }

    #[test]
    fn test_simulate_no_tool_needed() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut session = Session::new("no_tool");

            // User asks simple question
            session = session.with_message(crate::types::Message::user("What is 2+2?"));

            // LLM responds without tools
            let result = StreamResult {
                text: "2+2 equals 4.".to_string(),
                tool_calls: vec![],
            };

            assert!(!result.needs_tools());

            session = session.with_message(crate::types::Message::assistant(&result.text));

            assert_eq!(session.message_count(), 2);
        });
    }

    #[test]
    fn test_simulate_tool_chain() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Simulate: search -> fetch -> format
            let session = Session::new("chain")
                .with_message(crate::types::Message::user("Get weather for Tokyo"));

            // LLM calls multiple tools in sequence (simulated as parallel for this test)
            let result = StreamResult {
                text: "Let me get that info.".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("c1", "echo", json!({"message": "search:Tokyo"})),
                    crate::types::ToolCall::new("c2", "echo", json!({"message": "fetch:weather"})),
                    crate::types::ToolCall::new("c3", "echo", json!({"message": "format:result"})),
                ],
            };

            let session = session.with_message(crate::convert::assistant_tool_calls(
                &result.text,
                result.tool_calls.clone(),
            ));

            let tools = tool_map([EchoTool]);
            let session = execute_tools(session, &result, &tools, true).await.unwrap();

            // Session flow:
            // 1. user message (from Session::new().with_message)
            // 2. assistant message with tool calls (added above)
            // 3-5. 3 tool responses (added by execute_tools)
            // Total: 5 messages
            assert_eq!(session.messages.len(), 5);
        });
    }

    #[test]
    fn test_step_result_done_variant() {
        let session = Session::new("test");
        let step = StepResult::Done {
            session: session.clone(),
            response: "Final answer".to_string(),
        };

        match step {
            StepResult::Done { response, session: s } => {
                assert_eq!(response, "Final answer");
                assert_eq!(s.id, "test");
            }
            _ => panic!("Expected Done"),
        }
    }

    #[test]
    fn test_step_result_tools_executed_variant() {
        let session = Session::new("test");
        let step = StepResult::ToolsExecuted {
            session: session.clone(),
            text: "Calling tools".to_string(),
            tool_calls: vec![crate::types::ToolCall::new("c1", "test", json!({}))],
        };

        match step {
            StepResult::ToolsExecuted {
                text,
                tool_calls,
                session: s,
            } => {
                assert_eq!(text, "Calling tools");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(s.id, "test");
            }
            _ => panic!("Expected ToolsExecuted"),
        }
    }

    // ============================================================================
    // Plugin Integration Tests
    // ============================================================================

    #[test]
    fn test_agent_config_with_plugins() {
        use crate::plugin::AgentPlugin;

        struct TestPlugin;

        #[async_trait]
        impl AgentPlugin for TestPlugin {
            fn id(&self) -> &str {
                "test_plugin"
            }
        }

        let plugin: Arc<dyn AgentPlugin> = Arc::new(TestPlugin);
        let config = AgentConfig::new("gpt-4").with_plugin(plugin);

        assert!(config.has_plugins());
        assert_eq!(config.plugins.len(), 1);
    }

    #[test]
    fn test_agent_config_with_multiple_plugins() {
        use crate::plugin::AgentPlugin;

        struct Plugin1;
        struct Plugin2;

        #[async_trait]
        impl AgentPlugin for Plugin1 {
            fn id(&self) -> &str {
                "plugin1"
            }
        }

        #[async_trait]
        impl AgentPlugin for Plugin2 {
            fn id(&self) -> &str {
                "plugin2"
            }
        }

        let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(Plugin1), Arc::new(Plugin2)];
        let config = AgentConfig::new("gpt-4").with_plugins(plugins);

        assert!(config.has_plugins());
        assert_eq!(config.plugins.len(), 2);
    }

    #[test]
    fn test_agent_config_no_plugins() {
        let config = AgentConfig::default();
        assert!(!config.has_plugins());
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn test_agent_config_debug() {
        let config = AgentConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("AgentConfig"));
        assert!(debug_str.contains("[0 plugins]"));
    }

    #[test]
    fn test_execute_tools_with_plugins_empty_plugins() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Hello".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "echo",
                    json!({"message": "test"}),
                )],
            };
            let tools = tool_map([EchoTool]);

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &[]).await.unwrap();
            assert_eq!(session.message_count(), 1);
        });
    }

    #[test]
    fn test_execute_tools_with_plugins_sequential() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "Sequential".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "counter", json!({"amount": 1})),
                    crate::types::ToolCall::new("call_2", "counter", json!({"amount": 2})),
                ],
            };
            let tools = tool_map([CounterTool]);

            // Sequential with no plugins
            let session =
                execute_tools_with_plugins(session, &result, &tools, false, &[]).await.unwrap();
            assert_eq!(session.message_count(), 2);
            assert_eq!(session.patch_count(), 2);
        });
    }

    #[test]
    fn test_execute_tools_with_blocking_plugin() {
        use crate::plugin::AgentPlugin;
        use crate::plugins::ExecutionContextExt;

        struct BlockingPlugin;

        #[async_trait]
        impl AgentPlugin for BlockingPlugin {
            fn id(&self) -> &str {
                "blocker"
            }

            async fn before_tool_execute(
                &self,
                ctx: &Context<'_>,
                tool_id: &str,
                _args: &Value,
            ) {
                if tool_id == "echo" {
                    ctx.block(format!("Tool {} is blocked", tool_id));
                }
            }
        }

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
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(BlockingPlugin)];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            // Tool was blocked, so result should contain error status or blocked message
            assert_eq!(session.message_count(), 1);
            let msg = &session.messages[0];
            // The message is JSON serialized, so it may contain "Error" status or "blocked" message
            assert!(
                msg.content.contains("blocked")
                || msg.content.contains("Error")
                || msg.content.contains("error"),
                "Expected blocked/error in message, got: {}", msg.content
            );
        });
    }

    #[test]
    fn test_execute_tools_with_logging_plugin() {
        use crate::plugin::AgentPlugin;
        use crate::plugins::ReminderContextExt;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct LoggingPlugin {
            before_count: AtomicUsize,
            after_count: AtomicUsize,
        }

        impl LoggingPlugin {
            fn new() -> Self {
                Self {
                    before_count: AtomicUsize::new(0),
                    after_count: AtomicUsize::new(0),
                }
            }
        }

        #[async_trait]
        impl AgentPlugin for LoggingPlugin {
            fn id(&self) -> &str {
                "logger"
            }

            async fn before_tool_execute(
                &self,
                ctx: &Context<'_>,
                tool_id: &str,
                _args: &Value,
            ) {
                self.before_count.fetch_add(1, Ordering::SeqCst);
                ctx.add_reminder(format!("Before: {}", tool_id));
            }

            async fn after_tool_execute(
                &self,
                ctx: &Context<'_>,
                tool_id: &str,
                _result: &crate::traits::tool::ToolResult,
            ) {
                self.after_count.fetch_add(1, Ordering::SeqCst);
                ctx.add_reminder(format!("After: {}", tool_id));
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::new("test");
            let result = StreamResult {
                text: "Logging".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "a"})),
                    crate::types::ToolCall::new("call_2", "echo", json!({"message": "b"})),
                ],
            };
            let tools = tool_map([EchoTool]);
            let plugin = Arc::new(LoggingPlugin::new());
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![plugin.clone()];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            // Both tools should have been executed
            assert_eq!(session.message_count(), 2);
            // Plugin hooks should have been called
            assert_eq!(plugin.before_count.load(Ordering::SeqCst), 2);
            assert_eq!(plugin.after_count.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn test_execute_tools_with_plugins_parallel_vs_sequential() {
        use crate::plugin::AgentPlugin;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CounterPlugin {
            call_count: AtomicUsize,
        }

        impl CounterPlugin {
            fn new() -> Self {
                Self {
                    call_count: AtomicUsize::new(0),
                }
            }
        }

        #[async_trait]
        impl AgentPlugin for CounterPlugin {
            fn id(&self) -> &str {
                "counter_plugin"
            }

            async fn before_tool_execute(&self, _ctx: &Context<'_>, _tool_id: &str, _args: &Value) {
                self.call_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Test parallel
            let session = Session::new("test_parallel");
            let result = StreamResult {
                text: "Parallel".to_string(),
                tool_calls: vec![
                    crate::types::ToolCall::new("call_1", "echo", json!({"message": "a"})),
                    crate::types::ToolCall::new("call_2", "echo", json!({"message": "b"})),
                    crate::types::ToolCall::new("call_3", "echo", json!({"message": "c"})),
                ],
            };
            let tools = tool_map([EchoTool]);
            let plugin = Arc::new(CounterPlugin::new());
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![plugin.clone()];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();
            assert_eq!(session.message_count(), 3);
            assert_eq!(plugin.call_count.load(Ordering::SeqCst), 3);

            // Test sequential
            let session2 = Session::new("test_sequential");
            let plugin2 = Arc::new(CounterPlugin::new());
            let plugins2: Vec<Arc<dyn AgentPlugin>> = vec![plugin2.clone()];

            let session2 =
                execute_tools_with_plugins(session2, &result, &tools, false, &plugins2)
                    .await
                    .unwrap();
            assert_eq!(session2.message_count(), 3);
            assert_eq!(plugin2.call_count.load(Ordering::SeqCst), 3);
        });
    }

    #[test]
    fn test_execute_tools_with_plugins_state_changes() {
        use crate::plugin::AgentPlugin;

        struct StatePlugin;

        #[async_trait]
        impl AgentPlugin for StatePlugin {
            fn id(&self) -> &str {
                "state_plugin"
            }

            fn initial_state(&self) -> Option<(&'static str, Value)> {
                Some(("plugin_data", json!({"initialized": true})))
            }
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let session = Session::with_initial_state("test", json!({"counter": 0}));
            let result = StreamResult {
                text: "State".to_string(),
                tool_calls: vec![crate::types::ToolCall::new(
                    "call_1",
                    "counter",
                    json!({"amount": 5}),
                )],
            };
            let tools = tool_map([CounterTool]);
            let plugins: Vec<Arc<dyn AgentPlugin>> = vec![Arc::new(StatePlugin)];

            let session =
                execute_tools_with_plugins(session, &result, &tools, true, &plugins).await.unwrap();

            // Tool should have produced a patch
            assert_eq!(session.patch_count(), 1);
            let state = session.rebuild_state().unwrap();
            assert_eq!(state["counter"], 5);
        });
    }
}
