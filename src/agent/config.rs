//! Agent definition and configuration.

use crate::contract::executor::{LlmExecutor, ToolExecutionMode};
use crate::contract::tool::Tool;
use std::collections::HashMap;
use std::sync::Arc;

/// The sole interface the agent loop sees.
pub struct AgentConfig {
    pub id: String,
    pub model: String,
    pub system_prompt: String,
    pub max_rounds: usize,
    pub tool_execution_mode: ToolExecutionMode,
    pub tools: HashMap<String, Arc<dyn Tool>>,
    pub llm_executor: Arc<dyn LlmExecutor>,
}

impl AgentConfig {
    pub fn new(
        id: impl Into<String>,
        model: impl Into<String>,
        system_prompt: impl Into<String>,
        llm_executor: Arc<dyn LlmExecutor>,
    ) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            system_prompt: system_prompt.into(),
            max_rounds: 16,
            tool_execution_mode: ToolExecutionMode::Sequential,
            tools: HashMap::new(),
            llm_executor,
        }
    }

    #[must_use]
    pub fn with_max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    #[must_use]
    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Self {
        let desc = tool.descriptor();
        self.tools.insert(desc.id, tool);
        self
    }

    #[must_use]
    pub fn with_tools(mut self, tools: Vec<Arc<dyn Tool>>) -> Self {
        for tool in tools {
            let desc = tool.descriptor();
            self.tools.insert(desc.id, tool);
        }
        self
    }

    pub fn tool_descriptors(&self) -> Vec<crate::contract::tool::ToolDescriptor> {
        self.tools.values().map(|t| t.descriptor()).collect()
    }
}
