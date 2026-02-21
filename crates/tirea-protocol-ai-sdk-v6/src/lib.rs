//! AI SDK v6 compatible UI Message Stream protocol support.
#![allow(missing_docs)]

mod encoder;
mod events;
mod history_encoder;
mod input_adapter;
mod message;
mod output_encoder;
mod runtime;

/// Target AI SDK major version for this module.
pub const AI_SDK_VERSION: &str = "v6";

pub use encoder::{
    AiSdkEncoder, DATA_EVENT_ACTIVITY_DELTA, DATA_EVENT_ACTIVITY_SNAPSHOT,
    DATA_EVENT_INFERENCE_COMPLETE, DATA_EVENT_MESSAGES_SNAPSHOT, DATA_EVENT_REASONING_ENCRYPTED,
    DATA_EVENT_STATE_DELTA, DATA_EVENT_STATE_SNAPSHOT,
};
pub use events::UIStreamEvent;
pub use history_encoder::AiSdkV6HistoryEncoder;
pub use input_adapter::{AiSdkTrigger, AiSdkV6InputAdapter, AiSdkV6RunRequest};
pub use message::{
    DataUIPart, FileUIPart, ReasoningUIPart, SourceDocumentUIPart, SourceUrlUIPart,
    StepStartUIPart, StreamState, TextUIPart, ToolApproval, ToolState, ToolUIPart, UIMessage,
    UIMessagePart, UIRole,
};
pub use output_encoder::AiSdkV6ProtocolEncoder;
pub use runtime::apply_ai_sdk_extensions;
