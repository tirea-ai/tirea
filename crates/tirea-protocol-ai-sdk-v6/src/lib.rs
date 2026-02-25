//! AI SDK v6 compatible UI Message Stream protocol support.
#![allow(missing_docs)]

mod encoder;
mod events;
mod history_encoder;
mod input_adapter;
mod message;

/// Target AI SDK major version for this module.
pub const AI_SDK_VERSION: &str = "v6";

pub use encoder::AiSdkEncoder;
pub use events::UIStreamEvent;
pub use history_encoder::AiSdkV6HistoryEncoder;
pub use input_adapter::{AiSdkTrigger, AiSdkV6RunRequest};
pub use message::{
    DataUIPart, FileUIPart, ReasoningUIPart, SourceDocumentUIPart, SourceUrlUIPart,
    StepStartUIPart, StreamState, TextUIPart, ToolApproval, ToolState, ToolUIPart, UIMessage,
    UIMessagePart, UIRole,
};
