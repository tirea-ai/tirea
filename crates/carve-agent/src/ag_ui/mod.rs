//! AG-UI protocol support and request/runtime adapters.

mod request;
mod runner;

pub use crate::interaction::AgUiInteractionPlugin;
pub use carve_protocol_ag_ui::{
    build_context_addendum, convert_agui_messages, core_message_from_ag_ui,
    interaction_runtime_values, request_runtime_values, AGUIContext, AGUIContextEntry, AGUIEvent,
    AGUIMessage, AGUIToolDef, AgUiHistoryEncoder, AgUiInputAdapter, AgUiProtocolEncoder,
    BaseEventFields, MessageRole, RequestError, RunAgentRequest, ToolExecutionLocation,
};
pub use request::InteractionPlugin;
pub use runner::{
    run_agent_events_with_request, run_agent_stream, run_agent_stream_with_parent,
    run_agent_stream_with_request,
};
