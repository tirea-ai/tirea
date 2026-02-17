use super::*;
use crate::contracts::AgentState as ContextAgentState;
pub struct AgentRecoveryPlugin {
    manager: Arc<AgentRunManager>,
}

impl AgentRecoveryPlugin {
    pub fn new(manager: Arc<AgentRunManager>) -> Self {
        Self { manager }
    }

    async fn on_run_start(&self, step: &mut StepContext<'_>, ctx: &ContextAgentState) {
        let state = match step.thread.rebuild_state() {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut runs = parse_persisted_runs_from_doc(&state);
        if runs.is_empty() {
            return;
        }

        let has_pending_interaction = state
            .get(AGENT_STATE_PATH)
            .and_then(|a| a.get("pending_interaction"))
            .is_some_and(|v| !v.is_null());

        let outcome =
            reconcile_persisted_runs(self.manager.as_ref(), &step.thread.id, &mut runs).await;
        if outcome.changed {
            if let Some(patch) = set_agent_runs_patch_from_state_doc(
                &state,
                runs.clone(),
                &format!("agent_recovery_reconcile_{}", step.thread.id),
            ) {
                step.pending_patches.push(patch);
            }
        }

        if has_pending_interaction || outcome.orphaned_run_ids.is_empty() {
            return;
        }

        let run_id = outcome.orphaned_run_ids[0].clone();
        let Some(run) = runs.get(&run_id) else {
            return;
        };

        let behavior = ctx.get_permission(AGENT_RECOVERY_INTERACTION_ACTION);
        match behavior {
            ToolPermissionBehavior::Allow => {
                schedule_recovery_replay(&state, step, &run_id);
            }
            ToolPermissionBehavior::Deny => {}
            ToolPermissionBehavior::Ask => {
                let interaction = build_recovery_interaction(&run_id, run);
                if let Some(patch) =
                    set_pending_interaction_patch(&state, interaction, "agent_recovery_pending")
                {
                    step.pending_patches.push(patch);
                }
            }
        }
    }

    async fn on_before_inference(&self, step: &mut StepContext<'_>) {
        let state = match step.thread.rebuild_state() {
            Ok(v) => v,
            Err(_) => return,
        };

        let Some(pending) = parse_pending_interaction_from_state(&state) else {
            return;
        };
        if pending.action == AGENT_RECOVERY_INTERACTION_ACTION {
            step.skip_inference = true;
        }
    }
}

#[async_trait]
impl AgentPlugin for AgentRecoveryPlugin {
    fn id(&self) -> &str {
        AGENT_RECOVERY_PLUGIN_ID
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &ContextAgentState) {
        match phase {
            Phase::RunStart => self.on_run_start(step, ctx).await,
            Phase::BeforeInference => self.on_before_inference(step).await,
            _ => {}
        }
    }
}

#[derive(Clone)]
pub struct AgentToolsPlugin {
    agents: Arc<dyn AgentRegistry>,
    manager: Arc<AgentRunManager>,
    max_entries: usize,
    max_chars: usize,
}

impl AgentToolsPlugin {
    pub fn new(agents: Arc<dyn AgentRegistry>, manager: Arc<AgentRunManager>) -> Self {
        Self {
            agents,
            manager,
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
        scope: Option<&carve_state::ScopeState>,
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
            "Stop running background run: tool \"agent_stop\" with {\"run_id\":\"...\"}.\n",
        );
        out.push_str("Statuses: running, completed, failed, stopped (stopped can be resumed).\n");
        out.push_str("</agent_tools_usage>");

        if out.len() > self.max_chars {
            out.truncate(self.max_chars);
        }

        out.trim_end().to_string()
    }

    async fn maybe_reminder(&self, step: &mut StepContext<'_>) {
        let owner_thread_id = step.thread.id.as_str();
        let runs = self
            .manager
            .running_or_stopped_for_owner(owner_thread_id)
            .await;
        if runs.is_empty() {
            return;
        }

        let mut s = String::new();
        s.push_str("<agent_runs>\n");
        let total = runs.len();
        let mut shown = 0usize;
        for r in runs.into_iter().take(self.max_entries) {
            s.push_str(&format!(
                "<run id=\"{}\" agent=\"{}\" status=\"{}\"/>\n",
                r.run_id,
                r.target_agent_id,
                r.status.as_str(),
            ));
            shown += 1;
            if s.len() >= self.max_chars {
                break;
            }
        }
        s.push_str("</agent_runs>\n");
        if shown < total {
            s.push_str(&format!(
                "Note: agent_runs truncated (total={}, shown={}).\n",
                total, shown
            ));
        }
        s.push_str("Use tool \"agent_run\" with run_id to resume/check, and \"agent_stop\" to stop running runs.");
        if s.len() > self.max_chars {
            s.truncate(self.max_chars);
        }
        step.reminder(s);
    }
}

#[async_trait]
impl AgentPlugin for AgentToolsPlugin {
    fn id(&self) -> &str {
        AGENT_TOOLS_PLUGIN_ID
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &AgentState) {
        match phase {
            Phase::BeforeInference => {
                let caller_agent = step
                    .thread
                    .scope
                    .value(SCOPE_CALLER_AGENT_ID_KEY)
                    .and_then(|v| v.as_str());
                let rendered = self.render_available_agents(caller_agent, Some(&step.thread.scope));
                if !rendered.is_empty() {
                    step.system(rendered);
                }
            }
            Phase::AfterToolExecute => {
                // Inject system reminders after tool execution so the reminder is persisted
                // as internal-system history for subsequent turns.
                self.maybe_reminder(step).await;
            }
            _ => {}
        }
    }
}
