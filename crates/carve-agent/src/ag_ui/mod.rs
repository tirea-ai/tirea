//! AG-UI protocol support and request/runtime adapters.

mod context;
mod protocol;
mod request;
mod runner;

pub use crate::interaction::AgUiInteractionPlugin;
pub use request::InteractionPlugin;
pub(crate) use request::core_message_from_ag_ui;

pub use context::AGUIContext;
pub use protocol::{AGUIEvent, BaseEventFields, MessageRole};
pub use request::{
    AGUIContextEntry, AGUIMessage, AGUIToolDef, RequestError, RunAgentRequest, ToolExecutionLocation,
};
pub use runner::{
    run_agent_events_with_request, run_agent_stream,
    run_agent_stream_with_parent, run_agent_stream_with_request,
};

#[cfg(test)]
use request::{
    AGUI_REQUEST_APPLIED_RUNTIME_KEY,
    apply_agui_request_to_thread, build_context_addendum,
    ensure_agui_request_applied, merge_frontend_tools, prepare_request_runtime,
    seed_session_from_request, session_has_message_id, session_has_tool_call_id,
    should_seed_session_from_request,
};
#[cfg(test)]
use context::value_to_map;

#[cfg(test)]
use crate::r#loop::AgentConfig;
#[cfg(test)]
use crate::state_types::{Interaction, InteractionResponse};
#[cfg(test)]
use crate::thread::Thread;
#[cfg(test)]
use crate::traits::tool::Tool;
#[cfg(test)]
use genai::Client;
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;

#[cfg(test)]
use crate::interaction::{FrontendToolPlugin, InteractionResponsePlugin};
#[cfg(test)]
use crate::interaction::FrontendToolStub;

#[cfg(test)]
mod tests;
