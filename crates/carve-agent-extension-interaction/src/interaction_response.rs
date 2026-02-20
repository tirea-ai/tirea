//! Interaction Response Plugin.
//!
//! Handles client responses to pending interactions (approvals/denials).

use super::{INTERACTION_RESPONSE_PLUGIN_ID, RECOVERY_RESUME_TOOL_ID};
use crate::outbox::InteractionOutbox;
use crate::{AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX};
use carve_agent_contract::runtime::control::LoopControlState;
use async_trait::async_trait;
use carve_agent_contract::plugin::AgentPlugin;
use carve_agent_contract::plugin::phase::{Phase, StepContext};
use carve_state::{Op, Patch, Path, State, TrackedPatch};
use carve_agent_contract::event::interaction::{
    FrontendToolInvocation, InvocationOrigin, ResponseRouting,
};
use carve_agent_contract::{Interaction, InteractionResponse};
use serde_json::json;
use std::collections::HashMap;

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
        let state = step.snapshot();
        state
            .get(LoopControlState::PATH)
            .and_then(|agent| agent.get("pending_interaction"))
            .cloned()
            .and_then(|v| serde_json::from_value::<Interaction>(v).ok())
    }

    fn persisted_pending_interaction(step: &StepContext<'_>) -> Option<Interaction> {
        Self::pending_interaction_from_step_thread(step).or_else(|| {
            let agent = step.state_of::<LoopControlState>();
            agent.pending_interaction().ok().flatten()
        })
    }

    fn persisted_frontend_invocation(step: &StepContext<'_>) -> Option<FrontendToolInvocation> {
        let state = step.snapshot();
        state
            .get(LoopControlState::PATH)
            .and_then(|lc| lc.get("pending_frontend_invocation"))
            .cloned()
            .and_then(|v| serde_json::from_value::<FrontendToolInvocation>(v).ok())
    }

    fn push_resolution(step: &StepContext<'_>, interaction_id: String, result: serde_json::Value) {
        let outbox = step.ctx().state_of::<InteractionOutbox>();
        outbox.interaction_resolutions_push(InteractionResponse::new(interaction_id, result));
    }

    fn queue_replay_call(step: &StepContext<'_>, call: carve_agent_contract::thread::ToolCall) {
        let outbox = step.ctx().state_of::<InteractionOutbox>();
        outbox.replay_tool_calls_push(call);
    }

    /// During RunStart, detect pending_interaction and schedule tool replay if approved.
    fn on_run_start(&self, step: &mut StepContext<'_>) {
        let Some(pending) = Self::persisted_pending_interaction(step) else {
            return;
        };

        // Try the first-class FrontendToolInvocation path first.
        let invocation = Self::persisted_frontend_invocation(step);
        let pending_id_owned = if let Some(ref inv) = invocation {
            inv.call_id.clone()
        } else {
            pending.id.clone()
        };
        let pending_id = pending_id_owned.as_str();

        if self.is_denied(pending_id) {
            step.state_of::<LoopControlState>().pending_interaction_none();
            step.state_of::<LoopControlState>()
                .pending_frontend_invocation_none();
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

        // Route based on FrontendToolInvocation if available.
        if let Some(inv) = invocation {
            self.route_frontend_invocation(step, &inv);
            return;
        }

        // Legacy fallback: route based on Interaction parameters.
        self.route_legacy_interaction(step, &pending, pending_id);
    }

    /// Route an approved response using the first-class `FrontendToolInvocation` model.
    fn route_frontend_invocation(
        &self,
        step: &mut StepContext<'_>,
        inv: &FrontendToolInvocation,
    ) {
        match &inv.routing {
            ResponseRouting::ReplayOriginalTool { state_patches } => {
                // Apply pre-replay state patches (e.g. permission → allow).
                if !state_patches.is_empty() {
                    let patch =
                        TrackedPatch::new(Patch::with_ops(state_patches.clone()))
                            .with_source("interaction_response");
                    step.pending_patches.push(patch);
                }
                // Queue replay of the original backend tool.
                match &inv.origin {
                    InvocationOrigin::ToolCallIntercepted {
                        backend_call_id,
                        backend_tool_name,
                        backend_arguments,
                    } => {
                        let replay_call = carve_agent_contract::thread::ToolCall::new(
                            backend_call_id.clone(),
                            backend_tool_name.clone(),
                            backend_arguments.clone(),
                        );
                        Self::queue_replay_call(step, replay_call);
                    }
                    InvocationOrigin::PluginInitiated { .. } => {
                        // PluginInitiated with ReplayOriginalTool is unusual but
                        // fallback to replaying the frontend tool itself.
                        let replay_call = carve_agent_contract::thread::ToolCall::new(
                            inv.call_id.clone(),
                            inv.tool_name.clone(),
                            inv.arguments.clone(),
                        );
                        Self::queue_replay_call(step, replay_call);
                    }
                }
            }
            ResponseRouting::UseAsToolResult => {
                // The frontend result is the tool result. Replay the tool call
                // so the result enters LLM message history.
                let replay_call = carve_agent_contract::thread::ToolCall::new(
                    inv.call_id.clone(),
                    inv.tool_name.clone(),
                    inv.arguments.clone(),
                );
                Self::queue_replay_call(step, replay_call);
            }
            ResponseRouting::PassToLLM => {
                // Future: pass the result to LLM as an independent message.
                // For now, fallback to replay.
                let replay_call = carve_agent_contract::thread::ToolCall::new(
                    inv.call_id.clone(),
                    inv.tool_name.clone(),
                    inv.arguments.clone(),
                );
                Self::queue_replay_call(step, replay_call);
            }
        }
    }

    /// Legacy routing based on `Interaction` parameters (backward compat).
    fn route_legacy_interaction(
        &self,
        step: &mut StepContext<'_>,
        pending: &Interaction,
        pending_id: &str,
    ) {
        // When a permission interaction is approved, update the permission state
        // so that the replayed tool execution sees Allow and doesn't re-trigger Ask.
        if pending.parameters.get("source").and_then(|v| v.as_str()) == Some("permission") {
            if let Some(tool_name) = pending.action.strip_prefix("tool:") {
                let patch = TrackedPatch::new(Patch::with_ops(vec![Op::set(
                    Path::root().key("permissions").key("tools").key(tool_name),
                    json!("allow"),
                )]))
                .with_source("interaction_response");
                step.pending_patches.push(patch);
            }
        }

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
                step.state_of::<LoopControlState>().pending_interaction_none();
                return;
            };

            let replay_call = carve_agent_contract::thread::ToolCall::new(
                format!("recovery_resume_{run_id}"),
                RECOVERY_RESUME_TOOL_ID,
                json!({
                    "run_id": run_id,
                    "background": false
                }),
            );
            Self::queue_replay_call(step, replay_call);
            return;
        }

        if let Some(replay_call) = pending
            .parameters
            .get("origin_tool_call")
            .cloned()
            .and_then(|v| {
                serde_json::from_value::<carve_agent_contract::thread::ToolCall>(v).ok()
            })
        {
            Self::queue_replay_call(step, replay_call);
            return;
        }

        if let Some(replay_call) =
            pending.parameters.get("tool_call").cloned().and_then(|v| {
                serde_json::from_value::<carve_agent_contract::thread::ToolCall>(v).ok()
            })
        {
            Self::queue_replay_call(step, replay_call);
            return;
        }

        // Unified: both FrontendTool and Permission interactions use tool_call_id
        // as interaction id and "tool:<name>" as action.
        if let Some(tool_name) = pending.action.strip_prefix("tool:") {
            let replay_call = carve_agent_contract::thread::ToolCall::new(
                pending_id.to_string(),
                tool_name,
                pending.parameters.clone(),
            );
            Self::queue_replay_call(step, replay_call);
            return;
        }

        // Fallback: find the pending tool call from message history by tool_call_id.
        let tool_call = step
            .messages()
            .iter()
            .rev()
            .find(|m| {
                m.role == carve_agent_contract::thread::Role::Assistant
                    && m.tool_calls.is_some()
            })
            .and_then(|m| m.tool_calls.as_ref())
            .and_then(|calls| {
                calls
                    .iter()
                    .find(|c| c.id.as_str() == pending_id)
                    .cloned()
            });

        if let Some(call) = tool_call {
            Self::queue_replay_call(step, call);
        }
    }
}

#[async_trait]
impl AgentPlugin for InteractionResponsePlugin {
    fn id(&self) -> &str {
        INTERACTION_RESPONSE_PLUGIN_ID
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        match phase {
            Phase::RunStart => {
                self.on_run_start(step);
                return;
            }
            Phase::BeforeToolExecute => {}
            _ => return,
        }

        // Check if there's a tool context
        let Some(tool) = step.tool.as_ref() else {
            return;
        };

        // Check both the tool call ID and the frontend invocation call_id.
        // For direct frontend tools, interaction_id == tool.id.
        // For indirect (permission), the frontend invocation has a different call_id.
        let interaction_id = tool.id.clone();
        let frontend_call_id = Self::persisted_frontend_invocation(step)
            .map(|inv| inv.call_id);

        // The client may respond with either the tool call ID or the frontend call ID.
        let effective_id = if let Some(ref fc_id) = frontend_call_id {
            if self.is_approved(fc_id) || self.is_denied(fc_id) {
                fc_id.clone()
            } else {
                interaction_id.clone()
            }
        } else {
            interaction_id.clone()
        };

        let is_approved = self.is_approved(&effective_id);
        let is_denied = self.is_denied(&effective_id);

        if !is_approved && !is_denied {
            return;
        }

        // Verify that the server actually has a persisted pending interaction whose ID
        // matches the one the client claims to be responding to.  Without this check a
        // malicious client could pre-approve arbitrary tool calls by injecting approved
        // IDs in a fresh request that has no outstanding pending interaction.
        let persisted_id = Self::persisted_pending_interaction(step).map(|i| i.id);

        let id_matches = persisted_id
            .as_deref()
            .is_some_and(|id| id == interaction_id || Some(id) == frontend_call_id.as_deref());

        if !id_matches {
            return;
        }

        if is_denied {
            step.confirm();
            step.block("User denied the action".to_string());
            step.state_of::<LoopControlState>().pending_interaction_none();
            step.state_of::<LoopControlState>()
                .pending_frontend_invocation_none();
            let resolved_id = persisted_id.unwrap_or(effective_id);
            Self::push_resolution(step, resolved_id, serde_json::Value::Bool(false));
        } else if is_approved {
            step.confirm();
            step.state_of::<LoopControlState>().pending_interaction_none();
            step.state_of::<LoopControlState>()
                .pending_frontend_invocation_none();
            let resolved_id = persisted_id.unwrap_or(effective_id);
            Self::push_resolution(step, resolved_id, serde_json::Value::Bool(true));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agent_contract::thread::{Message, ToolCall};
    use carve_agent_contract::testing::TestFixture;
    use carve_state::DocCell;
    use serde_json::json;
    use std::sync::Arc;

    fn replay_calls_from_state(state: &serde_json::Value) -> Vec<ToolCall> {
        state
            .get("interaction_outbox")
            .and_then(|agent| agent.get("replay_tool_calls"))
            .cloned()
            .and_then(|v| serde_json::from_value::<Vec<ToolCall>>(v).ok())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn run_start_replays_tool_matching_pending_interaction() {
        // Unified format: id = tool_call_id, action = "tool:<name>"
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_write",
                    "action": "tool:write_file",
                    "parameters": {
                        "source": "permission",
                        "origin_tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "b.txt" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture {
            doc: DocCell::new(state),
            messages: vec![Arc::new(Message::assistant_with_tool_calls(
                "tools",
                vec![
                    ToolCall::new("call_read", "read_file", json!({"path": "a.txt"})),
                    ToolCall::new("call_write", "write_file", json!({"path": "b.txt"})),
                ],
            ))],
            ..TestFixture::new()
        };
        let plugin =
            InteractionResponsePlugin::new(vec!["call_write".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");

        // Permission state patch should be emitted
        assert!(!step.pending_patches.is_empty(), "permission state patch should be emitted");
    }

    #[tokio::test]
    async fn run_start_replay_does_not_require_prior_intent_channel() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_write",
                    "action": "tool:write_file",
                    "parameters": {
                        "source": "permission",
                        "origin_tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "b.txt" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture {
            doc: DocCell::new(state),
            messages: vec![Arc::new(Message::assistant_with_tool_calls(
                "tools",
                vec![ToolCall::new(
                    "call_write",
                    "write_file",
                    json!({"path": "b.txt"}),
                )],
            ))],
            ..TestFixture::new()
        };
        let plugin =
            InteractionResponsePlugin::new(vec!["call_write".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
    }

    #[tokio::test]
    async fn run_start_frontend_interaction_replay_works_without_prior_channel() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_copy_1",
                    "action": "tool:copyToClipboard"
                }
            }
        });
        let fixture = TestFixture {
            doc: DocCell::new(state),
            messages: vec![Arc::new(Message::assistant_with_tool_calls(
                "tools",
                vec![
                    ToolCall::new("call_search_1", "search", json!({"query": "x"})),
                    ToolCall::new("call_copy_1", "copyToClipboard", json!({"text": "hello"})),
                ],
            ))],
            ..TestFixture::new()
        };
        let plugin = InteractionResponsePlugin::new(vec!["call_copy_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
    }

    #[tokio::test]
    async fn run_start_frontend_interaction_replay_without_history_uses_pending_payload() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_copy_1",
                    "action": "tool:copyToClipboard",
                    "parameters": { "text": "hello" }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionResponsePlugin::new(vec!["call_copy_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
        assert_eq!(replay_after[0].arguments["text"], "hello");
    }

    #[tokio::test]
    async fn run_start_permission_replay_without_history_uses_embedded_tool_call() {
        // Unified format with origin_tool_call embedded in parameters
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_write",
                    "action": "tool:write_file",
                    "parameters": {
                        "source": "permission",
                        "origin_tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "a.txt" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin =
            InteractionResponsePlugin::new(vec!["call_write".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "a.txt");
    }

    #[tokio::test]
    async fn run_start_permission_replay_prefers_origin_tool_call_mapping() {
        // origin_tool_call is checked before tool:<name> fallback
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_write",
                    "action": "tool:write_file",
                    "parameters": {
                        "source": "permission",
                        "origin_tool_call": {
                            "id": "call_write",
                            "name": "write_file",
                            "arguments": { "path": "b.txt" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin =
            InteractionResponsePlugin::new(vec!["call_write".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "b.txt");
    }

    #[tokio::test]
    async fn run_start_routes_via_frontend_invocation_replay_original_tool() {
        use carve_agent_contract::event::interaction::{
            FrontendToolInvocation, InvocationOrigin, ResponseRouting,
        };

        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_write",
                    "action": "tool:write_file",
                    "parameters": {}
                },
                "pending_frontend_invocation": {
                    "call_id": "fc_ask_1",
                    "tool_name": "PermissionConfirm",
                    "arguments": { "tool_name": "write_file", "tool_args": { "path": "a.txt" } },
                    "origin": {
                        "type": "tool_call_intercepted",
                        "backend_call_id": "call_write",
                        "backend_tool_name": "write_file",
                        "backend_arguments": { "path": "a.txt" }
                    },
                    "routing": {
                        "strategy": "replay_original_tool",
                        "state_patches": [{
                            "op": "set",
                            "path": ["permissions", "tools", "write_file"],
                            "value": "allow"
                        }]
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        // Client responds with the frontend call_id
        let plugin = InteractionResponsePlugin::new(vec!["fc_ask_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        // Should replay the original backend tool, not the frontend tool
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");
        assert_eq!(replay_calls[0].arguments["path"], "a.txt");

        // State patches should be applied
        assert!(!step.pending_patches.is_empty());
    }

    #[tokio::test]
    async fn run_start_routes_via_frontend_invocation_use_as_tool_result() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "call_copy",
                    "action": "tool:copyToClipboard",
                    "parameters": { "text": "hello" }
                },
                "pending_frontend_invocation": {
                    "call_id": "call_copy",
                    "tool_name": "copyToClipboard",
                    "arguments": { "text": "hello" },
                    "origin": {
                        "type": "plugin_initiated",
                        "plugin_id": "agui_frontend_tools"
                    },
                    "routing": {
                        "strategy": "use_as_tool_result"
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionResponsePlugin::new(vec!["call_copy".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_copy");
        assert_eq!(replay_calls[0].name, "copyToClipboard");
        assert_eq!(replay_calls[0].arguments["text"], "hello");

        // No state patches for UseAsToolResult
        assert!(step.pending_patches.is_empty());
    }

    #[tokio::test]
    async fn run_start_recovery_approval_schedules_agent_run_replay() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "agent_recovery_run-1",
                    "action": "recover_agent_run",
                    "parameters": {
                        "run_id": "run-1"
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin =
            InteractionResponsePlugin::new(vec!["agent_recovery_run-1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].name, RECOVERY_RESUME_TOOL_ID);
        assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
        assert_eq!(replay_calls[0].arguments["background"], false);
    }

    #[tokio::test]
    async fn run_start_recovery_denial_clears_pending_interaction() {
        let state = json!({
            "loop_control": {
                "pending_interaction": {
                    "id": "agent_recovery_run-1",
                    "action": "recover_agent_run",
                    "parameters": {
                        "run_id": "run-1"
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin =
            InteractionResponsePlugin::new(vec![], vec!["agent_recovery_run-1".to_string()]);

        let mut step = fixture.step(vec![]);
        plugin.on_phase(Phase::RunStart, &mut step).await;

        assert!(
            fixture.has_changes(),
            "denied recovery must clear pending interaction state"
        );

        let updated = fixture.updated_state();
        let pending = updated
            .get("loop_control")
            .and_then(|a| a.get("pending_interaction"));
        assert!(pending.is_none() || pending == Some(&serde_json::Value::Null));
    }
}
