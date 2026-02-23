//! Unified interaction plugin.
//!
//! Handles client interaction responses (approvals/denials) and maps them to
//! runtime resume decisions for suspended tool calls.

use super::INTERACTION_PLUGIN_ID;
use crate::outbox::ResumeDecisionsState;
use crate::AGENT_RECOVERY_INTERACTION_ACTION;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tirea_contract::event::suspension::ResponseRouting;
use tirea_contract::event::termination::TerminationReason;
use tirea_contract::plugin::phase::{
    BeforeInferenceContext, BeforeToolExecuteContext, PluginPhaseContext, RunStartContext,
};
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::runtime::control::{
    InferenceError, InferenceErrorState, ResumeDecision, ResumeDecisionAction,
    SuspendedToolCallsState,
};
use tirea_contract::runtime::state_paths::SUSPENDED_TOOL_CALLS_STATE_PATH;
use tirea_contract::{SuspensionResponse, SuspendedCall};

/// Unified interaction plugin.
///
/// This plugin is strategy-agnostic: it applies interaction responses from the
/// inbound request to suspended tool calls.
pub struct InteractionPlugin {
    /// Suspension responses keyed by interaction ID.
    responses: HashMap<String, serde_json::Value>,
    /// Guards the `before_inference` check so it only runs on the first call per run.
    /// On the first BeforeInference, we check for unresolved suspended interactions
    /// from a previous run. On subsequent calls, the pending was created during this
    /// run's tool execution — let the LLM continue with completed results (HOL fix).
    first_inference_checked: AtomicBool,
}

impl InteractionPlugin {
    /// Build plugin from interaction responses.
    pub fn new(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        let mut responses = HashMap::new();
        for id in approved_ids {
            responses.insert(id, serde_json::Value::Bool(true));
        }
        for id in denied_ids {
            responses.insert(id, serde_json::Value::Bool(false));
        }
        Self {
            responses,
            first_inference_checked: AtomicBool::new(false),
        }
    }

    /// Build combined plugin from interaction responses only.
    pub fn with_responses(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self::new(approved_ids, denied_ids)
    }

    /// Build plugin from explicit interaction response payloads.
    pub fn from_interaction_responses(responses: Vec<SuspensionResponse>) -> Self {
        Self {
            responses: responses
                .into_iter()
                .map(|r| (r.target_id, r.result))
                .collect(),
            first_inference_checked: AtomicBool::new(false),
        }
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.has_responses()
    }

    /// Whether any interaction responses are present.
    pub fn has_responses(&self) -> bool {
        !self.responses.is_empty()
    }

    /// Return a raw response payload for an interaction id.
    fn result_for(&self, target_id: &str) -> Option<&serde_json::Value> {
        self.responses.get(target_id)
    }

    /// Check if an interaction ID is approved.
    pub fn is_approved(&self, target_id: &str) -> bool {
        self.result_for(target_id)
            .map(SuspensionResponse::is_approved)
            .unwrap_or(false)
    }

    /// Check if an interaction ID is denied.
    pub fn is_denied(&self, target_id: &str) -> bool {
        self.result_for(target_id)
            .map(SuspensionResponse::is_denied)
            .unwrap_or(false)
    }

    fn suspended_calls_from_step_thread(
        step: &impl PluginPhaseContext,
    ) -> HashMap<String, SuspendedCall> {
        let state = step.snapshot();
        state
            .get(SUSPENDED_TOOL_CALLS_STATE_PATH)
            .and_then(|agent| agent.get("calls"))
            .cloned()
            .and_then(|v| serde_json::from_value::<HashMap<String, SuspendedCall>>(v).ok())
            .unwrap_or_default()
    }

    fn persisted_suspended_calls(step: &impl PluginPhaseContext) -> HashMap<String, SuspendedCall> {
        let from_step = Self::suspended_calls_from_step_thread(step);
        if !from_step.is_empty() {
            return from_step;
        }
        let state = step.state_of::<SuspendedToolCallsState>();
        state.calls().ok().unwrap_or_default()
    }

    fn upsert_resume_decision(
        step: &impl PluginPhaseContext,
        call_id: String,
        decision: ResumeDecision,
    ) -> Result<(), String> {
        let mailbox = step.state_of::<ResumeDecisionsState>();
        let mut calls = mailbox.calls().ok().unwrap_or_default();
        calls.insert(call_id, decision);
        mailbox
            .set_calls(calls)
            .map_err(|err| format!("failed to persist resume decision: {err}"))
    }

    fn current_unix_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis().min(u128::from(u64::MAX)) as u64)
    }

    fn resolve_response_key(&self, call: &SuspendedCall) -> Option<String> {
        let frontend_id = call.invocation.as_ref().map(|inv| inv.call_id.as_str());
        [
            frontend_id,
            Some(call.suspension.id.as_str()),
            Some(call.call_id.as_str()),
        ]
        .into_iter()
        .flatten()
        .find(|id| self.result_for(id).is_some())
        .map(str::to_string)
    }

    fn has_response_for_call(&self, call: &SuspendedCall) -> bool {
        self.resolve_response_key(call).is_some()
    }

    fn report_run_start_error(step: &impl PluginPhaseContext, message: impl Into<String>) {
        let message = message.into();
        tracing::error!(
            plugin = INTERACTION_PLUGIN_ID,
            error = %message,
            "interaction run_start handling failed"
        );

        let state = step.state_of::<InferenceErrorState>();
        if let Err(err) = state.set_error(Some(InferenceError {
            error_type: "interaction_response_error".to_string(),
            message,
        })) {
            tracing::error!(
                plugin = INTERACTION_PLUGIN_ID,
                error = %err,
                "failed to persist interaction response error"
            );
        }
    }

    /// During RunStart, resolve suspended calls and schedule replay when needed.
    fn on_run_start(&self, step: &mut RunStartContext<'_, '_>) {
        let suspended_calls = Self::persisted_suspended_calls(step);
        if suspended_calls.is_empty() {
            return;
        }
        let mut call_ids: Vec<String> = suspended_calls.keys().cloned().collect();
        call_ids.sort();
        for call_id in call_ids {
            let Some(call) = suspended_calls.get(&call_id).cloned() else {
                continue;
            };
            let Some(response_key) = self.resolve_response_key(&call) else {
                continue;
            };
            let result_payload = self.result_for(&response_key).cloned();
            let is_denied = result_payload
                .as_ref()
                .is_some_and(SuspensionResponse::is_denied);
            let is_approved = result_payload
                .as_ref()
                .is_some_and(SuspensionResponse::is_approved);
            let resolution_value = result_payload
                .clone()
                .unwrap_or(serde_json::Value::Bool(!is_denied));

            if call.suspension.action == AGENT_RECOVERY_INTERACTION_ACTION
                && !is_denied
                && !is_approved
            {
                continue;
            }

            let should_continue_use_as_result = result_payload.is_some()
                && call.invocation.as_ref().is_some_and(|inv| {
                    matches!(
                        inv.routing,
                        ResponseRouting::UseAsToolResult | ResponseRouting::PassToLLM
                    )
                });
            let permission_source_without_channel = call.invocation.is_none()
                && call
                    .suspension
                    .parameters
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|source| source == "permission");
            if permission_source_without_channel && !is_denied {
                continue;
            }
            if !is_denied && !is_approved && !should_continue_use_as_result {
                continue;
            }

            let reason = if is_denied {
                result_payload
                    .as_ref()
                    .and_then(|payload| {
                        payload
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .or_else(|| payload.get("message").and_then(serde_json::Value::as_str))
                    })
                    .map(str::to_string)
            } else {
                None
            };
            let decision = ResumeDecision {
                decision_id: response_key,
                action: if is_denied {
                    ResumeDecisionAction::Cancel
                } else {
                    ResumeDecisionAction::Resume
                },
                result: resolution_value,
                reason,
                updated_at: Self::current_unix_millis(),
            };
            if let Err(err) = Self::upsert_resume_decision(step, call.call_id.clone(), decision) {
                Self::report_run_start_error(step, err);
                return;
            }
        }
    }
}

impl Default for InteractionPlugin {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

#[async_trait]
impl AgentPlugin for InteractionPlugin {
    fn id(&self) -> &str {
        INTERACTION_PLUGIN_ID
    }

    async fn run_start(&self, ctx: &mut RunStartContext<'_, '_>) {
        self.on_run_start(ctx);
    }

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        // Only check on the first BeforeInference of the run.
        // On subsequent calls, the pending was created during this run's tool
        // execution — let the LLM continue with completed results (HOL fix).
        if self.first_inference_checked.swap(true, Ordering::Relaxed) {
            return;
        }

        let suspended_calls = Self::persisted_suspended_calls(ctx);
        if suspended_calls.is_empty() {
            return;
        }
        let has_unresolved = suspended_calls
            .values()
            .any(|call| !self.has_response_for_call(call));
        if has_unresolved {
            ctx.request_termination(TerminationReason::Suspended);
        }
    }

    async fn before_tool_execute(&self, _step: &mut BeforeToolExecuteContext<'_, '_>) {
        // Decision application is handled by the loop-level resume mailbox.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RECOVERY_RESUME_TOOL_ID;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;
    use tirea_contract::plugin::phase::{
        AfterInferenceContext, AfterToolExecuteContext, BeforeInferenceContext,
        BeforeToolExecuteContext, Phase, RunEndContext, RunStartContext, StepContext,
        StepEndContext, StepStartContext,
    };
    use tirea_contract::plugin::AgentPlugin;
    use tirea_contract::runtime::state_paths::{
        RESUME_DECISIONS_STATE_PATH, SUSPENDED_TOOL_CALLS_STATE_PATH,
    };
    use tirea_contract::runtime::{ResumeDecision, ResumeDecisionAction};
    use tirea_contract::testing::TestFixture;
    use tirea_contract::thread::{Message, ToolCall};
    use tirea_contract::SuspendedCall;
    use tirea_state::DocCell;

    #[async_trait]
    trait AgentPluginTestDispatch {
        async fn run_phase(&self, phase: Phase, step: &mut StepContext<'_>);
    }

    #[async_trait]
    impl<T> AgentPluginTestDispatch for T
    where
        T: AgentPlugin + ?Sized,
    {
        async fn run_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
            match phase {
                Phase::RunStart => {
                    let mut ctx = RunStartContext::new(step);
                    self.run_start(&mut ctx).await;
                }
                Phase::StepStart => {
                    let mut ctx = StepStartContext::new(step);
                    self.step_start(&mut ctx).await;
                }
                Phase::BeforeInference => {
                    let mut ctx = BeforeInferenceContext::new(step);
                    self.before_inference(&mut ctx).await;
                }
                Phase::AfterInference => {
                    let mut ctx = AfterInferenceContext::new(step);
                    self.after_inference(&mut ctx).await;
                }
                Phase::BeforeToolExecute => {
                    let mut ctx = BeforeToolExecuteContext::new(step);
                    self.before_tool_execute(&mut ctx).await;
                }
                Phase::AfterToolExecute => {
                    let mut ctx = AfterToolExecuteContext::new(step);
                    self.after_tool_execute(&mut ctx).await;
                }
                Phase::StepEnd => {
                    let mut ctx = StepEndContext::new(step);
                    self.step_end(&mut ctx).await;
                }
                Phase::RunEnd => {
                    let mut ctx = RunEndContext::new(step);
                    self.run_end(&mut ctx).await;
                }
            }
        }
    }

    fn replay_calls_from_state(state: &serde_json::Value) -> Vec<ToolCall> {
        let decisions = resume_decisions_from_state(state);
        if decisions.is_empty() {
            return Vec::new();
        }
        let suspended = state
            .get(SUSPENDED_TOOL_CALLS_STATE_PATH)
            .and_then(|agent| agent.get("calls"))
            .cloned()
            .and_then(|v| serde_json::from_value::<HashMap<String, SuspendedCall>>(v).ok())
            .unwrap_or_default();

        let mut call_ids: Vec<String> = decisions.keys().cloned().collect();
        call_ids.sort();
        call_ids
            .into_iter()
            .filter_map(|call_id| {
                let decision = decisions.get(&call_id)?;
                if !matches!(decision.action, ResumeDecisionAction::Resume) {
                    return None;
                }
                let call = suspended.get(&call_id)?;
                if let Some(inv) = call.invocation.as_ref() {
                    return match &inv.routing {
                        ResponseRouting::ReplayOriginalTool => match &inv.origin {
                            tirea_contract::InvocationOrigin::ToolCallIntercepted {
                                backend_call_id,
                                backend_tool_name,
                                backend_arguments,
                            } => Some(ToolCall::new(
                                backend_call_id.clone(),
                                backend_tool_name.clone(),
                                backend_arguments.clone(),
                            )),
                            tirea_contract::InvocationOrigin::PluginInitiated { .. } => {
                                Some(ToolCall::new(
                                    inv.call_id.clone(),
                                    inv.tool_name.clone(),
                                    inv.arguments.clone(),
                                ))
                            }
                        },
                        ResponseRouting::UseAsToolResult | ResponseRouting::PassToLLM => {
                            let args = if decision.result.is_null()
                                || matches!(decision.result, serde_json::Value::Bool(_))
                            {
                                inv.arguments.clone()
                            } else {
                                decision.result.clone()
                            };
                            Some(ToolCall::new(
                                inv.call_id.clone(),
                                inv.tool_name.clone(),
                                args,
                            ))
                        }
                    };
                }
                Some(ToolCall::new(
                    call.call_id.clone(),
                    call.tool_name.clone(),
                    call.suspension.parameters.clone(),
                ))
            })
            .collect()
    }

    fn resume_decisions_from_state(state: &serde_json::Value) -> HashMap<String, ResumeDecision> {
        state
            .get(RESUME_DECISIONS_STATE_PATH)
            .and_then(|agent| agent.get("calls"))
            .cloned()
            .and_then(|v| serde_json::from_value::<HashMap<String, ResumeDecision>>(v).ok())
            .unwrap_or_default()
    }

    fn interaction_resolutions_from_state(state: &serde_json::Value) -> Vec<SuspensionResponse> {
        let decisions = resume_decisions_from_state(state);
        if decisions.is_empty() {
            return Vec::new();
        }
        let suspended = state
            .get(SUSPENDED_TOOL_CALLS_STATE_PATH)
            .and_then(|agent| agent.get("calls"))
            .cloned()
            .and_then(|v| serde_json::from_value::<HashMap<String, SuspendedCall>>(v).ok())
            .unwrap_or_default();
        let mut call_ids: Vec<String> = decisions.keys().cloned().collect();
        call_ids.sort();
        call_ids
            .into_iter()
            .filter_map(|call_id| {
                let decision = decisions.get(&call_id)?;
                let target_id = suspended
                    .get(&call_id)
                    .map(|call| call.suspension.id.clone())
                    .unwrap_or(call_id.clone());
                let result = if decision.result.is_null() {
                    serde_json::Value::Bool(matches!(decision.action, ResumeDecisionAction::Resume))
                } else {
                    decision.result.clone()
                };
                Some(SuspensionResponse::new(target_id, result))
            })
            .collect()
    }

    #[test]
    fn plugin_inactive_without_responses() {
        let plugin = InteractionPlugin::new(Vec::new(), Vec::new());
        assert!(!plugin.is_active());
    }

    #[test]
    fn plugin_active_with_responses() {
        let plugin = InteractionPlugin::new(vec!["call_1".to_string()], Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_responses());
    }

    #[tokio::test]
    async fn run_start_replays_tool_matching_suspended_call() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_write": {
                        "call_id": "call_write",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "fc_ask_1",
                            "action": "tool:write_file",
                            "parameters": {
                                "source": "permission"
                            }
                        },
                        "invocation": {
                            "call_id": "fc_ask_1",
                            "tool_name": "PermissionConfirm",
                            "arguments": { "tool_name": "write_file", "tool_args": { "path": "b.txt" } },
                            "origin": {
                                "type": "tool_call_intercepted",
                                "backend_call_id": "call_write",
                                "backend_tool_name": "write_file",
                                "backend_arguments": { "path": "b.txt" }
                            },
                            "routing": {
                                "strategy": "replay_original_tool"
                            }
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
        let plugin = InteractionPlugin::new(vec!["fc_ask_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");

        let decisions = resume_decisions_from_state(&updated);
        assert!(
            decisions.contains_key("call_write"),
            "decision rendezvous should be persisted for call_write"
        );
    }

    #[tokio::test]
    async fn run_start_replay_requires_invocation_channel() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_write": {
                        "call_id": "call_write",
                        "tool_name": "write_file",
                        "suspension": {
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
        let plugin = InteractionPlugin::new(vec!["call_write".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert!(
            replay_after.is_empty(),
            "without invocation metadata, replay must not happen"
        );
    }

    #[tokio::test]
    async fn run_start_frontend_interaction_replay_works_without_prior_channel() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_copy_1": {
                        "call_id": "call_copy_1",
                        "tool_name": "copyToClipboard",
                        "suspension": {
                            "id": "call_copy_1",
                            "action": "tool:copyToClipboard"
                        },
                        "invocation": {
                            "call_id": "call_copy_1",
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
        let plugin = InteractionPlugin::new(vec!["call_copy_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
    }

    #[tokio::test]
    async fn run_start_frontend_interaction_replay_without_history_uses_suspended_payload() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_copy_1": {
                        "call_id": "call_copy_1",
                        "tool_name": "copyToClipboard",
                        "suspension": {
                            "id": "call_copy_1",
                            "action": "tool:copyToClipboard",
                            "parameters": { "text": "hello" }
                        },
                        "invocation": {
                            "call_id": "call_copy_1",
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
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec!["call_copy_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_copy_1");
        assert_eq!(replay_after[0].name, "copyToClipboard");
        assert_eq!(replay_after[0].arguments["text"], "hello");
    }

    #[tokio::test]
    async fn run_start_permission_replay_without_history_uses_embedded_tool_call() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_write": {
                        "call_id": "call_write",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "fc_ask_2",
                            "action": "tool:write_file",
                            "parameters": {
                                "source": "permission"
                            }
                        },
                        "invocation": {
                            "call_id": "fc_ask_2",
                            "tool_name": "PermissionConfirm",
                            "arguments": { "tool_name": "write_file", "tool_args": { "path": "a.txt" } },
                            "origin": {
                                "type": "tool_call_intercepted",
                                "backend_call_id": "call_write",
                                "backend_tool_name": "write_file",
                                "backend_arguments": { "path": "a.txt" }
                            },
                            "routing": {
                                "strategy": "replay_original_tool"
                            }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec!["fc_ask_2".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "a.txt");
    }

    #[tokio::test]
    async fn run_start_permission_replay_prefers_origin_tool_call_mapping() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_write": {
                        "call_id": "call_write",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "fc_ask_3",
                            "action": "tool:write_file",
                            "parameters": {
                                "source": "permission"
                            }
                        },
                        "invocation": {
                            "call_id": "fc_ask_3",
                            "tool_name": "PermissionConfirm",
                            "arguments": { "tool_name": "write_file", "tool_args": { "path": "b.txt" } },
                            "origin": {
                                "type": "tool_call_intercepted",
                                "backend_call_id": "call_write",
                                "backend_tool_name": "write_file",
                                "backend_arguments": { "path": "b.txt" }
                            },
                            "routing": {
                                "strategy": "replay_original_tool"
                            }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec!["fc_ask_3".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_after = replay_calls_from_state(&updated);
        assert_eq!(replay_after.len(), 1);
        assert_eq!(replay_after[0].id, "call_write");
        assert_eq!(replay_after[0].name, "write_file");
        assert_eq!(replay_after[0].arguments["path"], "b.txt");
    }

    #[tokio::test]
    async fn run_start_routes_via_invocation_replay_original_tool() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_write": {
                        "call_id": "call_write",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "call_write",
                            "action": "tool:write_file",
                            "parameters": {}
                        },
                        "invocation": {
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
                                "strategy": "replay_original_tool"
                            }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        // Client responds with the frontend call_id
        let plugin = InteractionPlugin::new(vec!["fc_ask_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        // Should replay the original backend tool, not the frontend tool
        assert_eq!(replay_calls[0].id, "call_write");
        assert_eq!(replay_calls[0].name, "write_file");
        assert_eq!(replay_calls[0].arguments["path"], "a.txt");

        let decisions = resume_decisions_from_state(&updated);
        assert!(
            decisions.contains_key("call_write"),
            "decision rendezvous should be persisted for call_write"
        );
    }

    #[tokio::test]
    async fn run_start_routes_via_invocation_use_as_tool_result() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_copy": {
                        "call_id": "call_copy",
                        "tool_name": "copyToClipboard",
                        "suspension": {
                            "id": "call_copy",
                            "action": "tool:copyToClipboard",
                            "parameters": { "text": "hello" }
                        },
                        "invocation": {
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
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec!["call_copy".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

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
    async fn run_start_use_as_tool_result_preserves_non_boolean_payload() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "call_copy": {
                        "call_id": "call_copy",
                        "tool_name": "copyToClipboard",
                        "suspension": {
                            "id": "call_copy",
                            "action": "tool:copyToClipboard",
                            "parameters": { "text": "hello" }
                        },
                        "invocation": {
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
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::from_interaction_responses(vec![SuspensionResponse::new(
            "call_copy",
            json!({
                "ok": true,
                "copied": "hello"
            }),
        )]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].id, "call_copy");
        assert_eq!(replay_calls[0].name, "copyToClipboard");
        assert_eq!(replay_calls[0].arguments["ok"], true);
        assert_eq!(replay_calls[0].arguments["copied"], "hello");

        let resolutions = interaction_resolutions_from_state(&updated);
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].target_id, "call_copy");
        assert_eq!(resolutions[0].result["ok"], true);
        assert_eq!(resolutions[0].result["copied"], "hello");
    }

    #[tokio::test]
    async fn run_start_recovery_approval_schedules_agent_run_replay() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "agent_recovery_run-1": {
                        "call_id": "agent_recovery_run-1",
                        "tool_name": "agent_run",
                        "suspension": {
                            "id": "agent_recovery_run-1",
                            "action": "recover_agent_run",
                            "parameters": {
                                "run_id": "run-1",
                                "background": false
                            }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec!["agent_recovery_run-1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let replay_calls = replay_calls_from_state(&updated);
        assert_eq!(replay_calls.len(), 1);
        assert_eq!(replay_calls[0].name, RECOVERY_RESUME_TOOL_ID);
        assert_eq!(replay_calls[0].arguments["run_id"], "run-1");
        assert_eq!(replay_calls[0].arguments["background"], false);
    }

    #[tokio::test]
    async fn run_start_recovery_denial_queues_cancel_and_keeps_suspended_call() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "agent_recovery_run-1": {
                        "call_id": "agent_recovery_run-1",
                        "tool_name": "agent_run",
                        "suspension": {
                            "id": "agent_recovery_run-1",
                            "action": "recover_agent_run",
                            "parameters": {
                                "run_id": "run-1",
                                "background": false
                            }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec![], vec!["agent_recovery_run-1".to_string()]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::RunStart, &mut step).await;

        let updated = fixture.updated_state();
        let decisions = resume_decisions_from_state(&updated);
        assert!(
            decisions
                .get("agent_recovery_run-1")
                .is_some_and(|decision| matches!(decision.action, ResumeDecisionAction::Cancel)),
            "denied recovery should queue a cancel decision"
        );
        let suspended = updated
            .get("__suspended_tool_calls")
            .and_then(|a| a.get("calls"))
            .and_then(|v| v.as_object());
        assert!(
            suspended.is_some_and(|calls| calls.contains_key("agent_recovery_run-1")),
            "suspended recovery call should remain until loop applies decision"
        );
    }

    // =========================================================================
    // before_inference tests (plugin-driven termination)
    // =========================================================================

    #[tokio::test]
    async fn before_inference_terminates_on_unresolved_suspended_call() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "fc_ask_1": {
                        "call_id": "fc_ask_1",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "fc_ask_1",
                            "action": "tool:write_file",
                            "parameters": { "source": "permission" }
                        },
                        "invocation": {
                            "call_id": "fc_ask_1",
                            "tool_name": "PermissionConfirm",
                            "arguments": { "tool_name": "write_file" },
                            "origin": { "type": "plugin_initiated", "plugin_id": "test" },
                            "routing": { "strategy": "replay_original_tool" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        // No responses → the pending is unresolved.
        let plugin = InteractionPlugin::new(vec![], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::BeforeInference, &mut step).await;

        assert!(
            step.skip_inference,
            "skip_inference should be set when terminating for unresolved pending"
        );
        assert_eq!(
            step.termination_request,
            Some(TerminationReason::Suspended),
            "should request Suspended termination"
        );
    }

    #[tokio::test]
    async fn before_inference_skips_when_response_provided() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "fc_ask_1": {
                        "call_id": "fc_ask_1",
                        "tool_name": "write_file",
                        "suspension": {
                            "id": "fc_ask_1",
                            "action": "tool:write_file",
                            "parameters": { "source": "permission" }
                        },
                        "invocation": {
                            "call_id": "fc_ask_1",
                            "tool_name": "PermissionConfirm",
                            "arguments": { "tool_name": "write_file" },
                            "origin": { "type": "plugin_initiated", "plugin_id": "test" },
                            "routing": { "strategy": "replay_original_tool" }
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        // Response provided for the frontend call_id → RunStart handled it.
        let plugin = InteractionPlugin::new(vec!["fc_ask_1".to_string()], vec![]);

        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::BeforeInference, &mut step).await;

        assert!(
            !step.skip_inference,
            "skip_inference should NOT be set when response was provided"
        );
        assert!(
            step.termination_request.is_none(),
            "should NOT request termination when response was provided"
        );
    }

    #[tokio::test]
    async fn before_inference_only_checks_first_call() {
        let state = json!({
            "__suspended_tool_calls": {
                "calls": {
                    "confirm_1": {
                        "call_id": "confirm_1",
                        "tool_name": "confirm",
                        "suspension": {
                            "id": "confirm_1",
                            "action": "confirm",
                            "parameters": {}
                        }
                    }
                }
            }
        });
        let fixture = TestFixture::new_with_state(state);
        let plugin = InteractionPlugin::new(vec![], vec![]);

        // First call: should detect pending and request termination.
        let mut step = fixture.step(vec![]);
        plugin.run_phase(Phase::BeforeInference, &mut step).await;
        assert_eq!(
            step.termination_request,
            Some(TerminationReason::Suspended),
            "first call should request termination"
        );

        // Second call: should be a no-op (HOL fix behavior).
        let mut step2 = fixture.step(vec![]);
        plugin.run_phase(Phase::BeforeInference, &mut step2).await;
        assert!(
            step2.termination_request.is_none(),
            "second call should NOT request termination (HOL fix)"
        );
        assert!(
            !step2.skip_inference,
            "second call should NOT set skip_inference"
        );
    }
}
