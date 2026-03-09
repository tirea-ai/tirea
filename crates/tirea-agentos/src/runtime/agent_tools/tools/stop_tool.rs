use super::*;

#[derive(Debug, Clone)]
pub struct AgentStopTool {
    handles: Arc<SubAgentHandleTable>,
}

impl AgentStopTool {
    pub fn new(handles: Arc<SubAgentHandleTable>) -> Self {
        Self { handles }
    }
}

#[async_trait]
impl Tool for AgentStopTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            AGENT_STOP_TOOL_ID,
            "Agent Stop",
            "Stop a background agent run by run_id",
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
        let tool_name = AGENT_STOP_TOOL_ID;
        let run_id = match required_string(&args, "run_id") {
            Ok(v) => v,
            Err(err) => return Ok(err.into_tool_result(tool_name)),
        };
        let owner_thread_id = ctx
            .run_config()
            .value(SCOPE_CALLER_SESSION_ID_KEY)
            .and_then(|v: &serde_json::Value| v.as_str())
            .map(|v: &str| v.to_string());
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_scope",
                "missing caller thread context",
            ));
        };

        let mut persisted_runs = ctx
            .state_of::<SubAgentState>()
            .runs()
            .ok()
            .unwrap_or_default();
        let mut tree_ids = collect_descendant_run_ids_from_state(&persisted_runs, &run_id, true);
        if tree_ids.is_empty() {
            tree_ids.push(run_id.clone());
        }

        let mut summaries: HashMap<String, SubAgentSummary> = HashMap::new();
        let mut manager_error = None;

        match self
            .handles
            .stop_owned_tree(&owner_thread_id, &run_id)
            .await
        {
            Ok(stopped) => {
                for summary in stopped {
                    summaries.insert(summary.run_id.clone(), summary);
                }
            }
            Err(e) => {
                manager_error = Some(e);
            }
        }

        let mut stopped_any = !summaries.is_empty();
        for id in &tree_ids {
            let Some(run) = persisted_runs.get_mut(id) else {
                continue;
            };
            if run.status != SubAgentStatus::Running {
                continue;
            }

            if let Some(summary) = summaries.remove(id) {
                run.status = summary.status;
                run.error = summary.error;
            } else {
                run.status = SubAgentStatus::Stopped;
                run.error =
                    Some("No live executor found in current process; marked stopped".to_string());
            }
            stopped_any = true;
            if let Err(err) = ctx
                .state_of::<SubAgentState>()
                .runs_insert(id.to_string(), run.clone())
            {
                return Ok(state_write_failed(tool_name, err));
            }
        }

        if !stopped_any {
            if let Some(err) = manager_error {
                return Ok(tool_error(tool_name, "invalid_state", err));
            }
            return Ok(tool_error(
                tool_name,
                "invalid_state",
                format!("Run '{run_id}' cannot be stopped"),
            ));
        }

        if let Some(summary) = {
            if let Some(summary) = summaries.remove(&run_id) {
                Some(summary)
            } else {
                persisted_runs.get(&run_id).map(|run| SubAgentSummary {
                    run_id: run_id.clone(),
                    agent_id: run.agent_id.clone(),
                    status: run.status,
                    error: run.error.clone(),
                })
            }
        } {
            return Ok(to_tool_result(tool_name, summary));
        }

        let fallback_target = persisted_runs.remove(&run_id);
        if let Some(run) = fallback_target {
            return Ok(to_tool_result(
                tool_name,
                SubAgentSummary {
                    run_id,
                    agent_id: run.agent_id,
                    status: run.status,
                    error: run.error,
                },
            ));
        }

        Ok(tool_error(
            tool_name,
            "invalid_state",
            "No matching run state for stopped run",
        ))
    }
}
