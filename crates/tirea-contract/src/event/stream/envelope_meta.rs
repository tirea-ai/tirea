use super::event::{AgentEvent, AgentEventType};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeEnvelopeMeta {
    pub event_type: AgentEventType,
    pub run_id: String,
    pub thread_id: String,
    pub seq: u64,
    pub timestamp_ms: u64,
    pub step_id: Option<String>,
}

const RUNTIME_ENVELOPE_META_MAX_BUFFERED: usize = 4096;
static RUNTIME_ENVELOPE_META_BUFFER: OnceLock<Mutex<VecDeque<RuntimeEnvelopeMeta>>> =
    OnceLock::new();

thread_local! {
    static SERIALIZE_RUN_HINT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

fn runtime_envelope_meta_buffer() -> &'static Mutex<VecDeque<RuntimeEnvelopeMeta>> {
    RUNTIME_ENVELOPE_META_BUFFER.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// Register runtime envelope metadata for one emitted event.
pub fn register_runtime_event_envelope_meta(
    event: &AgentEvent,
    run_id: &str,
    thread_id: &str,
    seq: u64,
    timestamp_ms: u64,
    step_id: Option<String>,
) {
    let entry = RuntimeEnvelopeMeta {
        event_type: event.event_type(),
        run_id: run_id.to_string(),
        thread_id: thread_id.to_string(),
        seq,
        timestamp_ms,
        step_id,
    };
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");
    buffer.push_back(entry);
    if buffer.len() > RUNTIME_ENVELOPE_META_MAX_BUFFERED {
        let overflow = buffer.len() - RUNTIME_ENVELOPE_META_MAX_BUFFERED;
        buffer.drain(0..overflow);
    }
}

/// Clear all buffered runtime envelope metadata used during event serialization.
pub fn clear_runtime_event_envelope_meta() {
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");
    buffer.clear();
    SERIALIZE_RUN_HINT.with(|hint| {
        *hint.borrow_mut() = None;
    });
}

pub(crate) fn take_runtime_event_envelope_meta(
    event_type: AgentEventType,
    run_hint: Option<(&str, &str)>,
) -> Option<RuntimeEnvelopeMeta> {
    let mut buffer = runtime_envelope_meta_buffer()
        .lock()
        .expect("runtime envelope meta buffer mutex poisoned");

    if let Some((run_id, thread_id)) = run_hint {
        if let Some(pos) = buffer.iter().position(|entry| {
            entry.event_type == event_type && entry.run_id == run_id && entry.thread_id == thread_id
        }) {
            return buffer.remove(pos);
        }
    }

    if let Some(pos) = buffer
        .iter()
        .position(|entry| entry.event_type == event_type)
    {
        return buffer.remove(pos);
    }

    None
}

pub(crate) fn serialize_run_hint() -> Option<(String, String)> {
    SERIALIZE_RUN_HINT.with(|hint| hint.borrow().clone())
}

pub(crate) fn set_serialize_run_hint(run_id: String, thread_id: String) {
    SERIALIZE_RUN_HINT.with(|current| {
        *current.borrow_mut() = Some((run_id, thread_id));
    });
}

pub(crate) fn clear_serialize_run_hint_if_matches(run_id: &str, thread_id: &str) {
    SERIALIZE_RUN_HINT.with(|current| {
        let mut current = current.borrow_mut();
        let should_clear = current
            .as_ref()
            .map(|active| active.0 == run_id && active.1 == thread_id)
            .unwrap_or(true);
        if should_clear {
            *current = None;
        }
    });
}
