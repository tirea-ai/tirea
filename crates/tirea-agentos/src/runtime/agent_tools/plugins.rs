use super::state::current_unix_millis;
use super::*;
use crate::contracts::runtime::behavior::{AgentBehavior, ReadOnlyContext};
use crate::contracts::runtime::phase::{ActionSet, BeforeInferenceAction, LifecycleAction};
use crate::contracts::runtime::state::AnyStateAction;
use crate::runtime::background_tasks::{
    BackgroundTaskAction, BackgroundTaskManager, BackgroundTaskState, TaskStatus,
};
#[cfg(feature = "permission")]
use tirea_extension_permission::resolve_permission_behavior;

#[cfg(not(feature = "permission"))]
fn resolve_permission_behavior(
    _state: &serde_json::Value,
    _action: &str,
) -> ToolPermissionBehavior {
    ToolPermissionBehavior::Allow
}

pub struct AgentRecoveryPlugin {
    bg_manager: Arc<BackgroundTaskManager>,
}

impl AgentRecoveryPlugin {
    pub fn new(bg_manager: Arc<BackgroundTaskManager>) -> Self {
        Self { bg_manager }
    }
}

#[async_trait]
impl AgentBehavior for AgentRecoveryPlugin {
    fn id(&self) -> &str {
        AGENT_RECOVERY_PLUGIN_ID
    }

    async fn run_start(&self, ctx: &ReadOnlyContext<'_>) -> ActionSet<LifecycleAction> {
        use crate::contracts::runtime::{
            PendingToolCall, SuspendedCall, ToolCallResumeMode, ToolCallState,
        };
        use tirea_state::State;

        let state = ctx.snapshot();
        let tasks = state
            .get(BackgroundTaskState::PATH)
            .and_then(|v| BackgroundTaskState::from_value(v).ok())
            .map(|s| s.tasks)
            .unwrap_or_default();

        if tasks.is_empty() {
            return ActionSet::empty();
        }

        let has_suspended_recovery = has_suspended_recovery_interaction(&state);

        // Detect orphans: Running agent_run tasks in persisted state but no live handle.
        let mut orphaned_run_ids = Vec::new();
        let mut actions: ActionSet<LifecycleAction> = ActionSet::empty();

        for (task_id, task) in &tasks {
            if task.status != TaskStatus::Running {
                continue;
            }
            if task.task_type != AGENT_RUN_TOOL_ID {
                continue;
            }
            if !self.bg_manager.contains_any(task_id).await {
                // Orphaned: mark as stopped via SetStatus action.
                orphaned_run_ids.push(task_id.clone());
                actions = actions.and(LifecycleAction::State(AnyStateAction::new::<
                    BackgroundTaskState,
                >(
                    BackgroundTaskAction::SetStatus {
                        task_id: task_id.clone(),
                        status: TaskStatus::Stopped,
                        error: Some(
                            "No live executor found in current process; marked stopped".to_string(),
                        ),
                        completed_at_ms: None,
                    },
                )));
            }
        }

        if has_suspended_recovery || orphaned_run_ids.is_empty() {
            return actions;
        }

        let run_id = orphaned_run_ids[0].clone();
        let Some(task) = tasks.get(&run_id) else {
            return actions;
        };

        let agent_id = task
            .metadata
            .get("agent_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let behavior = resolve_permission_behavior(&state, AGENT_RECOVERY_INTERACTION_ACTION);

        let make_suspended_call = |interaction: &Suspension| -> SuspendedCall {
            let call_id = interaction.id.clone();
            let call_arguments = interaction.parameters.clone();
            let pending = PendingToolCall::new(
                call_id.clone(),
                AGENT_RECOVERY_INTERACTION_ACTION,
                call_arguments.clone(),
            );
            SuspendedCall {
                call_id,
                tool_name: AGENT_RUN_TOOL_ID.to_string(),
                arguments: call_arguments,
                ticket: crate::contracts::runtime::phase::SuspendTicket::new(
                    interaction.clone(),
                    pending,
                    ToolCallResumeMode::ReplayToolCall,
                ),
            }
        };

        match behavior {
            ToolPermissionBehavior::Allow => {
                let interaction = build_recovery_interaction(&run_id, agent_id);
                let suspended_call = make_suspended_call(&interaction);
                let call_id = suspended_call.call_id.clone();
                let resume_token = suspended_call.ticket.pending.id.clone();
                let arguments = suspended_call.arguments.clone();

                actions = actions.and(LifecycleAction::State(
                    suspended_call.clone().into_state_action(),
                ));
                actions = actions.and(LifecycleAction::State(
                    ToolCallState {
                        call_id: call_id.clone(),
                        tool_name: AGENT_RUN_TOOL_ID.to_string(),
                        arguments,
                        status: crate::contracts::runtime::ToolCallStatus::Resuming,
                        resume_token: Some(resume_token),
                        resume: Some(crate::contracts::runtime::ToolCallResume {
                            decision_id: recovery_target_id(&run_id),
                            action: crate::contracts::io::ResumeDecisionAction::Resume,
                            result: serde_json::Value::Bool(true),
                            reason: None,
                            updated_at: current_unix_millis(),
                        }),
                        scratch: serde_json::Value::Null,
                        updated_at: current_unix_millis(),
                    }
                    .into_state_action(),
                ));
            }
            ToolPermissionBehavior::Deny => {}
            ToolPermissionBehavior::Ask => {
                let interaction = build_recovery_interaction(&run_id, agent_id);
                let suspended_call = make_suspended_call(&interaction);
                actions = actions.and(LifecycleAction::State(suspended_call.into_state_action()));
            }
        }
        actions
    }
}

#[derive(Clone)]
pub struct AgentToolsPlugin {
    agents: Arc<dyn AgentRegistry>,
    max_entries: usize,
    max_chars: usize,
}

impl AgentToolsPlugin {
    pub fn new(agents: Arc<dyn AgentRegistry>) -> Self {
        Self {
            agents,
            max_entries: 64,
            max_chars: 16 * 1024,
        }
    }

    pub fn with_limits(mut self, max_entries: usize, max_chars: usize) -> Self {
        self.max_entries = max_entries.max(1);
        self.max_chars = max_chars.max(256);
        self
    }

    pub(super) fn render_available_agents(
        &self,
        caller_agent: Option<&str>,
        scope: Option<&tirea_contract::RunConfig>,
    ) -> String {
        let mut ids = self.agents.ids();
        ids.sort();
        if let Some(caller) = caller_agent {
            ids.retain(|id| id != caller);
        }
        ids.retain(|id| {
            is_scope_allowed(
                scope,
                id,
                SCOPE_ALLOWED_AGENTS_KEY,
                SCOPE_EXCLUDED_AGENTS_KEY,
            )
        });
        if ids.is_empty() {
            return String::new();
        }

        let total = ids.len();
        let mut out = String::new();
        out.push_str("<available_agents>\n");

        let mut shown = 0usize;
        for id in ids.into_iter().take(self.max_entries) {
            out.push_str("<agent>\n");
            out.push_str(&format!("<id>{}</id>\n", id));
            out.push_str("</agent>\n");
            shown += 1;
            if out.len() >= self.max_chars {
                break;
            }
        }

        out.push_str("</available_agents>\n");
        if shown < total {
            out.push_str(&format!(
                "Note: available_agents truncated (total={}, shown={}).\n",
                total, shown
            ));
        }

        out.push_str("<agent_tools_usage>\n");
        out.push_str("Run or resume: tool \"agent_run\" with {\"agent_id\":\"<id>\",\"prompt\":\"...\",\"fork_context\":false,\"background\":false}.\n");
        out.push_str("Resume existing run: tool \"agent_run\" with {\"run_id\":\"...\",\"prompt\":\"optional\",\"background\":false}.\n");
        out.push_str(
            "Check background run: tool \"task_status\" with {\"task_id\":\"<run_id>\"}.\n",
        );
        out.push_str(
            "Cancel background run: tool \"task_cancel\" with {\"task_id\":\"<run_id>\"}.\n",
        );
        out.push_str("Retrieve output: tool \"task_output\" with {\"task_id\":\"<run_id>\"}.\n");
        out.push_str(
            "Statuses: running, completed, failed, cancelled, stopped (stopped can be resumed).\n",
        );
        out.push_str("</agent_tools_usage>");

        if out.len() > self.max_chars {
            out.truncate(self.max_chars);
        }

        out.trim_end().to_string()
    }
}

#[async_trait]
impl AgentBehavior for AgentToolsPlugin {
    fn id(&self) -> &str {
        AGENT_TOOLS_PLUGIN_ID
    }

    async fn before_inference(
        &self,
        ctx: &ReadOnlyContext<'_>,
    ) -> ActionSet<BeforeInferenceAction> {
        let caller_agent = ctx
            .run_config()
            .value(SCOPE_CALLER_AGENT_ID_KEY)
            .and_then(|v| v.as_str());
        let rendered = self.render_available_agents(caller_agent, Some(ctx.run_config()));
        if rendered.is_empty() {
            ActionSet::empty()
        } else {
            ActionSet::single(BeforeInferenceAction::AddSystemContext(rendered))
        }
    }
}
