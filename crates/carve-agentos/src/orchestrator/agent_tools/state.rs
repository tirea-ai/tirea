use super::*;
use carve_agent_contract::context::ToolCallContext;
use carve_state::{DocCell, State};
use std::sync::{Arc, Mutex};
pub(super) fn parent_run_id_from_thread(
    thread: Option<&crate::contracts::state::AgentState>,
) -> Option<String> {
    thread
        .and_then(|s| s.scope.value(SCOPE_PARENT_RUN_ID_KEY))
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub(super) fn as_delegation_record(
    summary: &AgentRunSummary,
    thread: Option<crate::contracts::state::AgentState>,
) -> DelegationRecord {
    let parent_run_id = parent_run_id_from_thread(thread.as_ref());
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

const INTERACTION_OUTBOX_PATH: &str = "interaction_outbox";

pub(super) fn parse_pending_interaction_from_state(state: &Value) -> Option<Interaction> {
    state
        .get(crate::runtime::control::LoopControlState::PATH)
        .and_then(|a| a.get("pending_interaction"))
        .cloned()
        .and_then(|v| serde_json::from_value::<Interaction>(v).ok())
}

pub(super) fn parse_replay_tool_calls_from_state(state: &Value) -> Vec<ToolCall> {
    state
        .get(INTERACTION_OUTBOX_PATH)
        .and_then(|a| a.get("replay_tool_calls"))
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<ToolCall>>(v).ok())
        .unwrap_or_default()
}

/// Create storage for a short-lived ToolCallContext used for state patch generation.
fn patch_context_storage(
    state: &Value,
) -> (DocCell, Mutex<Vec<carve_state::Op>>, Arc<Mutex<Vec<carve_state::Op>>>, carve_state::ScopeState, Mutex<Vec<Arc<crate::contracts::state::Message>>>) {
    (
        DocCell::new(state.clone()),
        Mutex::new(Vec::new()),
        Arc::new(Mutex::new(Vec::new())),
        carve_state::ScopeState::default(),
        Mutex::new(Vec::new()),
    )
}

pub(super) fn set_pending_interaction_patch(
    state: &Value,
    interaction: Interaction,
    call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    let (doc, ops, overlay, scope, pending_msgs) = patch_context_storage(state);
    let ctx = ToolCallContext::new(&doc, &ops, overlay, call_id, AGENT_RECOVERY_PLUGIN_ID, &scope, &pending_msgs, None);
    let lc = ctx.state_of::<crate::runtime::control::LoopControlState>();
    lc.set_pending_interaction(Some(interaction));
    let patch = ctx.take_patch();
    if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    }
}

pub(super) fn set_replay_tool_calls_patch(
    _state: &Value,
    replay_calls: Vec<ToolCall>,
    _call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    // Write to interaction_outbox via raw patch (no type dependency on InteractionOutbox)
    let outbox_path = carve_state::Path::root().key(INTERACTION_OUTBOX_PATH);
    let value = serde_json::to_value(&replay_calls).unwrap_or_default();
    let patch = carve_state::Patch::new().with_op(carve_state::Op::set(
        outbox_path.key("replay_tool_calls"),
        value,
    ));
    if patch.is_empty() {
        None
    } else {
        Some(carve_state::TrackedPatch::new(patch).with_source(AGENT_RECOVERY_PLUGIN_ID))
    }
}

pub(super) fn schedule_recovery_replay(state: &Value, step: &mut StepContext<'_>, run_id: &str) {
    let mut replay_calls = parse_replay_tool_calls_from_state(state);

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
    if let Some(patch) = set_replay_tool_calls_patch(
        state,
        replay_calls,
        &format!("agent_recovery_replay_{run_id}"),
    ) {
        step.pending_patches.push(patch);
    }
}

#[derive(Debug, Default, Clone)]
pub(super) struct ReconcileOutcome {
    pub(super) changed: bool,
    pub(super) orphaned_run_ids: Vec<String>,
}

pub(super) fn set_agent_runs_patch_from_state_doc(
    state: &Value,
    next_runs: HashMap<String, DelegationRecord>,
    call_id: &str,
) -> Option<carve_state::TrackedPatch> {
    let (doc, ops, overlay, scope, pending_msgs) = patch_context_storage(state);
    let ctx = ToolCallContext::new(&doc, &ops, overlay, call_id, AGENT_TOOLS_PLUGIN_ID, &scope, &pending_msgs, None);
    let agent = ctx.state_of::<DelegationState>();
    agent.set_runs(next_runs);
    let patch = ctx.take_patch();
    if patch.patch().is_empty() {
        None
    } else {
        Some(patch)
    }
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
            let mut next =
                as_delegation_record(summary, thread.or_else(|| current.agent_state.clone()));
            if next.parent_run_id.is_none() {
                next.parent_run_id = current.parent_run_id.clone();
            }
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
        runs.insert(run_id.clone(), as_delegation_record(&summary, thread));
        changed = true;
    }

    ReconcileOutcome {
        changed,
        orphaned_run_ids,
    }
}
