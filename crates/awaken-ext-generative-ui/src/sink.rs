//! Forwarding sink: sub-agent TextDelta -> parent ActivityDelta.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use awaken_contract::contract::event::AgentEvent;
use awaken_contract::contract::event_sink::EventSink;
use serde_json::json;

/// Sink that intercepts sub-agent [`AgentEvent::TextDelta`] events and
/// re-emits them as [`AgentEvent::ActivityDelta`] on a parent sink.
///
/// Error events are forwarded as-is; all other events are silently dropped.
pub struct StreamingSubagentSink {
    call_id: String,
    activity_type: String,
    parent_sink: Arc<dyn EventSink>,
    buffer: Arc<Mutex<String>>,
}

impl StreamingSubagentSink {
    /// Create a sink and return a shared handle to the accumulated text buffer.
    pub fn new(
        call_id: String,
        activity_type: String,
        parent_sink: Arc<dyn EventSink>,
    ) -> (Self, Arc<Mutex<String>>) {
        let buffer = Arc::new(Mutex::new(String::new()));
        let sink = Self {
            call_id,
            activity_type,
            parent_sink,
            buffer: buffer.clone(),
        };
        (sink, buffer)
    }
}

#[async_trait]
impl EventSink for StreamingSubagentSink {
    async fn emit(&self, event: AgentEvent) {
        match &event {
            AgentEvent::TextDelta { delta } => {
                self.buffer.lock().expect("buffer poisoned").push_str(delta);
                self.parent_sink
                    .emit(AgentEvent::ActivityDelta {
                        message_id: self.call_id.clone(),
                        activity_type: self.activity_type.clone(),
                        patch: vec![json!({ "text_delta": delta })],
                    })
                    .await;
            }
            AgentEvent::Error { .. } => {
                self.parent_sink.emit(event).await;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::event_sink::VecEventSink;

    #[tokio::test]
    async fn forwards_text_delta_as_activity_delta() {
        let parent = Arc::new(VecEventSink::new());
        let (sink, buffer) =
            StreamingSubagentSink::new("call-1".into(), "gen-ui".into(), parent.clone());

        sink.emit(AgentEvent::TextDelta {
            delta: "Hello".into(),
        })
        .await;
        sink.emit(AgentEvent::TextDelta {
            delta: " world".into(),
        })
        .await;

        let events = parent.events();
        assert_eq!(events.len(), 2);

        // First ActivityDelta
        match &events[0] {
            AgentEvent::ActivityDelta {
                message_id,
                activity_type,
                patch,
            } => {
                assert_eq!(message_id, "call-1");
                assert_eq!(activity_type, "gen-ui");
                assert_eq!(patch.len(), 1);
                assert_eq!(patch[0]["text_delta"], "Hello");
            }
            other => panic!("expected ActivityDelta, got: {other:?}"),
        }

        // Second ActivityDelta
        match &events[1] {
            AgentEvent::ActivityDelta { patch, .. } => {
                assert_eq!(patch[0]["text_delta"], " world");
            }
            other => panic!("expected ActivityDelta, got: {other:?}"),
        }

        // Buffer accumulated both deltas
        let accumulated = buffer.lock().unwrap().clone();
        assert_eq!(accumulated, "Hello world");
    }

    #[tokio::test]
    async fn forwards_error_events() {
        let parent = Arc::new(VecEventSink::new());
        let (sink, _buffer) =
            StreamingSubagentSink::new("call-1".into(), "gen-ui".into(), parent.clone());

        sink.emit(AgentEvent::Error {
            message: "something broke".into(),
            code: Some("LLM_ERROR".into()),
        })
        .await;

        let events = parent.events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Error { message, code } => {
                assert_eq!(message, "something broke");
                assert_eq!(code.as_deref(), Some("LLM_ERROR"));
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn drops_other_events() {
        let parent = Arc::new(VecEventSink::new());
        let (sink, _buffer) =
            StreamingSubagentSink::new("call-1".into(), "gen-ui".into(), parent.clone());

        sink.emit(AgentEvent::StepStart {
            message_id: "m1".into(),
        })
        .await;
        sink.emit(AgentEvent::StepEnd).await;

        let events = parent.events();
        assert!(events.is_empty(), "non-text/error events should be dropped");
    }
}
