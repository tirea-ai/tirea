use serde_json::{json, Value};
use tirea_contract::runtime::phase::SuspendTicket;
use tirea_contract::runtime::{PendingToolCall, ToolCallResumeMode};
use tirea_contract::Suspension;

/// Frontend tool name for permission confirmation prompts.
pub const PERMISSION_CONFIRM_TOOL_NAME: &str = "PermissionConfirm";
const PERMISSION_CONFIRM_ACTION: &str = "tool:PermissionConfirm";

fn permission_message(tool_id: &str) -> String {
    format!("Permission required to run tool '{tool_id}'")
}

fn permission_response_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {
            "approved": { "type": "boolean" },
            "reason": { "type": "string" }
        },
        "required": ["approved"]
    })
}

/// Build the default tool-like permission confirmation form.
#[must_use]
pub fn permission_confirmation_ticket(
    call_id: &str,
    tool_id: &str,
    tool_args: Value,
) -> SuspendTicket {
    let pending_call_id = format!("fc_{call_id}");
    let arguments = json!({
        "tool_name": tool_id,
        "tool_args": tool_args,
    });
    let suspension = Suspension::new(&pending_call_id, PERMISSION_CONFIRM_ACTION)
        .with_message(permission_message(tool_id))
        .with_parameters(arguments.clone())
        .with_response_schema(permission_response_schema());

    SuspendTicket::new(
        suspension,
        PendingToolCall::new(pending_call_id, PERMISSION_CONFIRM_TOOL_NAME, arguments),
        ToolCallResumeMode::ReplayToolCall,
    )
}
