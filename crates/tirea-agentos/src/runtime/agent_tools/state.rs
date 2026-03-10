use super::*;
use tirea_contract::runtime::suspended_calls_from_state;

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
