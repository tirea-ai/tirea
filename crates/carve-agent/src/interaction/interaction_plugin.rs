//! Unified interaction mechanism plugin.
//!
//! This plugin is strategy-agnostic: it provides a shared lifecycle/channel
//! for interaction intents and responses, while concrete strategies (frontend
//! tools, permissions, recovery, etc.) emit intents independently.

use super::frontend_tool::FrontendToolPlugin;
use super::interaction_response::InteractionResponsePlugin;
use super::take_intents;
use super::InteractionIntent;
use super::{RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY, RUNTIME_INTERACTION_RESPONSES_KEY};
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use crate::state_types::InteractionResponse;
use async_trait::async_trait;
use carve_state::Context;
use std::collections::HashSet;

/// Unified interaction mechanism plugin.
pub struct InteractionPlugin {
    static_response: InteractionResponsePlugin,
    static_frontend_tools: HashSet<String>,
}

impl InteractionPlugin {
    /// Build combined plugin from explicit frontend tools and responses.
    pub fn new(
        frontend_tools: HashSet<String>,
        approved_ids: Vec<String>,
        denied_ids: Vec<String>,
    ) -> Self {
        Self {
            static_response: InteractionResponsePlugin::new(approved_ids, denied_ids),
            static_frontend_tools: frontend_tools,
        }
    }

    /// Build combined plugin from frontend tools only.
    pub fn with_frontend_tools(frontend_tools: HashSet<String>) -> Self {
        Self::new(frontend_tools, Vec::new(), Vec::new())
    }

    /// Build combined plugin from interaction responses only.
    pub fn with_responses(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self::new(HashSet::new(), approved_ids, denied_ids)
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.static_response.has_responses() || !self.static_frontend_tools.is_empty()
    }

    /// Whether request contains frontend tools and therefore needs tool stubs.
    pub fn has_frontend_tools(&self) -> bool {
        !self.static_frontend_tools.is_empty()
    }

    /// Whether any interaction responses are present.
    pub fn has_responses(&self) -> bool {
        self.static_response.has_responses()
    }

    /// Check if a specific tool is configured as a frontend tool.
    pub fn is_frontend_tool(&self, tool_name: &str) -> bool {
        self.static_frontend_tools.contains(tool_name)
    }

    /// Check if an interaction ID is approved.
    pub fn is_approved(&self, interaction_id: &str) -> bool {
        self.static_response.is_approved(interaction_id)
    }

    /// Check if an interaction ID is denied.
    pub fn is_denied(&self, interaction_id: &str) -> bool {
        self.static_response.is_denied(interaction_id)
    }

    fn runtime_frontend_tools(ctx: &Context<'_>) -> HashSet<String> {
        ctx.runtime_value(RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
            .collect()
    }

    fn runtime_responses(ctx: &Context<'_>) -> Vec<InteractionResponse> {
        let Some(raw) = ctx
            .runtime_value(RUNTIME_INTERACTION_RESPONSES_KEY)
            .cloned()
        else {
            return Vec::new();
        };
        serde_json::from_value::<Vec<InteractionResponse>>(raw).unwrap_or_default()
    }

    fn merged_frontend_plugin(&self, ctx: &Context<'_>) -> FrontendToolPlugin {
        let mut frontend_tools = self.static_frontend_tools.clone();
        frontend_tools.extend(Self::runtime_frontend_tools(ctx));
        FrontendToolPlugin::new(frontend_tools)
    }

    fn merged_response_plugin(&self, ctx: &Context<'_>) -> InteractionResponsePlugin {
        let mut merged: std::collections::HashMap<String, serde_json::Value> = self
            .static_response
            .responses()
            .into_iter()
            .map(|r| (r.interaction_id, r.result))
            .collect();
        for r in Self::runtime_responses(ctx) {
            merged.insert(r.interaction_id, r.result);
        }
        InteractionResponsePlugin::from_responses(
            merged
                .into_iter()
                .map(|(interaction_id, result)| InteractionResponse::new(interaction_id, result))
                .collect(),
        )
    }
}

impl Default for InteractionPlugin {
    fn default() -> Self {
        Self::new(HashSet::new(), Vec::new(), Vec::new())
    }
}

#[async_trait]
impl AgentPlugin for InteractionPlugin {
    fn id(&self) -> &str {
        "interaction"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        let response = self.merged_response_plugin(ctx);
        let frontend = self.merged_frontend_plugin(ctx);
        response.on_phase(phase, step, ctx).await;
        frontend.on_phase(phase, step, ctx).await;
        if phase != Phase::BeforeToolExecute {
            return;
        }

        if step.tool_blocked() || step.tool_pending() {
            let _ = take_intents(step);
            return;
        }

        let intents = take_intents(step);
        if intents.is_empty() {
            return;
        }

        if let Some(reason) = intents.iter().find_map(|intent| match intent {
            InteractionIntent::Block { reason } => Some(reason.clone()),
            _ => None,
        }) {
            step.block(reason);
            return;
        }

        if let Some(interaction) = intents.into_iter().find_map(|intent| match intent {
            InteractionIntent::Pending { interaction } => Some(interaction),
            _ => None,
        }) {
            step.pending(interaction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{push_block_intent, push_pending_intent};
    use super::*;
    use crate::phase::ToolContext;
    use crate::state_types::Interaction;
    use crate::state_types::InteractionResponse;
    use crate::thread::Thread;
    use crate::types::ToolCall;
    use carve_state::Runtime;
    use serde_json::json;

    #[test]
    fn plugin_inactive_without_frontend_tools_or_responses() {
        let plugin = InteractionPlugin::new(HashSet::new(), Vec::new(), Vec::new());
        assert!(!plugin.is_active());
    }

    #[test]
    fn plugin_active_with_frontend_tools() {
        let mut tools = HashSet::new();
        tools.insert("copyToClipboard".to_string());
        let plugin = InteractionPlugin::new(tools, Vec::new(), Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_frontend_tools());
    }

    #[test]
    fn plugin_active_with_responses() {
        let plugin = InteractionPlugin::new(HashSet::new(), vec!["call_1".to_string()], Vec::new());
        assert!(plugin.is_active());
        assert!(plugin.has_responses());
    }

    #[tokio::test]
    async fn converts_intent_to_pending_immediately_within_same_run() {
        let plugin = InteractionPlugin::new(HashSet::new(), Vec::new(), Vec::new());
        let state = json!({});
        let ctx = Context::new(&state, "test", "test");
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "any_tool", json!({"x": 1}));
        step.tool = Some(ToolContext::new(&call));

        let interaction = Interaction::new("perm_1", "confirm").with_parameters(json!({"x": 1}));
        push_pending_intent(&mut step, interaction);

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_pending());
        let pending = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref())
            .expect("pending interaction should exist");
        assert_eq!(pending.id, "perm_1");
        assert!(
            !ctx.has_changes(),
            "run-local intent conversion should not patch state"
        );
    }

    #[tokio::test]
    async fn block_intent_has_priority_over_pending_intent() {
        let plugin = InteractionPlugin::new(HashSet::new(), Vec::new(), Vec::new());
        let state = json!({});
        let ctx = Context::new(&state, "test", "test");
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));

        push_pending_intent(
            &mut step,
            Interaction::new("perm_1", "confirm").with_message("allow?"),
        );
        push_block_intent(&mut step, "denied by policy");

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_blocked());
        assert!(!step.tool_pending());
    }

    #[tokio::test]
    async fn existing_gate_state_is_not_overridden_and_intents_are_dropped() {
        let plugin = InteractionPlugin::new(HashSet::new(), Vec::new(), Vec::new());
        let state = json!({});
        let ctx = Context::new(&state, "test", "test");
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "any_tool", json!({}));
        step.tool = Some(ToolContext::new(&call));
        step.block("already blocked");

        push_pending_intent(
            &mut step,
            Interaction::new("perm_1", "confirm").with_message("allow?"),
        );

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_blocked());
        assert!(!step.tool_pending());
        let intents: Vec<InteractionIntent> = step
            .scratchpad_get("__interaction_intents")
            .unwrap_or_default();
        assert!(
            intents.is_empty(),
            "consumed/dropped intents should not leak to next step"
        );
    }

    #[tokio::test]
    async fn runtime_frontend_tools_are_honored_by_static_plugin() {
        let plugin = InteractionPlugin::default();
        let state = json!({});
        let mut runtime = Runtime::new();
        runtime
            .set(
                crate::interaction::RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY,
                vec!["copyToClipboard"],
            )
            .unwrap();
        let ctx = Context::new(&state, "test", "test").with_runtime(Some(&runtime));
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "copyToClipboard", json!({"text": "hello"}));
        step.tool = Some(ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_pending(), "frontend tool should become pending");
    }

    #[tokio::test]
    async fn runtime_interaction_responses_are_honored_by_static_plugin() {
        let plugin = InteractionPlugin::default();
        let state = json!({
            "agent": {
                "pending_interaction": {
                    "id": "permission_write_file",
                    "action": "confirm",
                    "parameters": {
                        "tool_call": {
                            "id": "call_1",
                            "name": "write_file",
                            "arguments": {"path": "a.txt"}
                        }
                    }
                }
            }
        });
        let mut runtime = Runtime::new();
        runtime
            .set(
                crate::interaction::RUNTIME_INTERACTION_RESPONSES_KEY,
                vec![InteractionResponse::new(
                    "permission_write_file",
                    json!(true),
                )],
            )
            .unwrap();
        let ctx = Context::new(&state, "test", "test").with_runtime(Some(&runtime));
        let thread = Thread::new("t1");
        let mut step = StepContext::new(&thread, vec![]);

        plugin.on_phase(Phase::SessionStart, &mut step, &ctx).await;

        let replay: Vec<ToolCall> = step
            .scratchpad_get("__replay_tool_calls")
            .unwrap_or_default();
        assert_eq!(replay.len(), 1, "approved response should schedule replay");
        assert_eq!(replay[0].name, "write_file");
    }
}
