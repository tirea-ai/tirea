//! Runtime wiring for AG-UI requests.
//!
//! Applies AG-UI–specific extensions to a [`ResolvedRun`]:
//! frontend tool descriptor stubs, frontend suspended-call strategy,
//! and interaction-response replay plugin wiring.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tirea_agent_loop::runtime::loop_runner::ResolvedRun;
use tirea_contract::event::interaction::{
    FrontendToolInvocation, InvocationOrigin, ResponseRouting,
};
use tirea_contract::plugin::phase::{
    BeforeInferenceContext, BeforeToolExecuteContext, SuspendTicket, ToolGateDecision,
};
use tirea_contract::plugin::AgentPlugin;
use tirea_contract::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_contract::ToolCallContext;
use tirea_extension_interaction::InteractionPlugin;

use crate::{build_context_addendum, RunAgentInput};

/// Run-config key carrying AG-UI request `config`.
pub const AGUI_CONFIG_KEY: &str = "agui_config";
/// Run-config key carrying AG-UI request `forwardedProps`.
pub const AGUI_FORWARDED_PROPS_KEY: &str = "agui_forwarded_props";

/// Apply AG-UI–specific extensions to a [`ResolvedRun`].
///
/// Injects frontend tool stubs, suspended-call plugins, context
/// injection, and interaction-response replay wiring.
pub fn apply_agui_extensions(resolved: &mut ResolvedRun, request: &RunAgentInput) {
    if let Some(model) = request.model.as_ref().filter(|m| !m.trim().is_empty()) {
        resolved.config.model = model.clone();
    }
    if let Some(system_prompt) = request
        .system_prompt
        .as_ref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        resolved.config.system_prompt = system_prompt.clone();
    }
    if let Some(config) = request.config.clone() {
        apply_agui_chat_options_overrides(resolved, &config);
        let _ = resolved.run_config.set(AGUI_CONFIG_KEY, config);
    }
    if let Some(forwarded_props) = request.forwarded_props.clone() {
        let _ = resolved
            .run_config
            .set(AGUI_FORWARDED_PROPS_KEY, forwarded_props);
    }

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

fn apply_agui_chat_options_overrides(resolved: &mut ResolvedRun, config: &Value) {
    let mut chat_options = resolved.config.chat_options.clone().unwrap_or_default();
    let mut changed = false;

    if let Some(map) = config.as_object() {
        if let Some(value) = get_bool(map, "captureReasoningContent", "capture_reasoning_content") {
            chat_options.capture_reasoning_content = Some(value);
            changed = true;
        }
        if let Some(value) = get_bool(
            map,
            "normalizeReasoningContent",
            "normalize_reasoning_content",
        ) {
            chat_options.normalize_reasoning_content = Some(value);
            changed = true;
        }
    }

    if let Some(map) = config
        .get("chatOptions")
        .and_then(Value::as_object)
        .or_else(|| config.get("chat_options").and_then(Value::as_object))
    {
        if let Some(value) = get_bool(map, "captureReasoningContent", "capture_reasoning_content") {
            chat_options.capture_reasoning_content = Some(value);
            changed = true;
        }
        if let Some(value) = get_bool(
            map,
            "normalizeReasoningContent",
            "normalize_reasoning_content",
        ) {
            chat_options.normalize_reasoning_content = Some(value);
            changed = true;
        }
    }

    if changed {
        resolved.config.chat_options = Some(chat_options);
    }
}

fn get_bool(map: &serde_json::Map<String, Value>, primary: &str, alias: &str) -> Option<bool> {
    map.get(primary)
        .or_else(|| map.get(alias))
        .and_then(Value::as_bool)
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
/// into the agent's system prompt before inference.
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

    async fn before_inference(&self, ctx: &mut BeforeInferenceContext<'_, '_>) {
        ctx.add_system_context(&self.addendum);
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

    async fn before_tool_execute(&self, ctx: &mut BeforeToolExecuteContext<'_, '_>) {
        if !matches!(ctx.decision(), ToolGateDecision::Proceed) {
            return;
        }

        let Some(tool_name) = ctx.tool_name() else {
            return;
        };
        if !self.frontend_tools.contains(tool_name) {
            return;
        }
        let Some(call_id) = ctx.tool_call_id().map(str::to_string) else {
            return;
        };

        let args = ctx.tool_args().cloned().unwrap_or_default();
        let invocation = FrontendToolInvocation::new(
            call_id,
            tool_name.to_string(),
            args,
            InvocationOrigin::PluginInitiated {
                plugin_id: self.id().to_string(),
            },
            ResponseRouting::UseAsToolResult,
        );
        ctx.suspend(SuspendTicket::from_invocation(invocation));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, Message, ToolExecutionLocation};
    use async_trait::async_trait;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tirea_agent_loop::runtime::loop_runner::AgentConfig;
    use tirea_contract::plugin::phase::{StepContext, ToolContext};
    use tirea_contract::plugin::AgentPlugin;
    use tirea_contract::testing::TestFixture;
    use tirea_contract::thread::ToolCall;

    async fn run_before_tool_execute(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = tirea_contract::plugin::phase::BeforeToolExecuteContext::new(step);
        plugin.before_tool_execute(&mut ctx).await;
    }

    async fn run_before_inference(plugin: &dyn AgentPlugin, step: &mut StepContext<'_>) {
        let mut ctx = tirea_contract::plugin::phase::BeforeInferenceContext::new(step);
        plugin.before_inference(&mut ctx).await;
    }

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
    }

    #[test]
    fn injects_frontend_tools_into_resolved() {
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![Message::user("hello")],
            tools: vec![
                crate::Tool {
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
                crate::Tool::backend("search", "backend search"),
            ],
            context: vec![],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
            forwarded_props: None,
        };

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert_eq!(resolved.config.tool_executor.name(), "parallel");
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
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![
                Message::user("hello"),
                Message::tool("true", "interaction_1"),
            ],
            tools: vec![],
            context: vec![],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
            forwarded_props: None,
        };

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.is_empty());
        // InteractionPlugin only
        assert_eq!(resolved.config.plugins.len(), 1);
    }

    #[test]
    fn injects_response_plugin_for_non_boolean_frontend_tool_payload() {
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![
                Message::user("hello"),
                Message::tool(r#"{"todo":"ship starter"}"#, "call_copy_1"),
            ],
            tools: vec![],
            context: vec![],
            state: Some(json!({
                "__suspended_tool_calls": {
                    "calls": {
                        "call_copy_1": {
                            "call_id": "call_copy_1",
                            "tool_name": "copyToClipboard",
                            "interaction": {
                                "id": "call_copy_1",
                                "action": "tool:copyToClipboard"
                            },
                            "frontend_invocation": {
                                "call_id": "call_copy_1",
                                "tool_name": "copyToClipboard",
                                "routing": { "strategy": "use_as_tool_result" },
                                "origin": {
                                    "type": "plugin_initiated",
                                    "plugin_id": "agui_frontend_tools"
                                }
                            }
                        }
                    }
                }
            })),
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
            forwarded_props: None,
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
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![Message::user("hello"), Message::tool("true", "call_1")],
            tools: vec![crate::Tool {
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
            forwarded_props: None,
        };

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(resolved.tools.contains_key("copyToClipboard"));
        // FrontendToolPendingPlugin + InteractionPlugin
        assert_eq!(resolved.config.plugins.len(), 2);
    }

    #[test]
    fn prepends_frontend_pending_plugin_before_existing_plugins() {
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![Message::user("hello")],
            tools: vec![crate::Tool {
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
            forwarded_props: None,
        };

        let mut resolved = empty_resolved();
        resolved.config.plugins.push(Arc::new(MarkerPlugin));

        apply_agui_extensions(&mut resolved, &request);

        assert_eq!(
            resolved.config.plugins.first().map(|p| p.id()),
            Some("agui_frontend_tools")
        );
        assert_eq!(
            resolved.config.plugins.get(1).map(|p| p.id()),
            Some("marker_plugin")
        );
    }

    #[test]
    fn no_changes_without_frontend_or_response_data() {
        let request = RunAgentInput::new("t1", "r1").with_message(Message::user("hello"));
        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert_eq!(resolved.config.tool_executor.name(), "parallel");
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

        run_before_tool_execute(&plugin, &mut step).await;

        assert!(step.tool_pending());

        // Suspension interaction should be set
        let pending = step
            .tool
            .as_ref()
            .and_then(|t| t.suspend_ticket.as_ref())
            .map(|ticket| &ticket.suspension)
            .expect("suspended interaction should exist");
        assert_eq!(pending.action, "tool:copyToClipboard");

        // First-class FrontendToolInvocation should be set
        let inv = step
            .tool
            .as_ref()
            .and_then(|t| t.suspend_ticket.as_ref())
            .and_then(|ticket| ticket.invocation.as_ref())
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
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            context: vec![Context {
                description: "Current tasks".to_string(),
                value: json!(["Review PR", "Write tests"]),
            }],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
            forwarded_props: None,
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
        let request = RunAgentInput {
            thread_id: "t1".to_string(),
            run_id: "r1".to_string(),
            messages: vec![Message::user("hello")],
            tools: vec![],
            context: vec![Context {
                description: "Task list".to_string(),
                value: json!(["Review PR", "Write tests"]),
            }],
            state: None,
            parent_run_id: None,
            model: None,
            system_prompt: None,
            config: None,
            forwarded_props: None,
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

        run_before_inference(plugin.as_ref(), &mut step).await;

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
        let request = RunAgentInput::new("t1", "r1").with_message(Message::user("hello"));
        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);
        assert!(!resolved
            .config
            .plugins
            .iter()
            .any(|p| p.id() == "agui_context_injection"));
    }

    #[test]
    fn applies_request_model_and_system_prompt_overrides() {
        let request = RunAgentInput::new("t1", "r1")
            .with_message(Message::user("hello"))
            .with_model("gpt-4.1")
            .with_system_prompt("You are precise.");

        let mut resolved = empty_resolved();
        resolved.config.model = "base-model".to_string();
        resolved.config.system_prompt = "base-prompt".to_string();

        apply_agui_extensions(&mut resolved, &request);

        assert_eq!(resolved.config.model, "gpt-4.1");
        assert_eq!(resolved.config.system_prompt, "You are precise.");
    }

    #[test]
    fn writes_config_and_forwarded_props_into_run_config() {
        let request = RunAgentInput::new("t1", "r1")
            .with_message(Message::user("hello"))
            .with_state(json!({"k":"v"}))
            .with_forwarded_props(json!({"session":"abc"}));
        let mut request = request;
        request.config = Some(json!({"temperature": 0.2}));

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);

        assert_eq!(
            resolved.run_config.value(AGUI_CONFIG_KEY),
            Some(&json!({"temperature": 0.2}))
        );
        assert_eq!(
            resolved.run_config.value(AGUI_FORWARDED_PROPS_KEY),
            Some(&json!({"session":"abc"}))
        );
    }

    #[test]
    fn applies_chat_options_overrides_from_agui_config() {
        let mut request = RunAgentInput::new("t1", "r1").with_message(Message::user("hello"));
        request.config = Some(json!({
            "captureReasoningContent": true,
            "normalizeReasoningContent": true,
            "reasoningEffort": "high"
        }));

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);

        let options = resolved
            .config
            .chat_options
            .expect("chat options should exist");
        assert_eq!(options.capture_reasoning_content, Some(true));
        assert_eq!(options.normalize_reasoning_content, Some(true));
        assert_eq!(
            resolved.run_config.value(AGUI_CONFIG_KEY),
            Some(&json!({
                "captureReasoningContent": true,
                "normalizeReasoningContent": true,
                "reasoningEffort": "high"
            }))
        );
    }

    #[test]
    fn applies_nested_chat_options_overrides_from_agui_config() {
        let mut request = RunAgentInput::new("t1", "r1").with_message(Message::user("hello"));
        request.config = Some(json!({
            "chat_options": {
                "capture_reasoning_content": false,
                "reasoning_effort": 256
            }
        }));

        let mut resolved = empty_resolved();
        apply_agui_extensions(&mut resolved, &request);

        let options = resolved
            .config
            .chat_options
            .expect("chat options should exist");
        assert_eq!(options.capture_reasoning_content, Some(false));
        assert_eq!(options.normalize_reasoning_content, None);
    }
}
