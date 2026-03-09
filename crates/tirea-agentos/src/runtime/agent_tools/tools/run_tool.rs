use super::*;

#[derive(Debug, Clone)]
pub struct AgentRunTool {
    os: AgentOs,
    handles: Arc<SubAgentHandleTable>,
}

impl AgentRunTool {
    pub fn new(os: AgentOs, handles: Arc<SubAgentHandleTable>) -> Self {
        Self { os, handles }
    }

    fn ensure_target_visible(
        &self,
        target_agent_id: &str,
        caller_agent_id: Option<&str>,
        scope: Option<&tirea_contract::RunConfig>,
    ) -> Result<(), ToolArgError> {
        if is_target_agent_visible(
            self.os.agents_registry().as_ref(),
            target_agent_id,
            caller_agent_id,
            scope,
        ) {
            return Ok(());
        }

        Err(ToolArgError::new(
            "unknown_agent",
            format!("Unknown or unavailable agent_id: {target_agent_id}"),
        ))
    }

    async fn persist_live_summary(
        &self,
        ctx: &ToolCallContext<'_>,
        run_id: &str,
        child_thread_id: &str,
        parent_run_id: Option<String>,
        summary: &SubAgentSummary,
        tool_name: &str,
    ) -> ToolResult {
        let sub = SubAgent {
            thread_id: child_thread_id.to_string(),
            parent_run_id,
            agent_id: summary.agent_id.clone(),
            status: summary.status,
            error: summary.error.clone(),
        };
        if let Err(err) = ctx
            .state_of::<SubAgentState>()
            .runs_insert(run_id.to_string(), sub)
        {
            return state_write_failed(tool_name, err);
        }
        to_tool_result(tool_name, summary.clone())
    }

    async fn launch_new_run(
        &self,
        ctx: &ToolCallContext<'_>,
        request: LaunchNewRunRequest,
        tool_name: &str,
    ) -> ToolResult {
        let LaunchNewRunRequest {
            run_id,
            owner_thread_id,
            agent_id,
            parent_run_id,
            child_thread_id,
            messages,
            initial_state,
            background,
        } = request;
        let sub = SubAgent {
            thread_id: child_thread_id.clone(),
            parent_run_id: parent_run_id.clone(),
            agent_id: agent_id.clone(),
            status: SubAgentStatus::Running,
            error: None,
        };
        let parent_tool_call_id = ctx.call_id().to_string();

        if background {
            let token = RunCancellationToken::new();
            let parent_run_id_bg = parent_run_id.clone();
            let parent_tool_call_id_bg = parent_tool_call_id.clone();
            let epoch = self
                .handles
                .put_running(
                    &run_id,
                    owner_thread_id.clone(),
                    child_thread_id.clone(),
                    agent_id.clone(),
                    parent_run_id.clone(),
                    Some(token.clone()),
                )
                .await;

            // Persist Running record immediately.
            if let Err(err) = ctx
                .state_of::<SubAgentState>()
                .runs_insert(run_id.to_string(), sub)
            {
                let _ = self.handles.remove_if_epoch(&run_id, epoch).await;
                return state_write_failed(tool_name, err);
            }

            let handles = self.handles.clone();
            let os = self.os.clone();
            let run_id_bg = run_id.clone();
            let agent_id_bg = agent_id.clone();
            let child_thread_id_bg = child_thread_id;
            let parent_thread_id_bg = owner_thread_id;
            tokio::spawn(async move {
                let completion = execute_sub_agent(
                    os,
                    SubAgentExecutionRequest {
                        agent_id: agent_id_bg,
                        child_thread_id: child_thread_id_bg,
                        run_id: run_id_bg.clone(),
                        parent_run_id: parent_run_id_bg,
                        parent_tool_call_id: Some(parent_tool_call_id_bg),
                        parent_thread_id: parent_thread_id_bg,
                        messages,
                        initial_state,
                        cancellation_token: Some(token),
                    },
                    None,
                )
                .await;
                let _ = handles
                    .update_after_completion(&run_id_bg, epoch, completion)
                    .await;
            });

            return to_tool_result(
                tool_name,
                SubAgentSummary {
                    run_id,
                    agent_id,
                    status: SubAgentStatus::Running,
                    error: None,
                },
            );
        }

        // Foreground run.
        let epoch = self
            .handles
            .put_running(
                &run_id,
                owner_thread_id.clone(),
                child_thread_id.clone(),
                agent_id.clone(),
                parent_run_id.clone(),
                None,
            )
            .await;

        let forward_progress =
            |update: crate::contracts::runtime::tool_call::ToolCallProgressUpdate| {
                ctx.report_tool_call_progress(update)
            };

        let completion = execute_sub_agent(
            self.os.clone(),
            SubAgentExecutionRequest {
                agent_id: agent_id.clone(),
                child_thread_id: child_thread_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: parent_run_id.clone(),
                parent_tool_call_id: Some(parent_tool_call_id),
                parent_thread_id: owner_thread_id,
                messages,
                initial_state,
                cancellation_token: None,
            },
            Some(&forward_progress),
        )
        .await;

        let completed_sub = SubAgent {
            thread_id: child_thread_id,
            parent_run_id,
            agent_id: agent_id.clone(),
            status: completion.status,
            error: completion.error.clone(),
        };

        let summary = self
            .handles
            .update_after_completion(&run_id, epoch, completion)
            .await
            .unwrap_or_else(|| SubAgentSummary {
                run_id: run_id.clone(),
                agent_id,
                status: completed_sub.status,
                error: completed_sub.error.clone(),
            });

        if let Err(err) = ctx
            .state_of::<SubAgentState>()
            .runs_insert(run_id.to_string(), completed_sub)
        {
            return state_write_failed(tool_name, err);
        }
        to_tool_result(tool_name, summary)
    }
}

#[async_trait]
impl Tool for AgentRunTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            AGENT_RUN_TOOL_ID,
            "Agent Run",
            "Run or resume a registry agent; can run in background",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Target agent id (required for new runs)" },
                "prompt": { "type": "string", "description": "Input for the target agent" },
                "run_id": { "type": "string", "description": "Existing run id to resume or inspect" },
                "fork_context": { "type": "boolean", "description": "Whether to fork caller state/messages into the new run" },
                "background": { "type": "boolean", "description": "true: run in background; false: wait for completion" }
            }
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, crate::contracts::runtime::tool_call::ToolError> {
        let tool_name = AGENT_RUN_TOOL_ID;
        let run_id = optional_string(&args, "run_id");
        let background = required_bool(&args, "background", false);
        let fork_context = required_bool(&args, "fork_context", false);

        let scope = ctx.run_config();
        let owner_thread_id = scope_string(Some(scope), SCOPE_CALLER_SESSION_ID_KEY);
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_scope",
                "missing caller thread context",
            ));
        };
        let caller_agent_id = scope_string(Some(scope), SCOPE_CALLER_AGENT_ID_KEY);
        let caller_run_id = scope_run_id(Some(scope));

        // ── Resume existing run by ID ──────────────────────────────
        if let Some(run_id) = run_id {
            // 1. Check live handle first.
            if let Some(existing) = self
                .handles
                .get_owned_summary(&owner_thread_id, &run_id)
                .await
            {
                match existing.status {
                    SubAgentStatus::Running
                    | SubAgentStatus::Completed
                    | SubAgentStatus::Failed => {
                        let handle = match self
                            .handles
                            .handle_for_resume(&owner_thread_id, &run_id)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => return Ok(tool_error(tool_name, "unknown_run", e)),
                        };
                        let result = self
                            .persist_live_summary(
                                ctx,
                                &run_id,
                                &handle.child_thread_id,
                                handle.parent_run_id.clone(),
                                &existing,
                                tool_name,
                            )
                            .await;
                        return Ok(result);
                    }
                    SubAgentStatus::Stopped => {
                        // Resume from live handle.
                        let handle = match self
                            .handles
                            .handle_for_resume(&owner_thread_id, &run_id)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => return Ok(tool_error(tool_name, "unknown_run", e)),
                        };

                        if let Err(error) = self.ensure_target_visible(
                            &handle.agent_id,
                            caller_agent_id.as_deref(),
                            Some(scope),
                        ) {
                            return Ok(error.into_tool_result(tool_name));
                        }

                        let mut messages = Vec::new();
                        if let Some(prompt) = optional_string(&args, "prompt") {
                            messages.push(Message::user(prompt));
                        }

                        return Ok(self
                            .launch_new_run(
                                ctx,
                                LaunchNewRunRequest {
                                    run_id,
                                    owner_thread_id,
                                    agent_id: handle.agent_id,
                                    parent_run_id: caller_run_id,
                                    child_thread_id: handle.child_thread_id,
                                    messages,
                                    initial_state: None,
                                    background,
                                },
                                tool_name,
                            )
                            .await);
                    }
                }
            }

            // 2. Check persisted state.
            let persisted_opt = ctx
                .state_of::<SubAgentState>()
                .runs()
                .ok()
                .unwrap_or_default()
                .remove(&run_id);

            let Some(persisted) = persisted_opt else {
                return Ok(tool_error(
                    tool_name,
                    "unknown_run",
                    format!("Unknown run_id: {run_id}"),
                ));
            };

            // Orphaned running → mark stopped.
            if persisted.status == SubAgentStatus::Running {
                let stopped = SubAgent {
                    status: SubAgentStatus::Stopped,
                    error: Some(
                        "No live executor found in current process; marked stopped".to_string(),
                    ),
                    ..persisted.clone()
                };
                if let Err(err) = ctx
                    .state_of::<SubAgentState>()
                    .runs_insert(run_id.to_string(), stopped.clone())
                {
                    return Ok(state_write_failed(tool_name, err));
                }
                return Ok(to_tool_result(
                    tool_name,
                    SubAgentSummary {
                        run_id,
                        agent_id: stopped.agent_id,
                        status: SubAgentStatus::Stopped,
                        error: stopped.error,
                    },
                ));
            }

            match persisted.status {
                SubAgentStatus::Completed | SubAgentStatus::Failed => {
                    return Ok(to_tool_result(
                        tool_name,
                        SubAgentSummary {
                            run_id,
                            agent_id: persisted.agent_id,
                            status: persisted.status,
                            error: persisted.error,
                        },
                    ));
                }
                SubAgentStatus::Stopped => {
                    if let Err(error) = self.ensure_target_visible(
                        &persisted.agent_id,
                        caller_agent_id.as_deref(),
                        Some(scope),
                    ) {
                        return Ok(error.into_tool_result(tool_name));
                    }

                    let mut messages = Vec::new();
                    if let Some(prompt) = optional_string(&args, "prompt") {
                        messages.push(Message::user(prompt));
                    }

                    return Ok(self
                        .launch_new_run(
                            ctx,
                            LaunchNewRunRequest {
                                run_id,
                                owner_thread_id,
                                agent_id: persisted.agent_id,
                                parent_run_id: caller_run_id,
                                child_thread_id: persisted.thread_id,
                                messages,
                                initial_state: None,
                                background,
                            },
                            tool_name,
                        )
                        .await);
                }
                SubAgentStatus::Running => unreachable!("handled above"),
            }
        }

        // ── New run ────────────────────────────────────────────────
        let target_agent_id = match required_string(&args, "agent_id") {
            Ok(v) => v,
            Err(err) => return Ok(err.into_tool_result(tool_name)),
        };
        let prompt = match required_string(&args, "prompt") {
            Ok(v) => v,
            Err(err) => return Ok(err.into_tool_result(tool_name)),
        };

        if let Err(error) =
            self.ensure_target_visible(&target_agent_id, caller_agent_id.as_deref(), Some(scope))
        {
            return Ok(error.into_tool_result(tool_name));
        }

        let run_id = uuid::Uuid::now_v7().to_string();
        let child_thread_id = sub_agent_thread_id(&run_id);

        let (messages, initial_state) = if fork_context {
            let fork_state = scope
                .value(SCOPE_CALLER_STATE_KEY)
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut msgs = if let Some(caller_msgs) = parse_caller_messages(Some(scope)) {
                filtered_fork_messages(caller_msgs)
            } else {
                Vec::new()
            };
            msgs.push(Message::user(prompt));
            (msgs, Some(fork_state))
        } else {
            (vec![Message::user(prompt)], None)
        };

        Ok(self
            .launch_new_run(
                ctx,
                LaunchNewRunRequest {
                    run_id,
                    owner_thread_id,
                    agent_id: target_agent_id,
                    parent_run_id: caller_run_id,
                    child_thread_id,
                    messages,
                    initial_state,
                    background,
                },
                tool_name,
            )
            .await)
    }
}
