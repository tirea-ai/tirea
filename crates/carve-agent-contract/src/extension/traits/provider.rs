//! ContextProvider trait for injecting context messages.
//!
//! Context providers inject dynamic content into the agent's context
//! based on the current state.

use async_trait::async_trait;
use crate::context::AgentState;
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
///
/// # Example
///
/// ```ignore
/// use carve_agent::contracts::traits::provider::{ContextCategory, ContextProvider};
/// use carve_agent::prelude::AgentState;
/// use carve_state_derive::State;
///
/// #[derive(State)]
/// struct BuildStatusState {
///     pub last_check: i64,
///     pub status: String,
/// }
///
/// struct BuildStatusProvider;
///
/// #[async_trait]
/// impl ContextProvider for BuildStatusProvider {
///     fn id(&self) -> &str {
///         "build_status"
///     }
///
///     fn category(&self) -> ContextCategory {
///         ContextCategory::ToolExecution
///     }
///
///     fn priority(&self) -> u32 {
///         100
///     }
///
///     async fn provide(&self, ctx: &AgentState<'_>) -> Vec<String> {
///         let state = ctx.state::<BuildStatusState>("components.build_status");
///
///         // Update last check time
///         state.set_last_check(current_timestamp());
///
///         // Return context messages based on status
///         if state.status().unwrap_or_default() == "failed" {
///             vec!["Build is currently failing. Please fix before deploying.".into()]
///         } else {
///             vec![]
///         }
///     }
/// }
/// ```
#[async_trait]
pub trait ContextProvider: Send + Sync {
    /// Unique identifier for this provider.
    fn id(&self) -> &str;

    /// Category determines when this provider is invoked.
    fn category(&self) -> ContextCategory;

    /// Priority within the category (lower = earlier).
    fn priority(&self) -> u32;

    /// Provide context messages.
    ///
    /// # Arguments
    ///
    /// - `ctx`: Context for state access (reference - framework extracts patch after execution)
    ///
    /// # Returns
    ///
    /// List of context messages to inject. Empty list means no injection.
    async fn provide(&self, ctx: &AgentState<'_>) -> Vec<String>;
}
