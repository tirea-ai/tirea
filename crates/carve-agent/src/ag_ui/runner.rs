use crate::ag_ui::context::AGUIContext;
use crate::ag_ui::protocol::AGUIEvent;
use crate::ag_ui::request::{prepare_request_runtime, set_run_identity, RunAgentRequest};
use crate::r#loop::run_loop_stream_with_checkpoints;
use crate::r#loop::{run_loop_stream, AgentConfig, RunContext, StreamWithCheckpoints};
use crate::thread::Thread;
use crate::traits::tool::Tool;
use async_stream::stream;
use futures::{Stream, StreamExt};
use genai::Client;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

/// Run the agent loop and return a stream of AG-UI events.
///
/// This is the main entry point for AG-UI compatible agent execution.
/// It wraps `run_loop_stream` and converts all events to AG-UI format.
///
/// # Arguments
///
/// * `client` - GenAI client for LLM API calls
/// * `config` - Agent configuration (model, system prompt, etc.)
/// * `session` - Current session with conversation history
/// * `tools` - Available tools for the agent
/// * `thread_id` - AG-UI thread identifier
/// * `run_id` - AG-UI run identifier
///
/// # Returns
///
/// A stream of `AGUIEvent` that can be serialized to SSE format.
///
/// # Example
///
/// ```ignore
/// use carve_agent::ag_ui::run_agent_stream;
///
/// let stream = run_agent_stream(
///     client,
///     config,
///     thread,
///     tools,
///     "thread_123".to_string(),
///     "run_456".to_string(),
/// );
///
/// // Convert to SSE and send to client
/// while let Some(event) = stream.next().await {
///     let json = serde_json::to_string(&event)?;
///     send_sse(format!("data: {}\n\n", json));
/// }
/// ```
pub fn run_agent_stream(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    thread_id: String,
    run_id: String,
) -> Pin<Box<dyn Stream<Item = AGUIEvent> + Send>> {
    run_agent_stream_with_parent(client, config, thread, tools, thread_id, run_id, None)
}

/// Run the agent loop and return a stream of AG-UI events with an explicit parent run ID.
pub fn run_agent_stream_with_parent(
    client: Client,
    config: AgentConfig,
    mut thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    thread_id: String,
    run_id: String,
    parent_run_id: Option<String>,
) -> Pin<Box<dyn Stream<Item = AGUIEvent> + Send>> {
    Box::pin(stream! {
        let mut ctx = AGUIContext::new(thread_id.clone(), run_id.clone());

        set_run_identity(&mut thread, &run_id, parent_run_id.as_deref());

        let run_ctx = RunContext {
            cancellation_token: None,
        };
        let mut inner_stream = run_loop_stream(client, config, thread, tools, run_ctx);

        while let Some(event) = inner_stream.next().await {
            for ag_event in ctx.on_agent_event(&event) {
                yield ag_event;
            }
        }
    })
}

/// Run the agent loop with an AG-UI request, handling frontend tools automatically.
///
/// This function extracts frontend tool definitions from the request and configures
/// the agent loop to delegate their execution to the client via AG-UI events.
///
/// Frontend tools (defined with `execute: "frontend"`) will NOT be executed on the
/// backend. Instead, AG-UI `TOOL_CALL_*` events will be emitted for the client to
/// execute. Results should be included in the next request's message history.
///
/// # Arguments
///
/// * `client` - GenAI client for LLM API calls
/// * `config` - Agent configuration (model, system prompt, etc.)
/// * `session` - Current session with conversation history
/// * `tools` - Available backend tools (frontend tools are handled separately)
/// * `request` - AG-UI request containing thread_id, run_id, and tool definitions
///
/// # Example
///
/// ```ignore
/// let request = RunAgentRequest::new("thread_1", "run_1")
///     .with_tool(AGUIToolDef::backend("search", "Search the web"))
///     .with_tool(AGUIToolDef::frontend("copyToClipboard", "Copy to clipboard"));
///
/// let stream = run_agent_stream_with_request(
///     client,
///     config,
///     thread,
///     tools,  // Only backend tools
///     request,
/// );
/// ```
pub fn run_agent_stream_with_request(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    request: RunAgentRequest,
) -> Pin<Box<dyn Stream<Item = AGUIEvent> + Send>> {
    let (config, thread, tools) = prepare_request_runtime(config, thread, tools, &request);

    // Use existing run_agent_stream with the enhanced config
    run_agent_stream_with_parent(
        client,
        config,
        thread,
        tools,
        request.thread_id,
        request.run_id,
        request.parent_run_id,
    )
}

/// Run the agent loop with an AG-UI request and return internal `AgentEvent`s plus checkpoints.
pub fn run_agent_events_with_request(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    request: RunAgentRequest,
) -> StreamWithCheckpoints {
    run_agent_events_with_request_checkpoints(client, config, thread, tools, request)
}

/// Run the agent loop with an AG-UI request and return internal `AgentEvent`s plus session checkpoints.
pub fn run_agent_events_with_request_checkpoints(
    client: Client,
    config: AgentConfig,
    thread: Thread,
    tools: HashMap<String, Arc<dyn Tool>>,
    request: RunAgentRequest,
) -> StreamWithCheckpoints {
    let (config, mut thread, tools) = prepare_request_runtime(config, thread, tools, &request);
    set_run_identity(
        &mut thread,
        &request.run_id,
        request.parent_run_id.as_deref(),
    );

    let run_ctx = RunContext {
        cancellation_token: None,
    };

    run_loop_stream_with_checkpoints(client, config, thread, tools, run_ctx)
}
