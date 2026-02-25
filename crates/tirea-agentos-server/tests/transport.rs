use futures::StreamExt;
use std::sync::Arc;
use tirea_agentos::contracts::{AgentEvent, ToolCallDecision};
use tirea_agentos_server::transport::{
    channel_pair, relay_binding, ChannelDownstreamEndpoint, Endpoint, RelayCancellation,
    SessionId, TranscoderEndpoint, TransportBinding, TransportCapabilities,
};
use tirea_contract::{Identity, Transcoder};
use tokio::sync::mpsc;

// ── Stub encoder ────────────────────────────────────────────────

#[derive(Default)]
struct StubEncoder {
    body_events: usize,
}

impl Transcoder for StubEncoder {
    type Input = AgentEvent;
    type Output = String;

    fn prologue(&mut self) -> Vec<String> {
        vec!["prologue-1".to_string(), "prologue-2".to_string()]
    }

    fn transcode(&mut self, _item: &AgentEvent) -> Vec<String> {
        self.body_events += 1;
        vec![format!("body-{}", self.body_events)]
    }

    fn epilogue(&mut self) -> Vec<String> {
        vec!["epilogue".to_string()]
    }
}

/// Encoder that fans out multiple events per agent event.
struct FanoutEncoder;

impl Transcoder for FanoutEncoder {
    type Input = AgentEvent;
    type Output = String;

    fn prologue(&mut self) -> Vec<String> {
        vec!["start".to_string()]
    }

    fn transcode(&mut self, item: &AgentEvent) -> Vec<String> {
        match item {
            AgentEvent::TextDelta { delta } => {
                vec![format!("text:{delta}"), format!("echo:{delta}")]
            }
            _ => vec!["other".to_string()],
        }
    }

    fn epilogue(&mut self) -> Vec<String> {
        vec!["end".to_string()]
    }
}

/// Encoder that emits nothing for prologue/epilogue.
struct MinimalEncoder;

impl Transcoder for MinimalEncoder {
    type Input = AgentEvent;
    type Output = String;

    fn transcode(&mut self, _item: &AgentEvent) -> Vec<String> {
        vec!["evt".to_string()]
    }
}

#[tokio::test]
async fn transcoder_recv_emits_prologue_body_and_epilogue_in_order() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

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
    let transcoder = TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

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
    let transcoder: TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>> =
        TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

    let _first = transcoder.recv().await.unwrap();
    let second = transcoder.recv().await;
    assert!(second.is_err());
}

#[tokio::test]
async fn transcoder_send_delegates_through_identity() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, mut decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder: TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>> =
        TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

    let decision = ToolCallDecision::resume("tc1", serde_json::Value::Null, 0);
    transcoder.send(decision.clone()).await.unwrap();

    let received = decision_rx.recv().await.unwrap();
    assert_eq!(received, decision);
}

// ── TranscoderEndpoint: custom send transcoder ──────────────────

/// Custom send transcoder: receives a String, produces a ToolCallDecision.
struct StringToDecisionTranscoder;

impl Transcoder for StringToDecisionTranscoder {
    type Input = String;
    type Output = ToolCallDecision;

    fn transcode(&mut self, item: &String) -> Vec<ToolCallDecision> {
        vec![ToolCallDecision::resume(
            item,
            serde_json::Value::Null,
            0,
        )]
    }
}

#[tokio::test]
async fn transcoder_send_with_custom_transcoder_transforms_input() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, mut decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));

    let transcoder =
        TranscoderEndpoint::new(inner, StubEncoder::default(), StringToDecisionTranscoder);

    transcoder.send("custom-tc".to_string()).await.unwrap();
    let received = decision_rx.recv().await.unwrap();
    assert_eq!(
        received,
        ToolCallDecision::resume("custom-tc", serde_json::Value::Null, 0)
    );
}

// ── TranscoderEndpoint: close delegates ─────────────────────────

#[tokio::test]
async fn transcoder_close_delegates_to_inner() {
    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder: TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>> =
        TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

    let result = transcoder.close().await;
    assert!(result.is_ok());
}

// ── TranscoderEndpoint: encoder variations ──────────────────────

#[tokio::test]
async fn transcoder_fanout_encoder_emits_multiple_events_per_input() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, FanoutEncoder, Identity::default());

    event_tx
        .send(AgentEvent::TextDelta {
            delta: "hi".to_string(),
        })
        .await
        .unwrap();
    event_tx.send(AgentEvent::StepEnd).await.unwrap();
    drop(event_tx);

    let stream = transcoder.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;

    assert_eq!(
        events,
        vec!["start", "text:hi", "echo:hi", "other", "end"]
    );
}

#[tokio::test]
async fn transcoder_minimal_encoder_no_prologue_no_epilogue() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, MinimalEncoder, Identity::default());

    event_tx
        .send(AgentEvent::TextDelta {
            delta: "x".to_string(),
        })
        .await
        .unwrap();
    drop(event_tx);

    let stream = transcoder.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;
    assert_eq!(events, vec!["evt"]);
}

#[tokio::test]
async fn transcoder_recv_many_events_in_order() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(32);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let inner = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));
    let transcoder = TranscoderEndpoint::new(inner, StubEncoder::default(), Identity::default());

    for _ in 0..10 {
        event_tx
            .send(AgentEvent::TextDelta {
                delta: "d".to_string(),
            })
            .await
            .unwrap();
    }
    drop(event_tx);

    let stream = transcoder.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;

    // prologue(2) + body(10) + epilogue(1) = 13
    assert_eq!(events.len(), 13);
    assert_eq!(&events[0], "prologue-1");
    assert_eq!(&events[1], "prologue-2");
    for i in 0..10 {
        assert_eq!(events[2 + i], format!("body-{}", i + 1));
    }
    assert_eq!(&events[12], "epilogue");
}

// ── Composition: transcoder + relay + channel_pair ──────────────

#[tokio::test]
async fn transcoder_through_channel_pair_relay_pipeline() {
    // Simulate the real protocol handler pattern:
    //   runtime_ep (ChannelDownstream) → TranscoderEndpoint → relay → channel_pair.server
    //   client reads from channel_pair.client

    // Runtime side: feed AgentEvents
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let runtime_ep = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));

    // Protocol transcoder
    let transcoder: Arc<
        TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>>,
    > = Arc::new(TranscoderEndpoint::new(
        runtime_ep,
        StubEncoder::default(),
        Identity::default(),
    ));

    // Transport channel pair
    let pair = channel_pair::<ToolCallDecision, String>(16);

    // Wire: upstream = pair.server, downstream = transcoder
    let binding = TransportBinding {
        session: SessionId {
            thread_id: "pipeline-test".to_string(),
        },
        caps: TransportCapabilities {
            upstream_async: true,
            downstream_streaming: true,
            single_channel_bidirectional: false,
            resumable_downstream: false,
        },
        upstream: pair.server,
        downstream: transcoder,
    };

    let cancel = RelayCancellation::new();
    let relay = tokio::spawn(relay_binding(binding, cancel));

    // Produce agent events
    event_tx
        .send(AgentEvent::TextDelta {
            delta: "hello".to_string(),
        })
        .await
        .unwrap();
    event_tx.send(AgentEvent::StepEnd).await.unwrap();
    drop(event_tx);

    // Read transcoded events from client side of the pair
    let stream = pair.client.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;

    assert_eq!(
        events,
        vec!["prologue-1", "prologue-2", "body-1", "body-2", "epilogue"]
    );

    let result = relay.await.unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn transcoder_pipeline_send_direction() {
    // Verify send direction: client → channel_pair → relay → transcoder → runtime decision_tx

    let (_event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, mut decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let runtime_ep = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));

    let transcoder: Arc<
        TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>>,
    > = Arc::new(TranscoderEndpoint::new(
        runtime_ep,
        StubEncoder::default(),
        Identity::default(),
    ));

    let pair = channel_pair::<ToolCallDecision, String>(16);

    let binding = TransportBinding {
        session: SessionId {
            thread_id: "send-test".to_string(),
        },
        caps: TransportCapabilities::default(),
        upstream: pair.server,
        downstream: transcoder,
    };

    let cancel = RelayCancellation::new();
    let relay = tokio::spawn(relay_binding(binding, cancel));

    // Client sends a decision through the pair
    let decision = ToolCallDecision::resume("tc-pipe", serde_json::Value::Null, 0);
    pair.client.send(decision.clone()).await.unwrap();

    // It should arrive at the runtime's decision_rx
    let received = decision_rx.recv().await.unwrap();
    assert_eq!(received, decision);

    // Close everything
    drop(_event_tx);
    drop(pair.client);
    let result = relay.await.unwrap();
    assert!(result.is_ok());
}

#[tokio::test]
async fn transcoder_pipeline_bidirectional_concurrent() {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(8);
    let (decision_tx, mut decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    let runtime_ep = Arc::new(ChannelDownstreamEndpoint::new(event_rx, decision_tx));

    let transcoder: Arc<
        TranscoderEndpoint<StubEncoder, Identity<ToolCallDecision>>,
    > = Arc::new(TranscoderEndpoint::new(
        runtime_ep,
        StubEncoder::default(),
        Identity::default(),
    ));

    let pair = channel_pair::<ToolCallDecision, String>(16);

    let binding = TransportBinding {
        session: SessionId {
            thread_id: "bidir".to_string(),
        },
        caps: TransportCapabilities::default(),
        upstream: pair.server,
        downstream: transcoder,
    };

    let cancel = RelayCancellation::new();
    let relay = tokio::spawn(relay_binding(binding, cancel));

    // Concurrently: push agent events + send decisions
    let producer = tokio::spawn(async move {
        event_tx
            .send(AgentEvent::TextDelta {
                delta: "a".to_string(),
            })
            .await
            .unwrap();
        event_tx
            .send(AgentEvent::TextDelta {
                delta: "b".to_string(),
            })
            .await
            .unwrap();
        drop(event_tx);
    });

    let decision = ToolCallDecision::resume("bidir-tc", serde_json::json!({"ok": true}), 0);
    pair.client.send(decision.clone()).await.unwrap();

    // Verify decision arrived
    let received = decision_rx.recv().await.unwrap();
    assert_eq!(received, decision);

    // Verify transcoded events arrived
    producer.await.unwrap();
    let stream = pair.client.recv().await.unwrap();
    let events: Vec<String> = stream.map(|r| r.unwrap()).collect().await;
    assert_eq!(
        events,
        vec!["prologue-1", "prologue-2", "body-1", "body-2", "epilogue"]
    );

    let result = relay.await.unwrap();
    assert!(result.is_ok());
}
