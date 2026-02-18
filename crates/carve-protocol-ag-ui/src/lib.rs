//! AG-UI protocol support and adapters.
#![allow(missing_docs)]

mod context;
mod history_encoder;
mod input_adapter;
mod output_encoder;
mod protocol;
mod request;

pub use context::AGUIContext;
pub use history_encoder::AgUiHistoryEncoder;
pub use input_adapter::AgUiInputAdapter;
pub use output_encoder::AgUiProtocolEncoder;
pub use protocol::{interaction_to_ag_ui_events, AGUIEvent, BaseEventFields, MessageRole};
pub use request::{
    build_context_addendum, convert_agui_messages, core_message_from_ag_ui, AGUIContextEntry,
    AGUIMessage, AGUIToolDef, RequestError, RunAgentRequest, ToolExecutionLocation,
};
