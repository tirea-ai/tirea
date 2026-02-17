use crate::conversation::ToolCall;
use genai::chat::Usage;

/// Result of stream collection used by runtime and plugin phase contracts.
#[derive(Debug, Clone)]
pub struct StreamResult {
    /// Accumulated text content.
    pub text: String,
    /// Collected tool calls.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage from the LLM response.
    pub usage: Option<Usage>,
}

impl StreamResult {
    /// Check if tool execution is needed.
    pub fn needs_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}
