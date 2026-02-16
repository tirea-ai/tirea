use super::*;

fn to_tool_result(tool_name: &str, summary: AgentRunSummary) -> ToolResult {
    ToolResult::success(
        tool_name,
        json!({
            "run_id": summary.run_id,
            "agent_id": summary.target_agent_id,
            "status": summary.status.as_str(),
            "assistant": summary.assistant,
            "error": summary.error,
        }),
    )
}

fn tool_error(tool_name: &str, code: &str, message: impl Into<String>) -> ToolResult {
    let message = message.into();
    ToolResult {
        tool_name: tool_name.to_string(),
        status: ToolStatus::Error,
        data: json!({
            "error": {
                "code": code,
                "message": message,
            }
        }),
        message: Some(format!("[{code}] {message}")),
        metadata: HashMap::new(),
    }
}

fn runtime_run_id(runtime: Option<&carve_state::ScopeState>) -> Option<String> {
    runtime
        .and_then(|rt| rt.value(RUNTIME_RUN_ID_KEY))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn bind_child_lineage(
    mut thread: crate::contracts::conversation::Thread,
    run_id: &str,
    parent_run_id: Option<&str>,
    parent_thread_id: Option<&str>,
) -> crate::contracts::conversation::Thread {
    if thread.parent_thread_id.is_none() {
        thread.parent_thread_id = parent_thread_id.map(str::to_string);
    }
    let current_run_id = thread
        .runtime
        .value(RUNTIME_RUN_ID_KEY)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let current_parent_run_id = thread
        .runtime
        .value(RUNTIME_PARENT_RUN_ID_KEY)
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let parent_mismatch = match (current_parent_run_id.as_deref(), parent_run_id) {
        (Some(cur), Some(expected)) => cur != expected,
        (Some(_), None) => true,
        _ => false,
    };
    let run_mismatch = current_run_id.as_deref().is_some_and(|cur| cur != run_id);

    if run_mismatch || parent_mismatch {
        thread = thread.with_scope(carve_state::ScopeState::new());
    }

    if thread.runtime.value(RUNTIME_RUN_ID_KEY).is_none() {
        let _ = thread.runtime.set(RUNTIME_RUN_ID_KEY, run_id);
    }
    if let Some(parent_run_id) = parent_run_id {
        if thread.runtime.value(RUNTIME_PARENT_RUN_ID_KEY).is_none() {
            let _ = thread.runtime.set(RUNTIME_PARENT_RUN_ID_KEY, parent_run_id);
        }
    }
    thread
}

fn required_bool(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn optional_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn required_string(args: &Value, key: &str, tool_name: &str) -> Result<String, ToolResult> {
    optional_string(args, key)
        .ok_or_else(|| tool_error(tool_name, "invalid_arguments", format!("missing '{key}'")))
}

fn parse_caller_messages(runtime: Option<&carve_state::ScopeState>) -> Option<Vec<Message>> {
    let value = runtime.and_then(|rt| rt.value(RUNTIME_CALLER_MESSAGES_KEY))?;
    serde_json::from_value::<Vec<Message>>(value.clone()).ok()
}

fn filtered_fork_messages(messages: Vec<Message>) -> Vec<Message> {
    messages
        .into_iter()
        .filter(|m| m.visibility == crate::contracts::conversation::Visibility::All)
        .filter(|m| matches!(m.role, Role::System | Role::User | Role::Assistant))
        .map(|mut m| {
            if m.role == Role::Assistant {
                m.tool_calls = None;
            }
            m.tool_call_id = None;
            m
        })
        .collect()
}

fn is_target_agent_visible(
    registry: &dyn AgentRegistry,
    target: &str,
    caller: Option<&str>,
    runtime: Option<&carve_state::ScopeState>,
) -> bool {
    if caller.is_some_and(|c| c == target) {
        return false;
    }
    if !is_runtime_allowed(
        runtime,
        target,
        RUNTIME_ALLOWED_AGENTS_KEY,
        RUNTIME_EXCLUDED_AGENTS_KEY,
    ) {
        return false;
    }
    registry.get(target).is_some()
}

#[derive(Debug, Clone)]
pub struct AgentRunTool {
    os: AgentOs,
    manager: Arc<AgentRunManager>,
}

#[derive(Debug, Clone)]
struct RunLaunch {
    run_id: String,
    owner_thread_id: String,
    target_agent_id: String,
    parent_run_id: Option<String>,
    thread: crate::contracts::conversation::Thread,
}

impl AgentRunTool {
    pub fn new(os: AgentOs, manager: Arc<AgentRunManager>) -> Self {
        Self { os, manager }
    }

    fn ensure_target_visible(
        &self,
        target_agent_id: &str,
        caller_agent_id: Option<&str>,
        runtime: Option<&carve_state::ScopeState>,
        tool_name: &str,
    ) -> Result<(), ToolResult> {
        if is_target_agent_visible(
            self.os.agents_registry().as_ref(),
            target_agent_id,
            caller_agent_id,
            runtime,
        ) {
            return Ok(());
        }

        Err(tool_error(
            tool_name,
            "unknown_agent",
            format!("Unknown or unavailable agent_id: {target_agent_id}"),
        ))
    }

    async fn persist_existing_live_summary(
        &self,
        ctx: &Context<'_>,
        owner_thread_id: &str,
        run_id: &str,
        summary: AgentRunSummary,
        tool_name: &str,
    ) -> ToolResult {
        let thread = self.manager.owned_record(owner_thread_id, run_id).await;
        set_persisted_run(ctx, run_id, as_agent_run_state(&summary, thread));
        to_tool_result(tool_name, summary)
    }

    async fn launch_run(
        &self,
        ctx: &Context<'_>,
        launch: RunLaunch,
        background: bool,
        tool_name: &str,
    ) -> ToolResult {
        let RunLaunch {
            run_id,
            owner_thread_id,
            target_agent_id,
            parent_run_id,
            thread,
        } = launch;

        if background {
            let token = RunCancellationToken::new();
            let epoch = self
                .manager
                .put_running(
                    &run_id,
                    owner_thread_id,
                    target_agent_id.clone(),
                    parent_run_id.clone(),
                    thread.clone(),
                    Some(token.clone()),
                )
                .await;
            let manager = self.manager.clone();
            let os = self.os.clone();
            let run_id_bg = run_id.clone();
            let target_agent_id_bg = target_agent_id.clone();
            let child_thread_bg = thread.clone();
            tokio::spawn(async move {
                let completion =
                    execute_target_agent(os, target_agent_id_bg, child_thread_bg, Some(token))
                        .await;
                let _ = manager
                    .update_after_completion(&run_id_bg, epoch, completion)
                    .await;
            });

            let running = AgentRunState {
                run_id: run_id.clone(),
                parent_run_id,
                target_agent_id,
                status: AgentRunStatus::Running,
                assistant: None,
                error: None,
                thread: Some(thread),
            };
            set_persisted_run(ctx, &run_id, running.clone());
            return to_tool_result(tool_name, as_agent_run_summary(&run_id, &running));
        }

        let epoch = self
            .manager
            .put_running(
                &run_id,
                owner_thread_id,
                target_agent_id.clone(),
                parent_run_id.clone(),
                thread.clone(),
                None,
            )
            .await;
        let completion =
            execute_target_agent(self.os.clone(), target_agent_id.clone(), thread, None).await;
        let completion_state = AgentRunState {
            run_id: run_id.clone(),
            parent_run_id,
            target_agent_id,
            status: completion.status,
            assistant: completion.assistant.clone(),
            error: completion.error.clone(),
            thread: Some(completion.thread.clone()),
        };
        let summary = self
            .manager
            .update_after_completion(&run_id, epoch, completion)
            .await
            .unwrap_or_else(|| as_agent_run_summary(&run_id, &completion_state));
        set_persisted_run(ctx, &run_id, completion_state);
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
        ctx: &Context<'_>,
    ) -> Result<ToolResult, crate::contracts::traits::tool::ToolError> {
        let tool_name = AGENT_RUN_TOOL_ID;
        let run_id = optional_string(&args, "run_id");
        let background = required_bool(&args, "background", false);
        let fork_context = required_bool(&args, "fork_context", false);

        let runtime = ctx.scope_ref();
        let owner_thread_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller thread context",
            ));
        };
        let caller_agent_id = runtime
            .and_then(|rt| rt.value(RUNTIME_CALLER_AGENT_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let caller_run_id = runtime_run_id(runtime);

        if let Some(run_id) = run_id {
            if let Some(existing) = self
                .manager
                .get_owned_summary(&owner_thread_id, &run_id)
                .await
            {
                match existing.status {
                    AgentRunStatus::Running
                    | AgentRunStatus::Completed
                    | AgentRunStatus::Failed => {
                        let result = self
                            .persist_existing_live_summary(
                                ctx,
                                &owner_thread_id,
                                &run_id,
                                existing,
                                tool_name,
                            )
                            .await;
                        return Ok(result);
                    }
                    AgentRunStatus::Stopped => {
                        let record = match self
                            .manager
                            .record_for_resume(&owner_thread_id, &run_id)
                            .await
                        {
                            Ok(v) => v,
                            Err(e) => return Ok(tool_error(tool_name, "unknown_run", e)),
                        };

                        if let Err(error) = self.ensure_target_visible(
                            &record.target_agent_id,
                            caller_agent_id.as_deref(),
                            runtime,
                            tool_name,
                        ) {
                            return Ok(error);
                        }

                        let mut child_thread = bind_child_lineage(
                            record.thread,
                            &run_id,
                            caller_run_id.as_deref(),
                            Some(&owner_thread_id),
                        );
                        if let Some(prompt) = optional_string(&args, "prompt") {
                            child_thread = child_thread.with_message(Message::user(prompt));
                        }

                        let launch = RunLaunch {
                            run_id,
                            owner_thread_id,
                            target_agent_id: record.target_agent_id,
                            parent_run_id: caller_run_id,
                            thread: child_thread,
                        };
                        return Ok(self.launch_run(ctx, launch, background, tool_name).await);
                    }
                }
            }

            let Some(mut persisted) = parse_persisted_runs(ctx).remove(&run_id) else {
                return Ok(tool_error(
                    tool_name,
                    "unknown_run",
                    format!("Unknown run_id: {run_id}"),
                ));
            };

            let orphaned_running = persisted.status == AgentRunStatus::Running;
            if orphaned_running {
                persisted = make_orphaned_running_state(&persisted);
                set_persisted_run(ctx, &run_id, persisted.clone());
                return Ok(to_tool_result(
                    tool_name,
                    as_agent_run_summary(&run_id, &persisted),
                ));
            }

            match persisted.status {
                AgentRunStatus::Running | AgentRunStatus::Completed | AgentRunStatus::Failed => {
                    return Ok(to_tool_result(
                        tool_name,
                        as_agent_run_summary(&run_id, &persisted),
                    ));
                }
                AgentRunStatus::Stopped => {
                    if let Err(error) = self.ensure_target_visible(
                        &persisted.target_agent_id,
                        caller_agent_id.as_deref(),
                        runtime,
                        tool_name,
                    ) {
                        return Ok(error);
                    }

                    let mut child_thread = match persisted.thread {
                        Some(s) => s,
                        None => {
                            return Ok(tool_error(
                                tool_name,
                                "invalid_state",
                                format!("Run '{run_id}' cannot be resumed: missing child thread"),
                            ))
                        }
                    };
                    child_thread = bind_child_lineage(
                        child_thread,
                        &run_id,
                        caller_run_id.as_deref(),
                        Some(&owner_thread_id),
                    );

                    if let Some(prompt) = optional_string(&args, "prompt") {
                        child_thread = child_thread.with_message(Message::user(prompt));
                    }

                    let launch = RunLaunch {
                        run_id,
                        owner_thread_id,
                        target_agent_id: persisted.target_agent_id,
                        parent_run_id: caller_run_id,
                        thread: child_thread,
                    };
                    return Ok(self.launch_run(ctx, launch, background, tool_name).await);
                }
            }
        }

        let target_agent_id = match required_string(&args, "agent_id", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let prompt = match required_string(&args, "prompt", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };

        if let Err(error) = self.ensure_target_visible(
            &target_agent_id,
            caller_agent_id.as_deref(),
            runtime,
            tool_name,
        ) {
            return Ok(error);
        }

        let run_id = uuid::Uuid::now_v7().to_string();
        let thread_id = format!("agent-run-{run_id}");

        let mut child_thread = if fork_context {
            let fork_state = runtime
                .and_then(|rt| rt.value(RUNTIME_CALLER_STATE_KEY))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let mut forked =
                crate::contracts::conversation::Thread::with_initial_state(thread_id, fork_state);
            if let Some(messages) = parse_caller_messages(runtime) {
                forked = forked.with_messages(filtered_fork_messages(messages));
            }
            forked
        } else {
            crate::contracts::conversation::Thread::new(thread_id)
        };
        child_thread = child_thread.with_message(Message::user(prompt));
        child_thread = bind_child_lineage(
            child_thread,
            &run_id,
            caller_run_id.as_deref(),
            Some(&owner_thread_id),
        );

        let launch = RunLaunch {
            run_id,
            owner_thread_id,
            target_agent_id,
            parent_run_id: caller_run_id,
            thread: child_thread,
        };
        Ok(self.launch_run(ctx, launch, background, tool_name).await)
    }
}

#[derive(Debug, Clone)]
pub struct AgentStopTool {
    manager: Arc<AgentRunManager>,
}

impl AgentStopTool {
    pub fn new(manager: Arc<AgentRunManager>) -> Self {
        Self { manager }
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
        ctx: &Context<'_>,
    ) -> Result<ToolResult, crate::contracts::traits::tool::ToolError> {
        let tool_name = AGENT_STOP_TOOL_ID;
        let run_id = match required_string(&args, "run_id", tool_name) {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let owner_thread_id = ctx
            .scope_ref()
            .and_then(|rt| rt.value(RUNTIME_CALLER_SESSION_ID_KEY))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let Some(owner_thread_id) = owner_thread_id else {
            return Ok(tool_error(
                tool_name,
                "missing_runtime",
                "missing caller thread context",
            ));
        };

        let mut persisted_runs = parse_persisted_runs(ctx);
        let mut tree_ids = collect_descendant_run_ids_from_state(&persisted_runs, &run_id, true);
        if tree_ids.is_empty() {
            tree_ids.push(run_id.clone());
        }

        let mut summaries: HashMap<String, AgentRunSummary> = HashMap::new();
        let mut manager_error = None;

        match self
            .manager
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
            if run.status != AgentRunStatus::Running {
                continue;
            }

            if let Some(summary) = summaries.remove(id) {
                run.status = summary.status;
                run.assistant = summary.assistant;
                run.error = summary.error;
            } else {
                let stopped = make_orphaned_running_state(run);
                *run = stopped;
            }
            stopped_any = true;
            set_persisted_run(ctx, id, run.clone());
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
                persisted_runs
                    .get(&run_id)
                    .map(|run| as_agent_run_summary(&run_id, run))
            }
        } {
            return Ok(to_tool_result(tool_name, summary));
        }

        let fallback_target = persisted_runs.remove(&run_id);
        if let Some(run) = fallback_target {
            return Ok(to_tool_result(
                tool_name,
                as_agent_run_summary(&run_id, &run),
            ));
        }

        Ok(tool_error(
            tool_name,
            "invalid_state",
            "No matching run state for stopped run",
        ))
    }
}
