//! System reminder trait for generating reminder messages.

use carve_agent_contract::AgentState;
use async_trait::async_trait;

/// System reminder for generating reminder messages.
#[async_trait]
pub trait SystemReminder: Send + Sync {
    /// Unique identifier for this reminder.
    fn id(&self) -> &str;

    /// Generate a reminder message.
    async fn remind(&self, ctx: &AgentState) -> Option<String>;
}
