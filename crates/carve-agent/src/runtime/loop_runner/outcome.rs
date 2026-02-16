use super::*;

/// Single round execution for fine-grained control.
///
/// This allows callers to control the loop manually.
#[derive(Debug)]
pub enum RoundResult {
    /// LLM responded with text, no tools needed.
    Done { thread: Thread, response: String },
    /// LLM requested tool calls, tools have been executed.
    ToolsExecuted {
        thread: Thread,
        text: String,
        tool_calls: Vec<crate::contracts::conversation::ToolCall>,
    },
}

/// Run a single round of the agent loop.
///
/// This gives you fine-grained control over the loop.
pub async fn run_round(
    client: &Client,
    config: &AgentConfig,
    thread: Thread,
    tools: &HashMap<String, Arc<dyn Tool>>,
) -> Result<RoundResult, AgentLoopError> {
    // Run one step
    let (thread, result) = run_step(client, config, thread, tools).await?;

    if !result.needs_tools() {
        return Ok(RoundResult::Done {
            thread,
            response: result.text,
        });
    }

    // Execute tools
    let thread = execute_tools_with_config(thread, &result, tools, config).await?;

    Ok(RoundResult::ToolsExecuted {
        thread,
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
        thread: Box<Thread>,
        reason: StopReason,
    },
    /// Pending user interaction; execution should pause until the client responds.
    ///
    /// The returned `session` includes any patches applied up to the point where the
    /// interaction was requested (including persisting the pending interaction).
    #[error("Pending interaction: {id} ({action})", id = interaction.id, action = interaction.action)]
    PendingInteraction {
        thread: Box<Thread>,
        interaction: Box<Interaction>,
    },
    /// External cancellation signal requested run termination.
    #[error("Run cancelled")]
    Cancelled { thread: Box<Thread> },
}

impl AgentLoopError {
    /// Normalize loop errors into lifecycle termination semantics.
    pub fn termination_reason(&self) -> TerminationReason {
        match self {
            Self::Stopped { reason, .. } => TerminationReason::Stopped(reason.clone()),
            Self::Cancelled { .. } => TerminationReason::Cancelled,
            Self::PendingInteraction { .. } => TerminationReason::PendingInteraction,
            Self::LlmError(_) | Self::StateError(_) => TerminationReason::Error,
        }
    }
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
