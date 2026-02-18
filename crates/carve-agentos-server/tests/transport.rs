use carve_agentos::contracts::runtime::AgentEvent;
use carve_agentos_server::transport::pump_encoded_stream;
use carve_protocol_contract::ProtocolOutputEncoder;
use futures::{future::ready, stream};
use std::pin::Pin;

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

fn boxed_events(
    events: Vec<AgentEvent>,
) -> Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>> {
    Box::pin(stream::iter(events))
}

#[tokio::test]
async fn pump_encoded_stream_emits_prologue_body_and_epilogue_in_order() {
    let events = boxed_events(vec![
        AgentEvent::TextDelta {
            delta: "hello".to_string(),
        },
        AgentEvent::StepEnd,
    ]);

    let mut sent = Vec::new();
    pump_encoded_stream(events, StubEncoder::default(), |event| {
        sent.push(event);
        ready(Ok(()))
    })
    .await;

    assert_eq!(
        sent,
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
async fn pump_encoded_stream_stops_when_send_fails_in_prologue() {
    let events = boxed_events(vec![AgentEvent::TextDelta {
        delta: "hello".to_string(),
    }]);

    let mut sent = Vec::new();
    let mut calls = 0usize;
    pump_encoded_stream(events, StubEncoder::default(), |event| {
        calls += 1;
        sent.push(event);
        if calls == 1 {
            return ready(Err(()));
        }
        ready(Ok(()))
    })
    .await;

    assert_eq!(sent, vec!["prologue-1".to_string()]);
}

#[tokio::test]
async fn pump_encoded_stream_stops_when_send_fails_in_body() {
    let events = boxed_events(vec![
        AgentEvent::TextDelta {
            delta: "a".to_string(),
        },
        AgentEvent::TextDelta {
            delta: "b".to_string(),
        },
    ]);

    let mut sent = Vec::new();
    let mut calls = 0usize;
    pump_encoded_stream(events, StubEncoder::default(), |event| {
        calls += 1;
        sent.push(event);
        if calls == 3 {
            return ready(Err(()));
        }
        ready(Ok(()))
    })
    .await;

    assert_eq!(
        sent,
        vec![
            "prologue-1".to_string(),
            "prologue-2".to_string(),
            "body-1".to_string(),
        ]
    );
}
