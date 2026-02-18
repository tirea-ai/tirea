//! Runtime wiring for AG-UI requests.
//!
//! This crate builds run-scoped [`carve_agentos::orchestrator::RunExtensions`] for AG-UI:
//! frontend tool descriptor stubs, frontend pending interaction strategy,
//! and interaction-response replay plugin wiring.

use async_trait::async_trait;
use carve_agentos::contracts::plugin::AgentPlugin;
use carve_agentos::contracts::runtime::phase::{Phase, StepContext};
use carve_agentos::contracts::runtime::Interaction;
use carve_agentos::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agentos::contracts::AgentState as RuntimeAgentState;
use carve_agentos::extensions::interaction::InteractionPlugin;
use carve_agentos::orchestrator::{RunExtensions, ToolPluginBundle};
use carve_protocol_ag_ui::RunAgentRequest;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Build run-scoped AgentOs extensions from an AG-UI run request.
pub fn build_agui_extensions(request: &RunAgentRequest) -> RunExtensions {
    let frontend_defs = request.frontend_tools();
    let frontend_tool_names: HashSet<String> =
        frontend_defs.iter().map(|tool| tool.name.clone()).collect();

    let mut bundle = ToolPluginBundle::new("agui_runtime");
    let mut has_contributions = false;
    for tool in frontend_defs {
        has_contributions = true;
        bundle = bundle.with_tool(Arc::new(FrontendToolStub::new(
            tool.name.clone(),
            tool.description.clone(),
            tool.parameters.clone(),
        )));
    }

    if !frontend_tool_names.is_empty() {
        has_contributions = true;
        bundle = bundle.with_plugin(Arc::new(FrontendToolPendingPlugin::new(
            frontend_tool_names,
        )));
    }

    let interaction_plugin = InteractionPlugin::with_responses(
        request.approved_interaction_ids(),
        request.denied_interaction_ids(),
    );
    if interaction_plugin.is_active() {
        has_contributions = true;
        bundle = bundle.with_plugin(Arc::new(interaction_plugin));
    }

    if has_contributions {
        RunExtensions::new().with_bundle(Arc::new(bundle))
    } else {
        RunExtensions::new()
    }
}

/// Runtime-only frontend tool descriptor stub.
///
/// The frontend pending plugin intercepts configured frontend tools before
/// backend execution. This stub exists only to expose tool descriptors to the model.
struct FrontendToolStub {
    descriptor: ToolDescriptor,
}

impl FrontendToolStub {
    fn new(name: String, description: String, parameters: Option<Value>) -> Self {
        let mut descriptor = ToolDescriptor::new(&name, &name, description);
        if let Some(parameters) = parameters {
            descriptor = descriptor.with_parameters(parameters);
        }
        Self { descriptor }
    }
}

#[async_trait]
impl Tool for FrontendToolStub {
    fn descriptor(&self) -> ToolDescriptor {
        self.descriptor.clone()
    }

    async fn execute(
        &self,
        _args: Value,
        _ctx: &RuntimeAgentState,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::error(
            &self.descriptor.id,
            "frontend tool stub should be intercepted before backend execution",
        ))
    }
}

/// Run-scoped frontend interaction strategy for AG-UI.
struct FrontendToolPendingPlugin {
    frontend_tools: HashSet<String>,
}

impl FrontendToolPendingPlugin {
    fn new(frontend_tools: HashSet<String>) -> Self {
        Self { frontend_tools }
    }
}

#[async_trait]
impl AgentPlugin for FrontendToolPendingPlugin {
    fn id(&self) -> &str {
        "agui_frontend_tools"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &RuntimeAgentState) {
        if phase != Phase::BeforeToolExecute {
            return;
        }

        if step.tool_blocked() || step.tool_pending() {
            return;
        }

        let Some(tool) = step.tool.as_ref() else {
            return;
        };

        if !self.frontend_tools.contains(&tool.name) {
            return;
        }

        let interaction = Interaction::new(&tool.id, format!("tool:{}", tool.name))
            .with_parameters(tool.args.clone());
        step.pending(interaction);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use carve_agentos::contracts::runtime::phase::{Phase, ToolContext};
    use carve_agentos::contracts::state::AgentState as ConversationAgentState;
    use carve_agentos::contracts::state::ToolCall;
    use carve_protocol_ag_ui::{AGUIMessage, AGUIToolDef, ToolExecutionLocation};
    use serde_json::json;

    #[test]
    fn builds_frontend_tool_stubs_from_request() {
        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![AGUIMessage::user("hello")],
            tools: vec![
                AGUIToolDef {
                    name: "copyToClipboard".to_string(),
                    description: "copy".to_string(),
                    parameters: Some(json!({
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        },
                        "required": ["text"]
                    })),
                    execute: ToolExecutionLocation::Frontend,
                },
                AGUIToolDef::backend("search", "backend search"),
            ],
            context: vec![],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let extensions = build_agui_extensions(&request);
        assert_eq!(extensions.bundles.len(), 1);
    }

    #[test]
    fn builds_response_only_plugin_when_no_frontend_tools() {
        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![
                AGUIMessage::user("hello"),
                AGUIMessage::tool("true", "interaction_1"),
            ],
            tools: vec![],
            context: vec![],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let extensions = build_agui_extensions(&request);
        assert_eq!(extensions.bundles.len(), 1);
    }

    #[test]
    fn builds_frontend_and_response_plugins_together() {
        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![
                AGUIMessage::user("hello"),
                AGUIMessage::tool("true", "call_1"),
            ],
            tools: vec![AGUIToolDef {
                name: "copyToClipboard".to_string(),
                description: "copy".to_string(),
                parameters: None,
                execute: ToolExecutionLocation::Frontend,
            }],
            context: vec![],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let extensions = build_agui_extensions(&request);
        assert_eq!(extensions.bundles.len(), 1);
    }

    #[test]
    fn returns_empty_extensions_without_frontend_or_response_data() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let extensions = build_agui_extensions(&request);
        assert!(extensions.is_empty());
    }

    #[tokio::test]
    async fn frontend_pending_plugin_marks_frontend_call_as_pending() {
        let plugin =
            FrontendToolPendingPlugin::new(["copyToClipboard".to_string()].into_iter().collect());
        let state = json!({});
        let ctx = RuntimeAgentState::new_transient(&state, "test-call", "agui_runtime_test");
        let thread = ConversationAgentState::new("t1");
        let mut step = StepContext::new(&thread, vec![]);
        let call = ToolCall::new("call_1", "copyToClipboard", json!({"text":"hello"}));
        step.tool = Some(ToolContext::new(&call));

        plugin
            .on_phase(Phase::BeforeToolExecute, &mut step, &ctx)
            .await;

        assert!(step.tool_pending());
        let pending = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref())
            .expect("pending interaction should exist");
        assert_eq!(pending.id, "call_1");
        assert_eq!(pending.action, "tool:copyToClipboard");
    }
}
