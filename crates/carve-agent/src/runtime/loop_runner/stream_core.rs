use super::*;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Pure stream-loop helpers:
/// - derive run identity
/// - read pending interaction from persisted state
/// - derive result payloads / preallocated message ids
///
/// No plugin execution, tool execution, or event emission happens here.
pub(super) struct StreamRunIdentity {
    pub(super) run_id: String,
    pub(super) parent_run_id: Option<String>,
}

pub(super) fn resolve_stream_run_identity(thread: &mut AgentState) -> StreamRunIdentity {
    let run_id = thread
        .scope
        .value("run_id")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| {
            let id = uuid_v7();
            let _ = thread.scope.set("run_id", &id);
            id
        });
    let parent_run_id = thread
        .scope
        .value("parent_run_id")
        .and_then(|v| v.as_str().map(String::from));
    StreamRunIdentity {
        run_id,
        parent_run_id,
    }
}

pub(super) fn natural_result_payload(text: &str) -> Option<Value> {
    if text.is_empty() {
        None
    } else {
        Some(json!({ "response": text }))
    }
}

pub(super) fn preallocate_tool_result_message_ids(
    results: &[ToolExecutionResult],
) -> HashMap<String, String> {
    results
        .iter()
        .filter(|result| result.pending_interaction.is_none())
        .map(|result| (result.execution.call.id.clone(), gen_message_id()))
        .collect()
}
