use super::*;
use tirea_contract::plugin::phase::PluginPhaseContext;
use tirea_extension_interaction::InteractionOutbox;
use tirea_state::State;
pub(super) fn as_delegation_record(
    summary: &AgentRunSummary,
    parent_run_id: Option<String>,
    thread: Option<crate::contracts::thread::Thread>,
) -> DelegationRecord {
    DelegationRecord {
        run_id: summary.run_id.clone(),
        parent_run_id,
        target_agent_id: summary.target_agent_id.clone(),
        status: summary.status,
        assistant: summary.assistant.clone(),
        error: summary.error.clone(),
        agent_state: thread,
    }
}

pub(super) fn as_agent_run_summary(run_id: &str, state: &DelegationRecord) -> AgentRunSummary {
    AgentRunSummary {
        run_id: run_id.to_string(),
        target_agent_id: state.target_agent_id.clone(),
        status: state.status,
        assistant: state.assistant.clone(),
        error: state.error.clone(),
    }
}

pub(super) fn parse_persisted_runs_from_doc(doc: &Value) -> HashMap<String, DelegationRecord> {
    doc.get(DelegationState::PATH)
        .and_then(|v| v.get("runs"))
        .cloned()
        .and_then(|v| serde_json::from_value::<HashMap<String, DelegationRecord>>(v).ok())
        .unwrap_or_default()
}

pub(super) fn make_orphaned_running_state(run: &DelegationRecord) -> DelegationRecord {
    let mut next = run.clone();
    next.status = DelegationStatus::Stopped;
    next.error = Some("No live executor found in current process; marked stopped".to_string());
    next
}

pub(super) fn collect_descendant_run_ids_from_state(
    runs: &HashMap<String, DelegationRecord>,
    root_run_id: &str,
    include_root: bool,
) -> Vec<String> {
    if !runs.contains_key(root_run_id) {
        return Vec::new();
    }

    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    for (run_id, run) in runs.iter() {
        if let Some(parent_run_id) = &run.parent_run_id {
            children_by_parent
                .entry(parent_run_id.clone())
                .or_default()
                .push(run_id.clone());
        }
    }
    super::collect_descendant_run_ids(&children_by_parent, root_run_id, include_root)
}

pub(super) fn recovery_interaction_id(run_id: &str) -> String {
    format!("{AGENT_RECOVERY_INTERACTION_PREFIX}{run_id}")
}

pub(super) fn build_recovery_interaction(run_id: &str, run: &DelegationRecord) -> Interaction {
    Interaction::new(
        recovery_interaction_id(run_id),
        AGENT_RECOVERY_INTERACTION_ACTION,
    )
    .with_message(format!(
        "Detected interrupted run '{run_id}' (agent '{}'). Resume now?",
        run.target_agent_id
    ))
    .with_parameters(json!({
        "run_id": run_id,
        "agent_id": run.target_agent_id,
        "background": false
    }))
    .with_response_schema(json!({
        "type": "boolean"
    }))
}

pub(super) fn parse_pending_interaction_from_state(state: &Value) -> Option<Interaction> {
    state
        .get(crate::runtime::control::LoopControlState::PATH)
        .and_then(|a| a.get("pending_interaction"))
        .cloned()
        .and_then(|v| serde_json::from_value::<Interaction>(v).ok())
}

pub(super) fn schedule_recovery_replay(step: &impl PluginPhaseContext, run_id: &str) {
    let outbox = step.state_of::<InteractionOutbox>();
    let mut replay_calls = outbox.replay_tool_calls().ok().unwrap_or_default();

    let exists = replay_calls.iter().any(|call| {
        call.name == AGENT_RUN_TOOL_ID
            && call
                .arguments
                .get("run_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id == run_id)
    });
    if exists {
        return;
    }

    replay_calls.push(ToolCall::new(
        format!("agent_recovery_resume_{run_id}"),
        AGENT_RUN_TOOL_ID,
        json!({
            "run_id": run_id,
            "background": false
        }),
    ));
    let _ = outbox.set_replay_tool_calls(replay_calls);
}

#[derive(Debug, Default, Clone)]
pub(super) struct ReconcileOutcome {
    pub(super) changed: bool,
    pub(super) orphaned_run_ids: Vec<String>,
}

pub(super) async fn reconcile_persisted_runs(
    manager: &AgentRunManager,
    owner_thread_id: &str,
    runs: &mut HashMap<String, DelegationRecord>,
) -> ReconcileOutcome {
    let summaries = manager.all_for_owner(owner_thread_id).await;
    let mut by_id: HashMap<String, AgentRunSummary> = HashMap::new();
    for summary in summaries {
        by_id.insert(summary.run_id.clone(), summary);
    }

    let mut changed = false;
    let mut orphaned_run_ids = Vec::new();
    let mut known_ids: Vec<String> = runs.keys().cloned().collect();
    known_ids.sort();

    for run_id in known_ids {
        let Some(current) = runs.get(&run_id).cloned() else {
            continue;
        };
        if let Some(summary) = by_id.get(&run_id) {
            let thread = manager.owned_record(owner_thread_id, &run_id).await;
            let next = as_delegation_record(
                summary,
                current.parent_run_id.clone(),
                thread.or_else(|| current.agent_state.clone()),
            );
            if current.status != next.status
                || current.assistant != next.assistant
                || current.error != next.error
                || current.parent_run_id != next.parent_run_id
                || current.agent_state.as_ref().map(|s| &s.id)
                    != next.agent_state.as_ref().map(|s| &s.id)
            {
                runs.insert(run_id.clone(), next);
                changed = true;
            }
            continue;
        }

        if current.status == DelegationStatus::Running {
            runs.insert(run_id.clone(), make_orphaned_running_state(&current));
            changed = true;
            orphaned_run_ids.push(run_id);
        }
    }

    for (run_id, summary) in by_id {
        if runs.contains_key(&run_id) {
            continue;
        }
        let thread = manager.owned_record(owner_thread_id, &run_id).await;
        runs.insert(run_id.clone(), as_delegation_record(&summary, None, thread));
        changed = true;
    }

    ReconcileOutcome {
        changed,
        orphaned_run_ids,
    }
}
