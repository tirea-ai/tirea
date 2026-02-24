use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;
use tirea_agentos::contracts::ToolCallDecision;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError};
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::ProtocolInputAdapter;
use tirea_protocol_ag_ui::{
    apply_agui_extensions, AgUiHistoryEncoder, AgUiInputAdapter, AgUiProtocolEncoder, RunAgentInput,
};
use tokio::sync::mpsc;
use tracing::warn;

use crate::service::{
    active_run_key, encode_message_page, load_message_page, register_active_run, remove_active_run,
    try_forward_decisions_to_active_run, ApiError, AppState, MessageQueryParams,
};
use crate::transport::http_sse::{sse_body_stream, sse_response, HttpSseUpstreamEndpoint};
use crate::transport::{
    pump_encoded_stream, relay_binding, ChannelDownstreamEndpoint, RelayCancellation, SessionId,
    TransportBinding, TransportCapabilities,
};

const RUN_PATH: &str = "/agents/:agent_id/runs";
const THREAD_MESSAGES_PATH: &str = "/threads/:id/messages";

/// Build AG-UI HTTP routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(RUN_PATH, post(run))
        .route(THREAD_MESSAGES_PATH, get(thread_messages))
}

async fn thread_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.read_store, &id, &params).await?;
    let encoded = encode_message_page::<AgUiHistoryEncoder>(page);
    Ok(Json(encoded))
}

async fn run(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<RunAgentInput>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let suspension_decisions = req.suspension_decisions();
    let decision_only = !req.has_user_input() && !suspension_decisions.is_empty();
    if decision_only {
        let key = active_run_key("ag_ui", &agent_id, &req.thread_id);
        if try_forward_decisions_to_active_run(&key, suspension_decisions).await {
            return Ok((
                axum::http::StatusCode::ACCEPTED,
                Json(json!({
                    "status": "decision_forwarded",
                    "threadId": req.thread_id,
                    "runId": req.run_id,
                })),
            )
                .into_response());
        }
    }

    let mut resolved = st.os.resolve(&agent_id).map_err(AgentOsRunError::from)?;
    apply_agui_extensions(&mut resolved, &req);
    let run_request = AgUiInputAdapter::to_run_request(agent_id.clone(), req);
    let cancellation_token = RunCancellationToken::new();
    let prepared = st.os.prepare_run(run_request, resolved).await?;
    let run =
        AgentOs::execute_prepared(prepared.with_cancellation_token(cancellation_token.clone()))?;

    let active_key = active_run_key("ag_ui", &agent_id, &run.thread_id);
    let (decision_ingress_tx, decision_ingress_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
    register_active_run(active_key.clone(), decision_ingress_tx).await;

    let thread_id = run.thread_id.clone();
    let enc = AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone());
    let (sse_tx, sse_rx) = mpsc::channel::<Bytes>(64);
    let upstream = Arc::new(HttpSseUpstreamEndpoint::new(
        decision_ingress_rx,
        sse_tx,
        cancellation_token.clone(),
    ));
    let decision_tx = run.decision_tx.clone();
    let (tx, rx) = mpsc::channel::<Bytes>(64);
    tokio::spawn(async move {
        let tx_events = tx.clone();
        let event_cancel_token = cancellation_token.clone();
        pump_encoded_stream(run.events, enc, move |event| {
            let tx = tx_events.clone();
            let token = event_cancel_token.clone();
            async move {
                let json = match serde_json::to_string(&event) {
                    Ok(json) => json,
                    Err(err) => {
                        warn!(error = %err, "failed to serialize SSE protocol event");
                        token.cancel();
                        return Ok(());
                    }
                };
                let chunk = Bytes::from(format!("data: {json}\n\n"));
                tx.send(chunk).await.map_err(|_| {
                    token.cancel();
                })
            }
        })
        .await;
    });
    let downstream = Arc::new(ChannelDownstreamEndpoint::new(rx, decision_tx));
    let binding = TransportBinding {
        session: SessionId { thread_id },
        caps: TransportCapabilities {
            upstream_async: true,
            downstream_streaming: true,
            single_channel_bidirectional: false,
            resumable_downstream: false,
        },
        upstream,
        downstream,
    };
    let relay_cancel = RelayCancellation::new();
    tokio::spawn(async move {
        if let Err(err) = relay_binding(binding, relay_cancel.clone()).await {
            warn!(error = %err, "ag-ui transport relay failed");
        }
        remove_active_run(&active_key).await;
    });

    Ok(sse_response(sse_body_stream(sse_rx)))
}
