/// Pure stream-loop helpers:
/// - read suspended-call state
/// - derive result payloads / preallocated message ids
///
/// No plugin execution, tool execution, or event emission happens here.
use super::*;
use std::collections::HashMap;

pub(super) fn preallocate_tool_result_message_ids(
    results: &[ToolExecutionResult],
) -> HashMap<String, String> {
    results
        .iter()
        .filter(|result| !matches!(result.outcome, crate::contracts::ToolCallOutcome::Suspended))
        .map(|result| (result.execution.call.id.clone(), gen_message_id()))
        .collect()
}
