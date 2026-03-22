//! Protocol-specific event encoding utilities.

use awaken_contract::contract::event::AgentEvent;
use awaken_contract::contract::transport::Transcoder;
use bytes::Bytes;
use serde::Serialize;

use crate::http_sse::format_sse_data;

/// Encode a single agent event through a transcoder into SSE data frames.
pub fn encode_event_to_sse<E>(encoder: &mut E, event: &AgentEvent) -> Vec<Bytes>
where
    E: Transcoder<Input = AgentEvent>,
    E::Output: Serialize,
{
    encoder
        .transcode(event)
        .into_iter()
        .filter_map(|item| {
            serde_json::to_string(&item)
                .ok()
                .map(|json| format_sse_data(&json))
        })
        .collect()
}

/// Encode prologue events from a transcoder into SSE data frames.
pub fn encode_prologue_to_sse<E>(encoder: &mut E) -> Vec<Bytes>
where
    E: Transcoder<Input = AgentEvent>,
    E::Output: Serialize,
{
    encoder
        .prologue()
        .into_iter()
        .filter_map(|item| {
            serde_json::to_string(&item)
                .ok()
                .map(|json| format_sse_data(&json))
        })
        .collect()
}

/// Encode epilogue events from a transcoder into SSE data frames.
pub fn encode_epilogue_to_sse<E>(encoder: &mut E) -> Vec<Bytes>
where
    E: Transcoder<Input = AgentEvent>,
    E::Output: Serialize,
{
    encoder
        .epilogue()
        .into_iter()
        .filter_map(|item| {
            serde_json::to_string(&item)
                .ok()
                .map(|json| format_sse_data(&json))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::transport::Identity;

    #[test]
    fn encode_identity_event_to_sse() {
        let mut encoder = Identity::<AgentEvent>::default();
        let event = AgentEvent::TextDelta { delta: "hi".into() };
        let frames = encode_event_to_sse(&mut encoder, &event);
        assert_eq!(frames.len(), 1);
        let frame = String::from_utf8(frames[0].to_vec()).unwrap();
        assert!(frame.starts_with("data: "));
        assert!(frame.contains("text_delta"));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn encode_prologue_empty_for_identity() {
        let mut encoder = Identity::<AgentEvent>::default();
        let frames = encode_prologue_to_sse(&mut encoder);
        assert!(frames.is_empty());
    }

    #[test]
    fn encode_epilogue_empty_for_identity() {
        let mut encoder = Identity::<AgentEvent>::default();
        let frames = encode_epilogue_to_sse(&mut encoder);
        assert!(frames.is_empty());
    }

    #[test]
    fn encode_ai_sdk_event_to_sse() {
        use crate::protocols::ai_sdk_v6::encoder::AiSdkEncoder;
        let mut encoder = AiSdkEncoder::new();
        let event = AgentEvent::TextDelta {
            delta: "hello".into(),
        };
        let frames = encode_event_to_sse(&mut encoder, &event);
        assert!(!frames.is_empty());
        let frame = String::from_utf8(frames[0].to_vec()).unwrap();
        assert!(frame.starts_with("data: "));
    }

    #[test]
    fn encode_ag_ui_event_to_sse() {
        use crate::protocols::ag_ui::encoder::AgUiEncoder;
        let mut encoder = AgUiEncoder::new();
        encoder.on_agent_event(&AgentEvent::RunStart {
            thread_id: "t1".into(),
            run_id: "r1".into(),
            parent_run_id: None,
        });
        let event = AgentEvent::TextDelta {
            delta: "hello".into(),
        };
        let frames = encode_event_to_sse(&mut encoder, &event);
        assert!(!frames.is_empty());
    }

    #[test]
    fn encode_acp_event_to_sse() {
        use crate::protocols::acp::encoder::AcpEncoder;
        let mut encoder = AcpEncoder::new();
        let event = AgentEvent::TextDelta {
            delta: "hello".into(),
        };
        let frames = encode_event_to_sse(&mut encoder, &event);
        assert_eq!(frames.len(), 1);
    }
}
