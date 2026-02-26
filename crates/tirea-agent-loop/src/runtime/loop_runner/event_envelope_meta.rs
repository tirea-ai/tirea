use crate::contracts::AgentEvent;

pub(super) fn register_runtime_event_envelope_meta(
    event: &AgentEvent,
    run_id: &str,
    thread_id: &str,
    seq: u64,
    timestamp_ms: u64,
    step_id: Option<String>,
) {
    crate::contracts::io::event::stream::internal::register_runtime_event_envelope_meta(
        event,
        run_id,
        thread_id,
        seq,
        timestamp_ms,
        step_id,
    );
}
