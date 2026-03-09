use super::*;

#[derive(Debug, Clone)]
pub struct AgentOutputTool {
    os: AgentOs,
}

impl AgentOutputTool {
    pub fn new(os: AgentOs) -> Self {
        Self { os }
    }
}

#[async_trait]
impl Tool for AgentOutputTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            AGENT_OUTPUT_TOOL_ID,
            "Agent Output",
            "Retrieve the latest output from a sub-agent run",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "run_id": { "type": "string", "description": "Run id returned by agent_run" }
            },
            "required": ["run_id"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, crate::contracts::runtime::tool_call::ToolError> {
        let tool_name = AGENT_OUTPUT_TOOL_ID;
        let run_id = match required_string(&args, "run_id") {
            Ok(v) => v,
            Err(err) => return Ok(err.into_tool_result(tool_name)),
        };

        let persisted = ctx
            .state_of::<SubAgentState>()
            .runs()
            .ok()
            .unwrap_or_default();

        let Some(sub) = persisted.get(&run_id) else {
            return Ok(tool_error(
                tool_name,
                "unknown_run",
                format!("Unknown run_id: {run_id}"),
            ));
        };

        let output = match self.os.load_thread(&sub.thread_id).await {
            Ok(Some(head)) => head
                .thread
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone()),
            Ok(None) => None,
            Err(e) => {
                return Ok(tool_error(
                    tool_name,
                    "store_error",
                    format!("failed to load sub-agent thread: {e}"),
                ));
            }
        };

        Ok(ToolResult::success(
            tool_name,
            json!({
                "run_id": run_id,
                "agent_id": sub.agent_id,
                "status": sub.status.as_str(),
                "output": output,
            }),
        ))
    }
}
