//! Runtime wiring for AG-UI requests.
//!
//! Applies AG-UI–specific extensions to a [`ResolvedRun`]:
//! frontend tool descriptor stubs, frontend pending interaction strategy,
//! and interaction-response replay plugin wiring.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tirea_agent_loop::runtime::loop_runner::ResolvedRun;
use tirea_contract::event::interaction::ResponseRouting;
use tirea_contract::plugin::phase::{Phase, StepContext};
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_contract::ToolCallContext;
use tirea_extension_interaction::InteractionPlugin;

use crate::{build_context_addendum, RunAgentRequest};

/// Apply AG-UI–specific extensions to a [`ResolvedRun`].
///
/// Injects frontend tool stubs, pending-interaction plugins, context
/// injection, and interaction-response replay wiring.
pub fn apply_agui_extensions(resolved: &mut ResolvedRun, request: &RunAgentRequest) {
    let frontend_defs = request.frontend_tools();
    let frontend_tool_names: HashSet<String> =
        frontend_defs.iter().map(|tool| tool.name.clone()).collect();

    // Frontend tools → insert into resolved.tools (overlay semantics)
    for tool in frontend_defs {
        let stub = Arc::new(FrontendToolStub::new(
            tool.name.clone(),
            tool.description.clone(),
            tool.parameters.clone(),
        ));
        let id = stub.descriptor().id.clone();
        resolved.tools.entry(id).or_insert(stub as Arc<dyn Tool>);
    }

    // Run-scoped plugins
    if !frontend_tool_names.is_empty() {
        resolved.config.plugins.insert(
            0,
            Arc::new(FrontendToolPendingPlugin::new(frontend_tool_names)),
        );
    }

    // Context injection: forward useCopilotReadable context to the agent's system prompt.
    if let Some(addendum) = build_context_addendum(request) {
        resolved
            .config
            .plugins
            .push(Arc::new(ContextInjectionPlugin::new(addendum)));
    }

    let interaction_plugin =
        InteractionPlugin::from_interaction_responses(request.interaction_responses());
    if interaction_plugin.is_active() {
        resolved.config.plugins.push(Arc::new(interaction_plugin));
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
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

        let tool_name = tool.name.clone();
        let args = tool.args.clone();
        step.invoke_frontend_tool(tool_name, args, ResponseRouting::UseAsToolResult);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::{AGUIContextEntry, AGUIMessage, AGUIToolDef, ToolExecutionLocation};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tirea_agent_loop::runtime::loop_runner::AgentConfig;
    use tirea_contract::plugin::AgentPlugin;
    use tirea_contract::plugin::phase::{Phase, StepContext, ToolContext};
    use tirea_contract::testing::TestFixture;
    use tirea_contract::thread::ToolCall;

    fn empty_resolved() -> ResolvedRun {
        ResolvedRun {
            config: AgentConfig::default(),
            tools: HashMap::new(),
            run_config: tirea_contract::RunConfig::new(),
        }
    }

    struct MarkerPlugin;

    #[async_trait]
    impl AgentPlugin for MarkerPlugin {
        fn id(&self) -> &str {
            "marker_plugin"
        }

        async fn on_phase(&self, _phase: Phase, _step: &mut StepContext<'_>) {}
    }

    #[test]
    fn injects_frontend_tools_into_resolved() {
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

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.contains_key("copyToClipboard"));
        // Only 1 frontend tool (backend tools are not stubs)
        assert_eq!(resolved.tools.len(), 1);
        // FrontendToolPendingPlugin
        assert!(resolved
            .config
            .plugins
            .iter()
            .any(|p| p.id() == "agui_frontend_tools"));
    }

    #[test]
    fn injects_response_only_plugin_when_no_frontend_tools() {
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

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.is_empty());
        // InteractionPlugin only
        assert_eq!(resolved.config.plugins.len(), 1);
    }

    #[test]
    fn injects_response_plugin_for_non_boolean_frontend_tool_payload() {
        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![
                AGUIMessage::user("hello"),
                AGUIMessage::tool(r#"{"todo":"ship starter"}"#, "call_copy_1"),
            ],
            tools: vec![],
            context: vec![],
            state: Some(json!({
                "loop_control": {
                    "pending_interaction": {
                        "id": "call_copy_1",
                        "action": "tool:copyToClipboard"
                    },
                    "pending_frontend_invocation": {
                        "call_id": "call_copy_1",
                        "tool_name": "copyToClipboard",
                        "routing": { "strategy": "use_as_tool_result" },
                        "origin": {
                            "type": "plugin_initiated",
                            "plugin_id": "agui_frontend_tools"
                        }
                    }
                }
            })),
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
        };

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.is_empty());
        // Non-boolean tool response must still install InteractionPlugin
        // so routing (e.g. use_as_tool_result) can replay and clear pending state.
        assert_eq!(resolved.config.plugins.len(), 1);
    }

    #[test]
    fn injects_frontend_and_response_plugins_together() {
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

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.contains_key("copyToClipboard"));
        // FrontendToolPendingPlugin + InteractionPlugin
        assert_eq!(resolved.config.plugins.len(), 2);
    }

    #[test]
    fn prepends_frontend_pending_plugin_before_existing_plugins() {
        let request = RunAgentRequest {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![AGUIMessage::user("hello")],
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

        let mut resolved = empty_resolved();
        resolved.config.plugins.push(Arc::new(MarkerPlugin));

        apply_agui_extensions(&mut resolved, &request);

        assert_eq!(
            resolved.config.plugins.first().map(|p| p.id()),
            Some("agui_frontend_tools")
        );
        assert_eq!(resolved.config.plugins.get(1).map(|p| p.id()), Some("marker_plugin"));
    }

    #[test]
    fn no_changes_without_frontend_or_response_data() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.is_empty());
        assert!(resolved.config.plugins.is_empty());
    }

    #[tokio::test]
    async fn frontend_pending_plugin_marks_frontend_call_as_pending() {
        use tirea_contract::event::interaction::{InvocationOrigin, ResponseRouting};

        let plugin =
            FrontendToolPendingPlugin::new(["copyToClipboard".to_string()].into_iter().collect());
        let fixture = TestFixture::new();
        let mut step = fixture.step(vec![]);
        let call = ToolCall::new("call_1", "copyToClipboard", json!({"text":"hello"}));
        step.tool = Some(ToolContext::new(&call));

        plugin.on_phase(Phase::BeforeToolExecute, &mut step).await;

        assert!(step.tool_pending());

        // Backward-compat Interaction should still be set
        let pending = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_interaction.as_ref())
            .expect("pending interaction should exist");
        assert_eq!(pending.action, "tool:copyToClipboard");

        // First-class FrontendToolInvocation should be set
        let inv = step
            .tool
            .as_ref()
            .and_then(|t| t.pending_frontend_invocation.as_ref())
            .expect("pending frontend invocation should exist");
        assert_eq!(inv.call_id, "call_1");
        assert_eq!(inv.tool_name, "copyToClipboard");
        assert_eq!(inv.arguments["text"], "hello");
        assert!(matches!(inv.routing, ResponseRouting::UseAsToolResult));
        assert!(matches!(
            inv.origin,
            InvocationOrigin::PluginInitiated { .. }
        ));
    }

    #[test]
    fn injects_context_injection_plugin_when_context_present() {
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

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved
            .config
            .plugins
            .iter()
            .any(|p| p.id() == "agui_context_injection"));
    }

    #[tokio::test]
    async fn context_injection_plugin_adds_system_context() {
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

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        let plugin = resolved
            .config
            .plugins
            .iter()
            .find(|p| p.id() == "agui_context_injection")
            .unwrap();

        let fixture = TestFixture::new();
        let mut step = fixture.step(vec![]);

        plugin.on_phase(Phase::StepStart, &mut step).await;

        assert!(!step.system_context.is_empty());
        let merged = step.system_context.join("\n");
        assert!(
            merged.contains("Task list"),
            "should contain context description"
        );
        assert!(
            merged.contains("Review PR"),
            "should contain context values"
        );
    }

    #[test]
    fn no_context_injection_plugin_when_context_empty() {
        let request = RunAgentRequest::new("t1", "r1").with_message(AGUIMessage::user("hello"));
        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(!resolved
            .config
            .plugins
            .iter()
            .any(|p| p.id() == "agui_context_injection"));
    }
}
