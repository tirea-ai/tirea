use serde::Serialize;
use tirea_agentos::contracts::{AgentEvent, RunRequest};
use tirea_agentos::orchestrator::{AgentOs, ResolvedRun};
use tirea_contract::ProtocolOutputEncoder;

use crate::protocol::NatsProtocolError;
use crate::transport::pump_encoded_stream;

pub async fn run_and_publish<E, ErrEvent, BuildEncoder, BuildErrorEvent>(
    os: &AgentOs,
    run_request: RunRequest,
    resolved: ResolvedRun,
    reply: async_nats::Subject,
    client: async_nats::Client,
    build_encoder: BuildEncoder,
    build_error_event: BuildErrorEvent,
) -> Result<(), NatsProtocolError>
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Serialize + Send + 'static,
    ErrEvent: Serialize,
    BuildEncoder: FnOnce(&tirea_agentos::orchestrator::RunStream) -> E,
    BuildErrorEvent: FnOnce(String) -> ErrEvent,
{
    let run = match os.prepare_run(run_request, resolved).await {
        Ok(prepared) => match AgentOs::execute_prepared(prepared) {
            Ok(run) => run,
            Err(err) => {
                let event = build_error_event(err.to_string());
                let payload = serde_json::to_vec(&event)
                    .map_err(|e| {
                        NatsProtocolError::Run(format!("serialize error event failed: {e}"))
                    })?
                    .into();
                if let Err(publish_err) = client.publish(reply, payload).await {
                    return Err(NatsProtocolError::Run(format!(
                        "publish error event failed: {publish_err}"
                    )));
                }
                return Ok(());
            }
        },
        Err(err) => {
            let event = build_error_event(err.to_string());
            let payload = serde_json::to_vec(&event)
                .map_err(|e| NatsProtocolError::Run(format!("serialize error event failed: {e}")))?
                .into();
            if let Err(publish_err) = client.publish(reply, payload).await {
                return Err(NatsProtocolError::Run(format!(
                    "publish error event failed: {publish_err}"
                )));
            }
            return Ok(());
        }
    };

    let encoder = build_encoder(&run);
    pump_encoded_stream(run.events, encoder, move |event| {
        let client = client.clone();
        let reply = reply.clone();
        async move {
            let payload = match serde_json::to_vec(&event) {
                Ok(payload) => payload.into(),
                Err(err) => {
                    tracing::warn!(error = %err, "failed to serialize NATS protocol event");
                    return Err(());
                }
            };
            client.publish(reply, payload).await.map_err(|_| ())
        }
    })
    .await;

    Ok(())
}
