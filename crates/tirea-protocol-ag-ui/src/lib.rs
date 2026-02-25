//! AG-UI protocol support, adapters, and encoding.
#![allow(missing_docs)]

mod context;
pub mod events;
mod history_encoder;
mod input_adapter;
mod output_encoder;
pub mod types;

pub use context::AgUiEventContext;
pub use events::{interaction_to_ag_ui_events, BaseEvent, Event, ReasoningEncryptedValueSubtype};
pub use history_encoder::AgUiHistoryEncoder;
pub use input_adapter::AgUiInputAdapter;
pub use output_encoder::AgUiProtocolEncoder;
pub use types::{
    build_context_addendum, convert_agui_messages, core_message_from_ag_ui, Context, Message,
    RequestError, Role, RunAgentInput, Tool, ToolExecutionLocation,
};
