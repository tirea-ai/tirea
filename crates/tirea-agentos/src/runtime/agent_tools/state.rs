use super::*;
use tirea_contract::runtime::suspended_calls_from_state;

pub(super) fn parse_persisted_runs_from_doc(state: &Value) -> HashMap<String, SubAgent> {
    let Some(sub_agents) = state.get("sub_agents") else {
        return HashMap::new();
    };

    let runs_value = sub_agents.get("runs").unwrap_or(sub_agents);
    serde_json::from_value(runs_value.clone()).unwrap_or_default()
}

pub(super) fn collect_descendant_run_ids_from_state(
    runs: &HashMap<String, SubAgent>,
    root_run_id: &str,
    include_root: bool,
) -> Vec<String> {
    if !runs.contains_key(root_run_id) {
        return Vec::new();
    }

    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    for (run_id, run) in runs {
        if let Some(parent_run_id) = &run.parent_run_id {
            children_by_parent
                .entry(parent_run_id.clone())
                .or_default()
                .push(run_id.clone());
        }
    }
    for children in children_by_parent.values_mut() {
        children.sort();
    }

    collect_descendant_run_ids(&children_by_parent, root_run_id, include_root)
}

pub(super) fn recovery_target_id(run_id: &str) -> String {
    format!("{AGENT_RECOVERY_INTERACTION_PREFIX}{run_id}")
}

pub(super) fn build_recovery_interaction(run_id: &str, agent_id: &str) -> Suspension {
    Suspension::new(
        recovery_target_id(run_id),
        AGENT_RECOVERY_INTERACTION_ACTION,
    )
    .with_message(format!(
        "Detected interrupted run '{run_id}' (agent '{agent_id}'). Resume now?",
    ))
    .with_parameters(json!({
        "run_id": run_id,
        "agent_id": agent_id,
        "background": false
    }))
    .with_response_schema(json!({
        "type": "boolean"
    }))
}

pub(super) fn has_suspended_recovery_interaction(state: &Value) -> bool {
    suspended_calls_from_state(state)
        .values()
        .any(|call| call.ticket.suspension.action == AGENT_RECOVERY_INTERACTION_ACTION)
}

pub(super) fn current_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
}
