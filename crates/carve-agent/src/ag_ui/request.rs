pub use crate::interaction::InteractionPlugin;
use crate::interaction::{
    merge_frontend_tools, FrontendToolSpec, RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY,
    RUNTIME_INTERACTION_RESPONSES_KEY,
};
use crate::r#loop::AgentConfig;
use crate::state_types::InteractionResponse;
use crate::thread::Thread;
use crate::traits::tool::Tool;
use carve_protocol_ag_ui::{
    build_context_addendum, core_message_from_ag_ui, interaction_runtime_values, MessageRole,
    RunAgentRequest,
};
use std::collections::HashMap;
use std::sync::Arc;

fn frontend_tool_specs_from_request(request: &RunAgentRequest) -> Vec<FrontendToolSpec> {
    request
        .frontend_tools()
        .into_iter()
        .map(|tool| FrontendToolSpec {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.parameters.clone(),
        })
        .collect()
}

impl InteractionPlugin {
    /// Build an interaction plugin from AG-UI request fields.
    pub fn from_request(request: &RunAgentRequest) -> Self {
        Self::new(
            request
                .frontend_tools()
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
            request.approved_interaction_ids(),
            request.denied_interaction_ids(),
        )
    }
}

pub(super) fn should_seed_session_from_request(thread: &Thread, request: &RunAgentRequest) -> bool {
    let session_state_is_empty_object = thread.state.as_object().is_some_and(|m| m.is_empty());

    let request_has_state = request.state.as_ref().is_some_and(|s| !s.is_null());
    let request_has_messages = !request.messages.is_empty();

    thread.messages.is_empty()
        && thread.patches.is_empty()
        && session_state_is_empty_object
        && (request_has_state || request_has_messages)
}

pub(super) fn seed_session_from_request(thread: Thread, request: &RunAgentRequest) -> Thread {
    let state = request
        .state
        .clone()
        .unwrap_or_else(|| thread.state.clone());
    let messages = request
        .messages
        .iter()
        .map(core_message_from_ag_ui)
        .collect::<Vec<_>>();

    Thread::with_initial_state(request.thread_id.clone(), state).with_messages(messages)
}

pub(super) fn session_has_message_id(thread: &Thread, id: &str) -> bool {
    thread
        .messages
        .iter()
        .any(|m| m.id.as_deref().is_some_and(|mid| mid == id))
}

pub(super) fn session_has_tool_call_id(thread: &Thread, tool_call_id: &str) -> bool {
    thread.messages.iter().any(|m| {
        m.tool_call_id
            .as_deref()
            .is_some_and(|tid| tid == tool_call_id)
    })
}

/// Apply request messages/state into an existing thread.
pub(crate) fn apply_agui_request_to_thread(thread: Thread, request: &RunAgentRequest) -> Thread {
    if should_seed_session_from_request(&thread, request) {
        return seed_session_from_request(thread, request);
    }

    let mut new_msgs = Vec::new();

    for msg in &request.messages {
        if msg.role == MessageRole::Assistant {
            continue;
        }

        if msg.role == MessageRole::Tool {
            if let Some(tool_call_id) = msg.tool_call_id.as_deref() {
                if !session_has_tool_call_id(&thread, tool_call_id) {
                    new_msgs.push(core_message_from_ag_ui(msg));
                }
            }
            continue;
        }

        if let Some(id) = msg.id.as_deref() {
            if !session_has_message_id(&thread, id) {
                new_msgs.push(core_message_from_ag_ui(msg));
            }
            continue;
        }

        new_msgs.push(core_message_from_ag_ui(msg));
    }

    if new_msgs.is_empty() {
        thread
    } else {
        thread.with_messages(new_msgs)
    }
}

/// Runtime key used to mark that a specific AG-UI request has already been applied.
pub(crate) const AGUI_REQUEST_APPLIED_RUNTIME_KEY: &str = "__agui_request_applied";

pub(super) fn ensure_agui_request_applied(mut thread: Thread, request: &RunAgentRequest) -> Thread {
    let already_applied = thread
        .runtime
        .value(AGUI_REQUEST_APPLIED_RUNTIME_KEY)
        .and_then(|v| v.as_str())
        .is_some_and(|run_id| run_id == request.run_id);
    if !already_applied {
        thread = apply_agui_request_to_thread(thread, request);
    }
    let _ = thread
        .runtime
        .set(AGUI_REQUEST_APPLIED_RUNTIME_KEY, request.run_id.clone());
    thread
}

fn apply_request_overrides(mut config: AgentConfig, request: &RunAgentRequest) -> AgentConfig {
    if let Some(model) = request.model.as_ref() {
        config.model = model.clone();
    }
    if let Some(prompt) = request.system_prompt.as_ref() {
        config.system_prompt = prompt.clone();
    }

    if let Some(addendum) = build_context_addendum(request) {
        config.system_prompt.push_str(&addendum);
    }

    config
}

struct RequestWiring {
    install_interaction: bool,
}

impl RequestWiring {
    fn from_request(request: &RunAgentRequest) -> Self {
        Self {
            install_interaction: request.tools.iter().any(|t| t.is_frontend())
                || request.has_any_interaction_responses(),
        }
    }

    fn apply(
        self,
        mut config: AgentConfig,
        tools: &mut HashMap<String, Arc<dyn Tool>>,
        request: &RunAgentRequest,
    ) -> AgentConfig {
        if !request.frontend_tools().is_empty() {
            let frontend_specs = frontend_tool_specs_from_request(request);
            merge_frontend_tools(tools, &frontend_specs);
        }

        if self.install_interaction && !config.plugins.iter().any(|p| p.id() == "interaction") {
            config = config.with_plugin(Arc::new(InteractionPlugin::default()));
        }
        config
    }
}

pub(super) fn set_run_identity(thread: &mut Thread, run_id: &str, parent_run_id: Option<&str>) {
    let _ = thread.runtime.set("run_id", run_id.to_string());
    if let Some(parent) = parent_run_id {
        let _ = thread.runtime.set("parent_run_id", parent.to_string());
    }
}

fn set_interaction_request_context(thread: &mut Thread, request: &RunAgentRequest) {
    for (key, value) in interaction_runtime_values(request) {
        if key == RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY {
            if let Ok(frontend_tools) = serde_json::from_value::<Vec<String>>(value) {
                let _ = thread.runtime.set(key, frontend_tools);
            }
        } else if key == RUNTIME_INTERACTION_RESPONSES_KEY {
            if let Ok(responses) = serde_json::from_value::<Vec<InteractionResponse>>(value) {
                let _ = thread.runtime.set(key, responses);
            }
        } else {
            let _ = thread.runtime.set(key, value);
        }
    }
}

pub(super) fn prepare_request_runtime(
    config: AgentConfig,
    thread: Thread,
    mut tools: HashMap<String, Arc<dyn Tool>>,
    request: &RunAgentRequest,
) -> (AgentConfig, Thread, HashMap<String, Arc<dyn Tool>>) {
    let mut thread = ensure_agui_request_applied(thread, request);
    set_interaction_request_context(&mut thread, request);
    let config = apply_request_overrides(config, request);
    let config = RequestWiring::from_request(request).apply(config, &mut tools, request);
    (config, thread, tools)
}
