use carve_agent::protocol::ProtocolOutputEncoder;
use carve_agent::AgentEvent;
use futures::Stream;
use futures::StreamExt;
use std::future::Future;
use std::pin::Pin;

/// Pump an internal agent event stream through a protocol output encoder and
/// forward each encoded event to the provided async sink callback.
pub async fn pump_encoded_stream<E, SendFn, SendFut>(
    mut events: Pin<Box<dyn Stream<Item = AgentEvent> + Send>>,
    mut encoder: E,
    mut send: SendFn,
) where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent>,
    SendFn: FnMut(E::Event) -> SendFut,
    SendFut: Future<Output = Result<(), ()>>,
{
    for event in encoder.prologue() {
        if send(event).await.is_err() {
            return;
        }
    }

    while let Some(agent_event) = events.next().await {
        for event in encoder.on_agent_event(&agent_event) {
            if send(event).await.is_err() {
                return;
            }
        }
    }

    for event in encoder.epilogue() {
        if send(event).await.is_err() {
            return;
        }
    }
}
