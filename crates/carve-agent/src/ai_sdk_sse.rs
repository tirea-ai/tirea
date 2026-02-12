//! AI SDK v6 SSE streaming helpers.
//!
//! Provides a high-level function to run the agent loop and emit events as
//! Server-Sent Events formatted for the Vercel AI SDK v6 UI Message Stream
//! protocol.
//!
//! # Example
//!
//! ```ignore
//! use carve_agent::ai_sdk_sse::{run_ai_sdk_sse, AiSdkSseStream};
//!
//! let AiSdkSseStream {
//!     events,
//!     checkpoints,
//!     final_session,
//! } = run_ai_sdk_sse(client, config, session, tools, run_id);
//!
//! // `events` is a Stream<Item = String> of SSE `data: {json}\n\n` lines.
//! // `checkpoints` can be drained for intermediate persistence.
//! // `final_session` resolves to the completed Session.
//! ```

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::{Stream, StreamExt};

use crate::r#loop::{
    run_loop_stream_with_checkpoints, AgentConfig, RunContext, SessionCheckpoint,
    StreamWithCheckpoints,
};
use crate::session::Session;
use crate::traits::tool::Tool;
use crate::ui_stream::{AiSdkAdapter, AiSdkEncoder};
use genai::Client;

/// Result of [`run_ai_sdk_sse`]: a stream of SSE-formatted JSON strings plus
/// checkpoint and final-session channels for persistence.
pub struct AiSdkSseStream {
    /// Stream of SSE data lines (`data: {json}\n\n`), including the
    /// `message-start`, lazily-opened `text-start`/`text-end` pairs around
    /// text content, and the framework-emitted `finish`.
    pub events: Pin<Box<dyn Stream<Item = String> + Send>>,

    /// Intermediate session checkpoints for crash-recovery persistence.
    pub checkpoints: tokio::sync::mpsc::UnboundedReceiver<SessionCheckpoint>,

    /// Resolves to the final [`Session`] when the stream completes.
    pub final_session: tokio::sync::oneshot::Receiver<Session>,

    /// The [`AiSdkAdapter`] used for this stream (exposes `message_id`,
    /// `text_id`, `run_id` for callers that need them).
    pub adapter: AiSdkAdapter,
}

/// Callback invoked for each [`AgentEvent`] before it is converted to SSE.
///
/// Use this to extract usage metrics or other side-channel data from the
/// event stream without post-processing the SSE output.
pub type EventHook = Box<dyn Fn(&crate::stream::AgentEvent) + Send>;

/// Run the carve-agent loop and produce an AI SDK v6 SSE event stream.
///
/// This wraps [`run_loop_stream_with_checkpoints`] and applies [`AiSdkEncoder`]
/// conversion, emitting the full AI SDK v6 framing:
///
/// 1. `message-start`
/// 2. `text-start` (lazy, on first `TextDelta`)
/// 3. Converted agent events (text-delta, tool-input-*, tool-output-*, etc.)
///    with automatic `text-end`/`text-start` around tool calls
/// 4. `text-end` + `finish` (on `RunFinish`)
///
/// The returned [`AiSdkSseStream`] also exposes `checkpoints` and
/// `final_session` for the caller to implement persistence.
pub fn run_ai_sdk_sse(
    client: Client,
    config: AgentConfig,
    session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_id: String,
) -> AiSdkSseStream {
    run_ai_sdk_sse_with_hook(client, config, session, tools, run_id, None, None)
}

/// Like [`run_ai_sdk_sse`] but with an optional parent run ID and an optional
/// per-event hook for metrics extraction.
pub fn run_ai_sdk_sse_with_hook(
    client: Client,
    config: AgentConfig,
    mut session: Session,
    tools: HashMap<String, Arc<dyn Tool>>,
    run_id: String,
    parent_run_id: Option<String>,
    event_hook: Option<EventHook>,
) -> AiSdkSseStream {
    let adapter = AiSdkAdapter::new(run_id.clone());

    // Set run_id and parent_run_id on the session runtime
    let _ = session.runtime.set("run_id", run_id.clone());
    if let Some(parent) = parent_run_id.clone() {
        let _ = session.runtime.set("parent_run_id", parent);
    }

    let StreamWithCheckpoints {
        events: mut agent_events,
        checkpoints,
        final_session,
    } = run_loop_stream_with_checkpoints(
        client,
        config,
        session,
        tools,
        RunContext {
            cancellation_token: None,
        },
    );

    let sse_stream = async_stream::stream! {
        let mut enc = AiSdkEncoder::new(run_id);

        // AI SDK v6 protocol framing: message-start (text-start is lazy).
        for e in enc.prologue() {
            if let Ok(json) = serde_json::to_string(&e) {
                yield format!("data: {}\n\n", json);
            }
        }

        while let Some(ev) = agent_events.next().await {
            if let Some(ref hook) = event_hook {
                hook(&ev);
            }
            for ui_event in enc.on_agent_event(&ev) {
                if let Ok(json) = serde_json::to_string(&ui_event) {
                    yield format!("data: {}\n\n", json);
                }
            }
        }
    };

    AiSdkSseStream {
        events: Box::pin(sse_stream),
        checkpoints,
        final_session,
        adapter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_stream::UIStreamEvent;

    #[test]
    fn test_ai_sdk_sse_stream_types() {
        // Verify that AiSdkSseStream fields are accessible.
        let adapter = AiSdkAdapter::new("test-run".to_string());
        assert_eq!(adapter.run_id(), "test-run");
    }

    #[test]
    fn test_sse_framing_format() {
        let event = UIStreamEvent::text_delta("txt_0", "Hello");
        let json = serde_json::to_string(&event).unwrap();
        let sse_line = format!("data: {}\n\n", json);
        assert!(sse_line.starts_with("data: "));
        assert!(sse_line.ends_with("\n\n"));
        assert!(sse_line.contains("text-delta"));
    }
}
