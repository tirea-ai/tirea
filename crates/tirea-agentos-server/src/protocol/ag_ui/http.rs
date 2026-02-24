use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tirea_agentos::contracts::AgentEvent;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError, RunStream};
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::{ProtocolInputAdapter, ProtocolOutputEncoder};
use tirea_protocol_ag_ui::{
    apply_agui_extensions, AgUiHistoryEncoder, AgUiInputAdapter, AgUiProtocolEncoder, RunAgentInput,
};
use tracing::warn;

use crate::service::{
    active_run_key, encode_message_page, load_message_page, register_active_run, remove_active_run,
    try_forward_decisions_to_active_run, ApiError, AppState, MessageQueryParams,
};
use crate::transport::pump_encoded_stream;

/// AG-UI run endpoint path (to be nested under protocol root).
pub const RUN_PATH: &str = "/agents/:agent_id/runs";
/// AG-UI history endpoint path (to be nested under protocol root).
pub const THREAD_MESSAGES_PATH: &str = "/threads/:id/messages";

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
                StatusCode::ACCEPTED,
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
    register_active_run(active_key.clone(), run.decision_tx.clone()).await;

    let enc = AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone());
    let body_stream = encoded_sse_body(run, enc, cancellation_token, active_key);

    Ok(sse_response(body_stream))
}

fn encoded_sse_body<E>(
    run: RunStream,
    encoder: E,
    cancellation_token: RunCancellationToken,
    active_key: String,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: serde::Serialize + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let tx_events = tx.clone();
        let event_cancel_token = cancellation_token.clone();
        let downstream_closed = Arc::new(AtomicBool::new(false));
        let send_state = downstream_closed.clone();
        pump_encoded_stream(run.events, encoder, move |event| {
            let tx = tx_events.clone();
            let token = event_cancel_token.clone();
            let closed = send_state.clone();
            async move {
                let json = match serde_json::to_string(&event) {
                    Ok(json) => json,
                    Err(err) => {
                        warn!(error = %err, "failed to serialize SSE protocol event");
                        closed.store(true, Ordering::Relaxed);
                        token.cancel();
                        return Ok(());
                    }
                };
                let chunk = Bytes::from(format!("data: {json}\n\n"));

                if closed.load(Ordering::Relaxed) {
                    return Ok(());
                }

                match tx.send(chunk).await {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        closed.store(true, Ordering::Relaxed);
                        token.cancel();
                        Ok(())
                    }
                }
            }
        })
        .await;

        remove_active_run(&active_key).await;
    });

    async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, Infallible>(chunk);
        }
    }
}

fn sse_response<S>(stream: S) -> Response
where
    S: futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static,
{
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    (headers, Body::from_stream(stream)).into_response()
}
