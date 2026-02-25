use futures::StreamExt;
use std::sync::Arc;
use tirea_agentos::contracts::{AgentEvent, ToolCallDecision};
use tirea_agentos_server::transport::{
    ChannelDownstreamEndpoint, Endpoint, TranscoderEndpoint, TransportError,
};
use tirea_contract::ProtocolOutputEncoder;
use tokio::sync::mpsc;

#[derive(Default)]
struct StubEncoder {
    body_events: usize,
}

impl ProtocolOutputEncoder for StubEncoder {
    type InputEvent = AgentEvent;
    type Event = String;

    fn prologue(&mut self) -> Vec<Self::Event> {
        vec!["prologue-1".to_string(), "prologue-2".to_string()]
    }

    fn on_agent_event(&mut self, _ev: &Self::InputEvent) -> Vec<Self::Event> {
        self.body_events += 1;
        vec![format!("body-{}", self.body_events)]
    }

    fn epilogue(&mut self) -> Vec<Self::Event> {
        vec!["epilogue".to_string()]
    }
}

#[tokio::test]
async fn transcoder_recv_emits_prologue_body_and_epilogue_in_order() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, StubEncoder::default(), Ok);

    event_tx
        .send(AgentEvent::TextDelta {
            delta: "hello".to_string(),
        })
        .await
        .unwrap();
    event_tx.send(AgentEvent::StepEnd).await.unwrap();
    drop(event_tx);

    let stream = transcoder.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;

    assert_eq!(
        events,
        vec![
            "prologue-1".to_string(),
            "prologue-2".to_string(),
            "body-1".to_string(),
            "body-2".to_string(),
            "epilogue".to_string(),
        ]
    );
}

#[tokio::test]
async fn transcoder_recv_with_empty_stream_emits_prologue_and_epilogue() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, StubEncoder::default(), Ok);

    drop(_event_tx);

    let stream = transcoder.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;

    assert_eq!(
        events,
        vec!["prologue-1", "prologue-2", "epilogue"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn transcoder_recv_can_only_be_called_once() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(
        inner,
        StubEncoder::default(),
        Ok::<ToolCallDecision, TransportError>,
    );

    let _first = transcoder.recv().await.unwrap();
    let second = transcoder.recv().await;
    assert!(second.is_err());
}

#[tokio::test]
async fn transcoder_send_delegates_through_mapper() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, mut decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(
        inner,
        StubEncoder::default(),
        Ok::<ToolCallDecision, TransportError>,
    );

    let decision = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
    transcoder.send(decision.clone()).await.unwrap();

    let received = decision_rx.recv().await.unwrap();
    assert_eq!(received, decision);
}
