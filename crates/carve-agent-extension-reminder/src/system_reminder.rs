//! System reminder trait for generating reminder messages.

use async_trait::async_trait;
use carve_agent_contract::context::ToolCallContext;

/// System reminder for generating reminder messages.
#[async_trait]
pub trait SystemReminder: Send + Sync {
    /// Unique identifier for this reminder.
    fn id(&self) -> &str;

    /// Generate a reminder message.
    async fn remind(&self, ctx: &ToolCallContext<'_>) -> Option<String>;
}
