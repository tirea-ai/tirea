//! AI SDK v6 compatible UI Message Stream types.
//!
//! This module provides types for the AI SDK v6 UI Message Stream protocol,
//! enabling direct integration with Vercel AI SDK frontend components.

mod adapter;
mod encoder;
mod events;
mod message;

/// Target AI SDK major version for this module.
pub const AI_SDK_VERSION: &str = "v6";

pub use adapter::{AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest};
pub use encoder::{
    AiSdkEncoder, DATA_EVENT_ACTIVITY_DELTA, DATA_EVENT_ACTIVITY_SNAPSHOT, DATA_EVENT_INTERACTION,
    DATA_EVENT_INTERACTION_REQUESTED, DATA_EVENT_INTERACTION_RESOLVED,
    DATA_EVENT_MESSAGES_SNAPSHOT, DATA_EVENT_STATE_DELTA, DATA_EVENT_STATE_SNAPSHOT,
};
pub use events::UIStreamEvent;
pub use message::{StreamState, ToolState, UIMessage, UIMessagePart, UIRole};

#[cfg(test)]
mod tests;
