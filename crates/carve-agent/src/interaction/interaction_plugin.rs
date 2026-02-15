//! Unified interaction mechanism plugin.
//!
//! This plugin is strategy-agnostic: it provides a shared lifecycle/channel
//! for interaction intents and responses, while concrete strategies
//! (permissions, recovery, protocol adapters, etc.) emit intents independently.

use super::interaction_response::InteractionResponsePlugin;
use super::take_intents;
use super::InteractionIntent;
use crate::phase::{Phase, StepContext};
use crate::plugin::AgentPlugin;
use async_trait::async_trait;
use carve_state::Context;

/// Unified interaction mechanism plugin.
pub struct InteractionPlugin {
    static_response: InteractionResponsePlugin,
}

impl InteractionPlugin {
    /// Build plugin from interaction responses.
    pub fn new(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self {
            static_response: InteractionResponsePlugin::new(approved_ids, denied_ids),
        }
    }

    /// Build combined plugin from interaction responses only.
    pub fn with_responses(approved_ids: Vec<String>, denied_ids: Vec<String>) -> Self {
        Self::new(approved_ids, denied_ids)
    }

    /// Whether this plugin should be installed for the current request.
    pub fn is_active(&self) -> bool {
        self.static_response.has_responses()
    }

    /// Whether any interaction responses are present.
    pub fn has_responses(&self) -> bool {
        self.static_response.has_responses()
    }

    /// Check if an interaction ID is approved.
    pub fn is_approved(&self, interaction_id: &str) -> bool {
        self.static_response.is_approved(interaction_id)
    }

    /// Check if an interaction ID is denied.
    pub fn is_denied(&self, interaction_id: &str) -> bool {
        self.static_response.is_denied(interaction_id)
    }

    fn response_plugin(&self) -> InteractionResponsePlugin {
        InteractionResponsePlugin::from_responses(self.static_response.responses())
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
        "interaction"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, ctx: &Context<'_>) {
        let response = self.response_plugin();
        response.on_phase(phase, step, ctx).await;
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

        if let Some(interaction) = intents.into_iter().find_map(|intent| match intent {
            InteractionIntent::Pending { interaction } => Some(interaction),
        }) {
            step.pending(interaction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::push_pending_intent;
    use super::*;
    use crate::phase::ToolContext;
    use crate::state_types::Interaction;
    use crate::thread::Thread;
    use crate::types::ToolCall;
    use serde_json::json;

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
    async fn converts_intent_to_pending_immediately_within_same_run() {
        let plugin = InteractionPlugin::new(Vec::new(), Vec::new());
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
    async fn first_pending_intent_is_applied() {
        let plugin = InteractionPlugin::new(Vec::new(), Vec::new());
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
        push_pending_intent(
            &mut step,
            Interaction::new("perm_2", "confirm").with_message("allow 2?"),
        );

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
    }

    #[tokio::test]
    async fn existing_gate_state_is_not_overridden_and_intents_are_dropped() {
        let plugin = InteractionPlugin::new(Vec::new(), Vec::new());
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
}
