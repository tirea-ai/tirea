//! ContextProvider trait for injecting dynamic context messages.

use carve_agent_contract::AgentState;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Category determines when context is injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextCategory {
    /// Injected at session start, before message history.
    AgentState,
    /// Injected after user input.
    UserInput,
    /// Injected after tool execution.
    ToolExecution,
}

/// Context provider for injecting dynamic content.
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Unique identifier for this provider.
    fn id(&self) -> &str;

    /// Category determines when this provider is invoked.
    fn category(&self) -> ContextCategory;

    /// Priority within the category (lower = earlier).
    fn priority(&self) -> u32;

    /// Provide context messages.
    async fn provide(&self, ctx: &AgentState) -> Vec<String>;
}
