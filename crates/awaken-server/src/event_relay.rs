//! Transport-agnostic event relay pipeline.
//!
//! Consumes [`AgentEvent`]s from a channel, runs them through a [`Transcoder`],
//! and yields serialized output items as a stream. Both SSE and NATS transports
//! consume this shared pipeline.

use serde::Serialize;
use tokio::sync::mpsc;

use awaken_contract::contract::event::AgentEvent;
use awaken_contract::contract::transport::Transcoder;

/// Produce a stream of transcoded output items from an event receiver.
///
/// Runs the full prologue → transcode → epilogue pipeline. Each output item
/// is serialized to a JSON `Vec<u8>`. The stream ends when the event channel
/// closes and the epilogue has been emitted.
pub fn relay_events<E>(
    mut event_rx: mpsc::UnboundedReceiver<AgentEvent>,
    mut encoder: E,
) -> impl futures::Stream<Item = Vec<u8>> + Send + 'static
where
    E: Transcoder<Input = AgentEvent> + 'static,
    E::Output: Serialize + Send + 'static,
{
    async_stream::stream! {
        // Emit prologue
        for item in encoder.prologue() {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                yield bytes;
            }
        }

        // Transcode each agent event
        while let Some(event) = event_rx.recv().await {
            for item in encoder.transcode(&event) {
                if let Ok(bytes) = serde_json::to_vec(&item) {
                    yield bytes;
                }
            }
        }

        // Emit epilogue
        for item in encoder.epilogue() {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                yield bytes;
            }
        }
    }
}

/// Produce a stream of transcoded output items from a **bounded** event receiver.
///
/// Identical to [`relay_events`] but accepts a bounded channel, suitable for
/// transports where back-pressure is desired (e.g. NATS).
pub fn relay_events_bounded<E>(
    mut event_rx: mpsc::Receiver<AgentEvent>,
    mut encoder: E,
) -> impl futures::Stream<Item = Vec<u8>> + Send + 'static
where
    E: Transcoder<Input = AgentEvent> + 'static,
    E::Output: Serialize + Send + 'static,
{
    async_stream::stream! {
        for item in encoder.prologue() {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                yield bytes;
            }
        }

        while let Some(event) = event_rx.recv().await {
            for item in encoder.transcode(&event) {
                if let Ok(bytes) = serde_json::to_vec(&item) {
                    yield bytes;
                }
            }
        }

        for item in encoder.epilogue() {
            if let Ok(bytes) = serde_json::to_vec(&item) {
                yield bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::transport::Identity;
    use futures::StreamExt;

    #[tokio::test]
    async fn relay_events_identity_transcoder() {
        let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
        let encoder = Identity::<AgentEvent>::default();
        let stream = relay_events(rx, encoder);
        tokio::pin!(stream);

        tx.send(AgentEvent::TextDelta {
            delta: "hello".into(),
        })
        .unwrap();
        drop(tx);

        let items: Vec<Vec<u8>> = stream.collect().await;
        assert_eq!(items.len(), 1);
        let json = String::from_utf8(items[0].clone()).unwrap();
        assert!(json.contains("text_delta"));
        assert!(json.contains("hello"));
    }

    #[tokio::test]
    async fn relay_events_with_prologue_epilogue() {
        use serde_json::Value;

        struct TestTranscoder;
        impl Transcoder for TestTranscoder {
            type Input = AgentEvent;
            type Output = Value;

            fn prologue(&mut self) -> Vec<Value> {
                vec![serde_json::json!({"type": "start"})]
            }

            fn transcode(&mut self, _item: &AgentEvent) -> Vec<Value> {
                vec![serde_json::json!({"type": "event"})]
            }

            fn epilogue(&mut self) -> Vec<Value> {
                vec![serde_json::json!({"type": "end"})]
            }
        }

        let (tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
        let stream = relay_events(rx, TestTranscoder);
        tokio::pin!(stream);

        tx.send(AgentEvent::StepEnd).unwrap();
        drop(tx);

        let items: Vec<Vec<u8>> = stream.collect().await;
        assert_eq!(items.len(), 3);

        let first = String::from_utf8(items[0].clone()).unwrap();
        assert!(first.contains("start"));
        let last = String::from_utf8(items[2].clone()).unwrap();
        assert!(last.contains("end"));
    }

    #[tokio::test]
    async fn relay_events_empty_stream() {
        let (_tx, rx) = mpsc::unbounded_channel::<AgentEvent>();
        let encoder = Identity::<AgentEvent>::default();
        let stream = relay_events(rx, encoder);
        tokio::pin!(stream);

        drop(_tx);

        let items: Vec<Vec<u8>> = stream.collect().await;
        // Identity has no prologue/epilogue, no events = empty
        assert!(items.is_empty());
    }
}
