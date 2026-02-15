//! Interaction Response Plugin.
//!
//! Handles client responses to pending interactions (approvals/denials).

use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::state_types::{
    Interaction, InteractionResponse, AGENT_RECOVERY_INTERACTION_ACTION,
    AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_STATE_PATH,
};
use async_trait::async_trait;
use carve_state::Context;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

pub(crate) const INTERACTION_RESOLUTIONS_KEY: &str = "__interaction_resolutions";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct InteractionResolution {
    pub(crate) interaction_id: String,
    pub(crate) result: serde_json::Value,
}

/// Plugin that handles interaction responses from client.
///
/// This plugin works with `FrontendToolPlugin` and `PermissionPlugin` to complete
/// the interaction flow:
///
/// 1. A plugin (e.g., PermissionPlugin) creates a pending interaction
/// 2. Agent emits `AgentEvent::Pending` which becomes protocol tool-call events
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
pub(crate) struct InteractionResponsePlugin {
    /// Interaction responses keyed by interaction ID.
    responses: HashMap<String, serde_json::Value>,
}

impl InteractionResponsePlugin {
    /// Create a new plugin with approved and denied interaction IDs.
    pub(crate) fn new(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        let mut responses = HashMap::new();
        for id in approved_ids {
            responses.insert(id, serde_json::Value::Bool(true));
        }
        for id in denied_ids {
            responses.insert(id, serde_json::Value::Bool(false));
        }
        Self { responses }
    }

    /// Create from explicit interaction response payloads.
    pub(crate) fn from_responses(responses: Vec<InteractionResponse>) -> Self {
        Self {
            responses: responses
                .into_iter()
                .map(|r| (r.interaction_id, r.result))
                .collect(),
        }
    }

    /// Return a raw response payload for an interaction id.
    pub(crate) fn result_for(&self, interaction_id: &str) -> Option<&serde_json::Value> {
        self.responses.get(interaction_id)
    }

    /// Return all configured responses.
    pub(crate) fn responses(&self) -> Vec<InteractionResponse> {
        self.responses
            .iter()
            .map(|(interaction_id, result)| {
                InteractionResponse::new(interaction_id.clone(), result.clone())
            })
            .collect()
    }

    /// Check if an interaction was approved.
    pub(crate) fn is_approved(&self, interaction_id: &str) -> bool {
        self.result_for(interaction_id)
            .map(InteractionResponse::is_approved)
            .unwrap_or(false)
    }

    /// Check if an interaction was denied.
    pub(crate) fn is_denied(&self, interaction_id: &str) -> bool {
        self.result_for(interaction_id)
            .map(InteractionResponse::is_denied)
            .unwrap_or(false)
    }

    /// Check if plugin has any responses to process.
    pub(crate) fn has_responses(&self) -> bool {
        !self.responses.is_empty()
    }

    fn pending_interaction_from_step_thread(step: &StepContext<'_>) -> Option<Interaction> {
        let state = step.thread.rebuild_state().ok()?;
        state
            .get(AGENT_STATE_PATH)
            .and_then(|agent| agent.get("pending_interaction"))
            .cloned()
            .and_then(|v| serde_json::from_value::<Interaction>(v).ok())
    }

    fn persisted_pending_interaction(
        step: &StepContext<'_>,
        ctx: &Context<'_>,
    ) -> Option<Interaction> {
        Self::pending_interaction_from_step_thread(step).or_else(|| {
            use crate::state_types::AgentState;
            ctx.state::<AgentState>(AGENT_STATE_PATH)
                .pending_interaction()
                .ok()
                .flatten()
        })
    }

    fn push_resolution(
        step: &mut StepContext<'_>,
        interaction_id: String,
        result: serde_json::Value,
    ) {
        let mut entries: Vec<InteractionResolution> = step
            .scratchpad_get(INTERACTION_RESOLUTIONS_KEY)
            .unwrap_or_default();
        entries.push(InteractionResolution {
            interaction_id,
            result,
        });
        let _ = step.scratchpad_set(INTERACTION_RESOLUTIONS_KEY, entries);
    }

    /// During SessionStart, detect pending_interaction and schedule tool replay if approved.
    fn on_session_start(&self, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        let agent = ctx.state::<crate::state_types::AgentState>(AGENT_STATE_PATH);
        let Some(pending) = Self::persisted_pending_interaction(step, ctx) else {
            return;
        };
        let pending_id_owned = pending.id.clone();
        let pending_id = pending_id_owned.as_str();

        if self.is_denied(pending_id) {
            agent.pending_interaction_none();
            Self::push_resolution(
                step,
                pending_id_owned.clone(),
                self.result_for(pending_id)
                    .cloned()
                    .unwrap_or(serde_json::Value::Bool(false)),
            );
            return;
        }

        // Check if the client approved this interaction.
        let is_approved = self.is_approved(pending_id);
        if !is_approved {
            return;
        }
        Self::push_resolution(
            step,
            pending_id_owned.clone(),
            self.result_for(pending_id)
                .cloned()
                .unwrap_or(serde_json::Value::Bool(true)),
        );

        if pending.action == AGENT_RECOVERY_INTERACTION_ACTION {
            let run_id = pending
                .parameters
                .get("run_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .or_else(|| {
                    pending_id
                        .strip_prefix(AGENT_RECOVERY_INTERACTION_PREFIX)
                        .map(str::to_string)
                });
            let Some(run_id) = run_id else {
                agent.pending_interaction_none();
                return;
            };

            let replay_call = crate::types::ToolCall::new(
                format!("recovery_resume_{run_id}"),
                "agent_run",
                json!({
                    "run_id": run_id,
                    "background": false
                }),
            );
            step.scratchpad_set("__replay_tool_calls", vec![replay_call]);
            return;
        }

        if let Some(replay_call) = pending
            .parameters
            .get("origin_tool_call")
            .cloned()
            .and_then(|v| serde_json::from_value::<crate::types::ToolCall>(v).ok())
        {
            step.scratchpad_set("__replay_tool_calls", vec![replay_call]);
            return;
        }

        if let Some(replay_call) = pending
            .parameters
            .get("tool_call")
            .cloned()
            .and_then(|v| serde_json::from_value::<crate::types::ToolCall>(v).ok())
        {
            step.scratchpad_set("__replay_tool_calls", vec![replay_call]);
            return;
        }

        if !pending_id_owned.starts_with("permission_") {
            if let Some(tool_name) = pending.action.strip_prefix("tool:") {
                let replay_call = crate::types::ToolCall::new(
                    pending_id_owned.clone(),
                    tool_name,
                    pending.parameters.clone(),
                );
                step.scratchpad_set("__replay_tool_calls", vec![replay_call]);
                return;
            }
        }

        // Find the pending tool call from the last assistant message with tool_calls.
        let tool_call = step
            .thread
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::types::Role::Assistant && m.tool_calls.is_some())
            .and_then(|m| m.tool_calls.as_ref())
            .and_then(|calls| {
                // Frontend tool interactions use tool call id as interaction id.
                if let Some(call) = calls
                    .iter()
                    .find(|c| c.id.as_str() == pending_id_owned.as_str())
                {
                    return Some(call.clone());
                }

                // Permission interactions use `permission_<tool_name>`.
                if let Some(tool_name) = pending_id_owned.strip_prefix("permission_") {
                    if let Some(call) = calls.iter().find(|c| c.name == tool_name) {
                        return Some(call.clone());
                    }
                }

                None
            });

        if let Some(call) = tool_call {
            // Schedule the tool call for replay by the loop.
            step.scratchpad_set(
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        match phase {
            Phase::SessionStart => {
                self.on_session_start(step, ctx);
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

        // Check if any of these interactions were approved/denied
        let is_frontend_approved = self.is_approved(&frontend_interaction_id);
        let is_permission_approved = self.is_approved(&permission_interaction_id);
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
        let agent = ctx.state::<crate::state_types::AgentState>(AGENT_STATE_PATH);
        let persisted_id = Self::persisted_pending_interaction(step, ctx).map(|i| i.id);

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
            agent.pending_interaction_none();
            let resolved_id = persisted_id.unwrap_or(permission_interaction_id);
            Self::push_resolution(step, resolved_id, serde_json::Value::Bool(false));
        } else if is_frontend_approved || is_permission_approved {
            // Interaction was approved - clear any pending state
            // This allows the tool to execute normally.
            step.confirm();
            agent.pending_interaction_none();
            let resolved_id = persisted_id.unwrap_or(permission_interaction_id);
            Self::push_resolution(step, resolved_id, serde_json::Value::Bool(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::Thread;
    use crate::types::{Message, ToolCall};
    use carve_state::{apply_patches, Context};
    use serde_json::json;

    #[tokio::test]
    async fn session_start_replays_tool_matching_pending_interaction() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "confirm"
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec!["permission_write_file".to_string()], vec![]);

        let thread = Thread::with_initial_state(
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

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_calls: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");
    }

    #[tokio::test]
    async fn session_start_replay_does_not_require_prior_intent_scratchpad() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "confirm"
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec!["permission_write_file".to_string()], vec![]);

        let thread = Thread::with_initial_state(
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
            vec![ToolCall::new(
                "call_write",
                "write_file",
                json!({"path": "b.txt"}),
            )],
        ));

        // Simulate a brand-new run: no previous run scratchpad keys.
        let mut step = StepContext::new(&thread, vec![]);
        let intents: Vec<serde_json::Value> = step
            .scratchpad_get("__interaction_intents")
            .unwrap_or_default();
        assert!(
            intents.is_empty(),
            "new run should not carry previous run intent scratchpad"
        );
        let replay_before: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert!(replay_before.is_empty());

        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_after: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
    }

    #[tokio::test]
    async fn session_start_frontend_interaction_replay_works_without_prior_scratchpad() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "call_copy_1",
                    "action": "tool:copyToClipboard"
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin = InteractionResponsePlugin::new(vec!["call_copy_1".to_string()], vec![]);

        let thread = Thread::with_initial_state(
            "s1",
            json!({
                "agent": {
                    "pending_interaction": {
                        "id": "call_copy_1",
                        "action": "tool:copyToClipboard"
                    }
                }
            }),
        )
        .with_message(Message::assistant_with_tool_calls(
            "tools",
            vec![
                ToolCall::new("call_search_1", "search", json!({"query": "x"})),
                ToolCall::new("call_copy_1", "copyToClipboard", json!({"text": "hello"})),
            ],
        ));

        let mut step = StepContext::new(&thread, vec![]);
        let replay_before: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert!(replay_before.is_empty());

        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_after: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
    }

    #[tokio::test]
    async fn session_start_frontend_interaction_replay_without_history_uses_pending_payload() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "call_copy_1",
                    "action": "tool:copyToClipboard",
                    "parameters": { "text": "hello" }
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin = InteractionResponsePlugin::new(vec!["call_copy_1".to_string()], vec![]);
        let thread = Thread::with_initial_state("s1", doc.clone());

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_after: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
        assert_eq!(replay_after[0].arguments["text"], "hello");
    }

    #[tokio::test]
    async fn session_start_permission_replay_without_history_uses_embedded_tool_call() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "confirm",
                    "parameters": {
                        "tool_id": "write_file",
                        "tool_call_id": "call_write",
                        "tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "a.txt" }
                        }
                    }
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec!["permission_write_file".to_string()], vec![]);
        let thread = Thread::with_initial_state("s1", doc.clone());

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_after: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "a.txt");
    }

    #[tokio::test]
    async fn session_start_permission_replay_prefers_origin_tool_call_mapping() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "tool:AskUserQuestion",
                    "parameters": {
                        "origin_call_id": "call_write",
                        "origin_call_name": "write_file",
                        "origin_tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "b.txt" }
                        }
                    }
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec!["permission_write_file".to_string()], vec![]);
        let thread = Thread::with_initial_state("s1", doc.clone());

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_after: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "b.txt");
    }

    #[tokio::test]
    async fn session_start_recovery_approval_schedules_agent_run_replay() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "agent_recovery_run-1",
                    "action": "recover_agent_run",
                    "parameters": {
                        "run_id": "run-1"
                    }
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec!["agent_recovery_run-1".to_string()], vec![]);

        let thread = Thread::with_initial_state(
            "s1",
            json!({
                "agent": {
                    "pending_interaction": {
                        "id": "agent_recovery_run-1",
                        "action": "recover_agent_run",
                        "parameters": {
                            "run_id": "run-1"
                        }
                    }
                }
            }),
        );

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay_calls: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].name, "agent_run");
        assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
        assert_eq!(replay_calls[0].arguments["background"], false);
    }

    #[tokio::test]
    async fn session_start_recovery_denial_clears_pending_interaction() {
        let doc = json!({
            "agent": {
                "pending_interaction": {
                    "id": "agent_recovery_run-1",
                    "action": "recover_agent_run",
                    "parameters": {
                        "run_id": "run-1"
                    }
                }
            }
        });
        let ctx = Context::new(&doc, "test", "test");
        let plugin =
            InteractionResponsePlugin::new(vec![], vec!["agent_recovery_run-1".to_string()]);

        let thread = Thread::with_initial_state(
            "s1",
            json!({
                "agent": {
                    "pending_interaction": {
                        "id": "agent_recovery_run-1",
                        "action": "recover_agent_run",
                        "parameters": {
                            "run_id": "run-1"
                        }
                    }
                }
            }),
        );

        let mut step = StepContext::new(&thread, vec![]);
        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        // Plugin ops are collected in ctx; flush them
        assert!(
            ctx.has_changes(),
            "denied recovery must clear pending interaction state"
        );
        let ctx_patch = ctx.take_patch();

        let updated = apply_patches(
            &thread.rebuild_state().unwrap(),
            std::iter::once(ctx_patch.patch()),
        )
        .expect("pending patch should apply");
        let pending = updated
            .get("agent")
            .and_then(|a| a.get("pending_interaction"));
        assert!(pending.is_none() || pending == Some(&serde_json::Value::Null));
    }
}
