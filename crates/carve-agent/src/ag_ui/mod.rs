//! AG-UI protocol support and request/runtime adapters.

pub use crate::interaction::AgUiInteractionPlugin;
pub use carve_protocol_ag_ui::{
    build_context_addendum, convert_agui_messages, core_message_from_ag_ui, AGUIContext,
    AGUIContextEntry, AGUIEvent, AGUIMessage, AGUIToolDef, AgUiHistoryEncoder, AgUiInputAdapter,
    AgUiProtocolEncoder, BaseEventFields, MessageRole, RequestError, RunAgentRequest,
    ToolExecutionLocation,
};

impl AgUiInteractionPlugin {
    /// Build interaction plugin from AG-UI request fields.
    ///
    /// This compatibility constructor is kept for existing tests/callers while
    /// execution now goes through `AgentOs::run_stream`.
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
