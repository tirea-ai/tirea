use async_trait::async_trait;
use carve_agent::interaction::InteractionPlugin;
use carve_agent::{
    AgentPlugin, Context, Interaction, Phase, RunExtensions, StepContext, Tool, ToolDescriptor,
    ToolError, ToolResult,
};
use carve_protocol_ag_ui::RunAgentRequest;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Build run-scoped AgentOs extensions from an AG-UI run request.
pub fn build_agui_extensions(request: &RunAgentRequest) -> RunExtensions {
    let frontend_defs = request.frontend_tools();
    let frontend_tool_names: HashSet<String> =
        frontend_defs.iter().map(|tool| tool.name.clone()).collect();

    let mut extensions = RunExtensions::new();
    for tool in frontend_defs {
        extensions = extensions.with_tool(Arc::new(FrontendToolStub::new(
            tool.name.clone(),
            tool.description.clone(),
            tool.parameters.clone(),
        )));
    }

    if !frontend_tool_names.is_empty() {
        extensions = extensions.with_plugin(Arc::new(FrontendToolPendingPlugin::new(
            frontend_tool_names,
        )));
    }

    let interaction_plugin = InteractionPlugin::with_responses(
        request.approved_interaction_ids(),
        request.denied_interaction_ids(),
    );
    if interaction_plugin.is_active() {
        extensions = extensions.with_plugin(Arc::new(interaction_plugin));
    }

    extensions
}

/// Runtime-only frontend tool descriptor stub.
///
/// The interaction plugin intercepts configured frontend tools before backend
/// execution. This stub exists only to expose tool descriptors to the model.
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

    async fn execute(&self, _args: Value, _ctx: &Context<'_>) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::error(
            &self.descriptor.id,
            "frontend tool stub should be intercepted before backend execution",
        ))
    }
}

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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &Context<'_>) {
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
        assert_eq!(extensions.tools.len(), 1);
        assert!(extensions.tools.contains_key("copyToClipboard"));
        assert_eq!(extensions.plugins.len(), 1);
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
        assert!(extensions.tools.is_empty());
        assert_eq!(extensions.plugins.len(), 1);
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
        assert_eq!(extensions.plugins.len(), 2);
    }

    #[test]
    fn returns_empty_extensions_without_frontend_or_response_data() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let extensions = build_agui_extensions(&request);
        assert!(extensions.is_empty());
    }
}
