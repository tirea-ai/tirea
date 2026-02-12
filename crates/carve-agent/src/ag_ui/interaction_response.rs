//! AG-UI Interaction Response Plugin.
//!
//! Handles client responses to pending interactions (approvals/denials).

use super::RunAgentRequest;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use std::collections::HashSet;

/// Plugin that handles interaction responses from client.
///
/// This plugin works with `FrontendToolPlugin` and `PermissionPlugin` to complete
/// the interaction flow:
///
/// 1. A plugin (e.g., PermissionPlugin) creates a pending interaction
/// 2. Agent emits `AgentEvent::Pending` which becomes AG-UI tool call events
/// 3. Client responds with a new request containing tool message(s)
/// 4. This plugin checks if the response approves/denies the pending interaction
/// 5. Based on response, tool execution proceeds or is blocked
///
/// # Usage
///
/// ```ignore
/// // Create plugin with approved interaction IDs from client request
/// let approved_ids = request.approved_interaction_ids();
/// let denied_ids = request.denied_interaction_ids();
/// let plugin = InteractionResponsePlugin::new(approved_ids, denied_ids);
///
/// let config = config.with_plugin(Arc::new(plugin));
/// ```
pub struct InteractionResponsePlugin {
    /// Interaction IDs that were approved by the client.
    approved_ids: HashSet<String>,
    /// Interaction IDs that were denied by the client.
    denied_ids: HashSet<String>,
}

impl InteractionResponsePlugin {
    /// Create a new plugin with approved and denied interaction IDs.
    pub fn new(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self {
            approved_ids: approved_ids.into_iter().collect(),
            denied_ids: denied_ids.into_iter().collect(),
        }
    }

    /// Create plugin from a RunAgentRequest.
    pub fn from_request(request: &RunAgentRequest) -> Self {
        Self::new(
            request.approved_interaction_ids(),
            request.denied_interaction_ids(),
        )
    }

    /// Check if an interaction was approved.
    pub fn is_approved(&self, interaction_id: &str) -> bool {
        self.approved_ids.contains(interaction_id)
    }

    /// Check if an interaction was denied.
    pub fn is_denied(&self, interaction_id: &str) -> bool {
        self.denied_ids.contains(interaction_id)
    }

    /// Check if plugin has any responses to process.
    pub fn has_responses(&self) -> bool {
        !self.approved_ids.is_empty() || !self.denied_ids.is_empty()
    }

    /// During SessionStart, detect pending_interaction and schedule tool replay if approved.
    fn on_session_start(&self, step: &mut StepContext<'_>) {
        use crate::state_types::AGENT_STATE_PATH;

        let Ok(state) = step.session.rebuild_state() else {
            return;
        };

        // Check if there's a persisted pending_interaction.
        let pending_id = state
            .get(AGENT_STATE_PATH)
            .and_then(|a| a.get("pending_interaction"))
            .and_then(|p| p.get("id"))
            .and_then(|id| id.as_str());

        let Some(pending_id) = pending_id else {
            return;
        };

        // Check if the client approved this interaction.
        let is_approved = self.approved_ids.iter().any(|id| id == pending_id);
        if !is_approved {
            return;
        }

        // Find the pending tool call from the last assistant message with tool_calls.
        let tool_call = step
            .session
            .messages
            .iter()
            .rev()
            .find(|m| {
                m.role == crate::types::Role::Assistant && m.tool_calls.is_some()
            })
            .and_then(|m| m.tool_calls.as_ref())
            .and_then(|calls| {
                // Frontend tool interactions use tool call id as interaction id.
                if let Some(call) = calls.iter().find(|c| c.id == pending_id) {
                    return Some(call.clone());
                }

                // Permission interactions use `permission_<tool_name>`.
                if let Some(tool_name) = pending_id.strip_prefix("permission_") {
                    if let Some(call) = calls.iter().find(|c| c.name == tool_name) {
                        return Some(call.clone());
                    }
                }

                // Fallback for unknown interaction id formats.
                calls.first().cloned()
            });

        if let Some(call) = tool_call {
            // Schedule the tool call for replay by the loop.
            step.set(
                "__replay_tool_calls",
                serde_json::to_value(vec![call]).unwrap_or_default(),
            );
        }
    }
}

#[async_trait]
impl AgentPlugin for InteractionResponsePlugin {
    fn id(&self) -> &str {
        "interaction_response"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        match phase {
            Phase::SessionStart => {
                self.on_session_start(step);
                return;
            }
            Phase::BeforeToolExecute => {}
            _ => return,
        }

        // Check if there's a tool context
        let Some(tool) = step.tool.as_ref() else {
            return;
        };

        // Generate possible interaction IDs for this tool call
        // These match the IDs generated by FrontendToolPlugin and PermissionPlugin
        let frontend_interaction_id = tool.id.clone(); // FrontendToolPlugin uses tool call ID
        let permission_interaction_id = format!("permission_{}", tool.name); // PermissionPlugin format

        // Check if any of these interactions were approved
        let is_frontend_approved = self.is_approved(&frontend_interaction_id);
        let is_permission_approved = self.is_approved(&permission_interaction_id);

        // Check if any of these interactions were denied
        let is_frontend_denied = self.is_denied(&frontend_interaction_id);
        let is_permission_denied = self.is_denied(&permission_interaction_id);

        // Only act if the client is responding to an interaction we actually match.
        let has_response = is_frontend_approved
            || is_permission_approved
            || is_frontend_denied
            || is_permission_denied;
        if !has_response {
            return;
        }

        // Verify that the server actually has a persisted pending interaction whose ID
        // matches one of the IDs the client claims to be responding to.  Without this
        // check a malicious client could pre-approve arbitrary tool names by injecting
        // approved IDs in a fresh request that has no outstanding pending interaction.
        let persisted_id = step
            .session
            .rebuild_state()
            .ok()
            .and_then(|s| {
                s.get(crate::state_types::AGENT_STATE_PATH)?
                    .get("pending_interaction")?
                    .get("id")?
                    .as_str()
                    .map(String::from)
            });

        let id_matches = persisted_id
            .as_deref()
            .map(|id| id == frontend_interaction_id || id == permission_interaction_id)
            .unwrap_or(false);

        if !id_matches {
            // No matching persisted pending interaction — ignore the client's response.
            return;
        }

        if is_frontend_denied || is_permission_denied {
            // Interaction was denied - block the tool
            step.confirm();
            step.block("User denied the action".to_string());
            clear_agent_pending_interaction(step);
        } else if is_frontend_approved || is_permission_approved {
            // Interaction was approved - clear any pending state
            // This allows the tool to execute normally.
            step.confirm();
            clear_agent_pending_interaction(step);
        }
    }
}

fn clear_agent_pending_interaction(step: &mut StepContext<'_>) {
    use crate::state_types::{AgentState, AGENT_STATE_PATH};
    use carve_state::Context;

    let Ok(state) = step.session.rebuild_state() else {
        return;
    };

    // Persist the clearance via patch so subsequent steps/runs don't remain stuck in Pending.
    let ctx = Context::new(&state, "agent_state", "interaction_response");
    let agent = ctx.state::<AgentState>(AGENT_STATE_PATH);
    agent.pending_interaction_none();
    let patch = ctx.take_patch();
    if !patch.patch().is_empty() {
        step.pending_patches.push(patch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use crate::types::{Message, ToolCall};
    use serde_json::json;

    #[tokio::test]
    async fn session_start_replays_tool_matching_pending_interaction() {
        let plugin = InteractionResponsePlugin::new(
            vec!["permission_write_file".to_string()],
            vec![],
        );

        let session = Session::with_initial_state(
            "s1",
            json!({
                "agent": {
                    "pending_interaction": {
                        "id": "permission_write_file",
                        "action": "confirm"
                    }
                }
            }),
        )
        .with_message(Message::assistant_with_tool_calls(
            "tools",
            vec![
                ToolCall::new("call_read", "read_file", json!({"path": "a.txt"})),
                ToolCall::new("call_write", "write_file", json!({"path": "b.txt"})),
            ],
        ));

        let mut step = StepContext::new(&session, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step).await;

        let replay_calls: Vec<ToolCall> = step.get("__replay_tool_calls").unwrap_or_default();
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");
    }
}
