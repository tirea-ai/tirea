//! Runtime wiring for AG-UI requests.
//!
//! This crate builds a run-scoped [`carve_agentos::orchestrator::RunScope`] for AG-UI:
//! frontend tool descriptor stubs, frontend pending interaction strategy,
//! and interaction-response replay plugin wiring.

use async_trait::async_trait;
use carve_agentos::contracts::plugin::AgentPlugin;
use carve_agentos::contracts::runtime::phase::{Phase, StepContext};
use carve_agentos::contracts::runtime::Interaction;
use carve_agentos::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use carve_agentos::contracts::AgentState as RuntimeAgentState;
use carve_agentos::contracts::ToolCallContext;
use carve_agentos::extensions::interaction::InteractionPlugin;
use carve_agentos::orchestrator::{InMemoryToolRegistry, RunScope, ToolRegistry};
use carve_protocol_ag_ui::{build_context_addendum, RunAgentRequest};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Build a run-scoped [`RunScope`] from an AG-UI run request.
pub fn build_agui_run_scope(request: &RunAgentRequest) -> RunScope {
    let frontend_defs = request.frontend_tools();
    let frontend_tool_names: HashSet<String> =
        frontend_defs.iter().map(|tool| tool.name.clone()).collect();

    let mut scope = RunScope::new();

    // Frontend tools → InMemoryToolRegistry → RunScope.tool_registries
    if !frontend_defs.is_empty() {
        let mut registry = InMemoryToolRegistry::new();
        for tool in frontend_defs {
            let stub = Arc::new(FrontendToolStub::new(
                tool.name.clone(),
                tool.description.clone(),
                tool.parameters.clone(),
            ));
            // register() uses descriptor.id as key, which equals tool.name
            registry.register(stub).expect("frontend tool stubs should not conflict");
        }
        scope = scope.with_tool_registry(Arc::new(registry) as Arc<dyn ToolRegistry>);
    }

    // Run-scoped plugins
    if !frontend_tool_names.is_empty() {
        scope = scope.with_plugin(Arc::new(FrontendToolPendingPlugin::new(frontend_tool_names)));
    }

    // Context injection: forward useCopilotReadable context to the agent's system prompt.
    if let Some(addendum) = build_context_addendum(request) {
        scope = scope.with_plugin(Arc::new(ContextInjectionPlugin::new(addendum)));
    }

    let interaction_plugin = InteractionPlugin::with_responses(
        request.approved_interaction_ids(),
        request.denied_interaction_ids(),
    );
    if interaction_plugin.is_active() {
        scope = scope.with_plugin(Arc::new(interaction_plugin));
    }

    scope
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
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::error(
            &self.descriptor.id,
            "frontend tool stub should be intercepted before backend execution",
        ))
    }
}

/// Run-scoped plugin that injects AG-UI context (from `useCopilotReadable`)
/// into the agent's system prompt at each step.
struct ContextInjectionPlugin {
    addendum: String,
}

impl ContextInjectionPlugin {
    fn new(addendum: String) -> Self {
        Self { addendum }
    }
}

#[async_trait]
impl AgentPlugin for ContextInjectionPlugin {
    fn id(&self) -> &str {
        "agui_context_injection"
    }

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>, _ctx: &RuntimeAgentState) {
        if phase == Phase::StepStart {
            step.system(&self.addendum);
        }
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
    fn builds_frontend_tool_registry_from_request() {
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

        let scope = build_agui_run_scope(&request);
        // Frontend tools go to tool_registries
        assert_eq!(scope.tool_registries.len(), 1);
        let tools = scope.tool_registries[0].snapshot();
        assert!(tools.contains_key("copyToClipboard"));
        assert_eq!(tools.len(), 1);
        // FrontendToolPendingPlugin
        assert_eq!(scope.plugins.len(), 1);
        assert_eq!(scope.plugins[0].id(), "agui_frontend_tools");
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

        let scope = build_agui_run_scope(&request);
        assert!(scope.tool_registries.is_empty());
        // InteractionPlugin only
        assert_eq!(scope.plugins.len(), 1);
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

        let scope = build_agui_run_scope(&request);
        assert_eq!(scope.tool_registries.len(), 1);
        let tools = scope.tool_registries[0].snapshot();
        assert!(tools.contains_key("copyToClipboard"));
        // FrontendToolPendingPlugin + InteractionPlugin
        assert_eq!(scope.plugins.len(), 2);
    }

    #[test]
    fn returns_empty_scope_without_frontend_or_response_data() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let scope = build_agui_run_scope(&request);
        assert!(scope.tool_registries.is_empty());
        assert!(scope.plugins.is_empty());
        assert!(scope.is_empty());
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

    #[test]
    fn builds_context_injection_plugin_when_context_present() {
        use carve_protocol_ag_ui::AGUIContextEntry;

        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![AGUIMessage::user("hello")],
            tools: vec![],
            context: vec![AGUIContextEntry {
                description: "Current tasks".to_string(),
                value: json!(["Review PR", "Write tests"]),
            }],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let scope = build_agui_run_scope(&request);
        assert!(scope.plugins.iter().any(|p| p.id() == "agui_context_injection"));
    }

    #[tokio::test]
    async fn context_injection_plugin_adds_system_context() {
        use carve_protocol_ag_ui::AGUIContextEntry;

        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![AGUIMessage::user("hello")],
            tools: vec![],
            context: vec![AGUIContextEntry {
                description: "Task list".to_string(),
                value: json!(["Review PR", "Write tests"]),
            }],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let scope = build_agui_run_scope(&request);
        let plugin = scope.plugins.iter().find(|p| p.id() == "agui_context_injection").unwrap();

        let state = json!({});
        let ctx = RuntimeAgentState::new_transient(&state, "test-ctx", "agui_context_test");
        let thread = ConversationAgentState::new("t1");
        let mut step = StepContext::new(&thread, vec![]);

        plugin.on_phase(Phase::StepStart, &mut step, &ctx).await;

        assert!(!step.system_context.is_empty());
        let merged = step.system_context.join("\n");
        assert!(merged.contains("Task list"), "should contain context description");
        assert!(merged.contains("Review PR"), "should contain context values");
    }

    #[test]
    fn no_context_injection_plugin_when_context_empty() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let scope = build_agui_run_scope(&request);
        assert!(!scope.plugins.iter().any(|p| p.id() == "agui_context_injection"));
    }
}
