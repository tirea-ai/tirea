use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError};
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::RuntimeInput;
use tirea_protocol_ag_ui::{AgUiHistoryEncoder, AgUiProtocolEncoder, RunAgentInput};

use super::runtime::apply_agui_extensions;
use tokio::sync::mpsc;

use crate::service::{
    active_run_key, encode_message_page, load_message_page, register_active_run, remove_active_run,
    try_forward_decisions_to_active_run, ApiError, AppState, MessageQueryParams,
};
use crate::transport::http_run::wire_http_sse_relay;
use crate::transport::http_sse::{sse_body_stream, sse_response};
use crate::transport::runtime_endpoint::RunStarter;
use crate::transport::TransportError;

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
    let encoded = encode_message_page(page, AgUiHistoryEncoder::encode_message);
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
    let run_request = req.into_runtime_run_request(agent_id.clone());

    // Call prepare_run synchronously so storage errors surface as HTTP 500.
    let cancellation_token = RunCancellationToken::new();
    let prepared = st.os.prepare_run(run_request.clone(), resolved).await?;
    let thread_id = prepared.thread_id.clone();

    let token_for_starter = cancellation_token.clone();
    let starter: RunStarter = Box::new(move |_request| {
        Box::pin(async move {
            let run = AgentOs::execute_prepared(
                prepared.with_cancellation_token(token_for_starter.clone()),
            )
            .map_err(|e| TransportError::Internal(e.to_string()))?;
            Ok((run, Some(token_for_starter)))
        })
    });

    let active_key = active_run_key("ag_ui", &agent_id, &thread_id);
    let (ingress_tx, ingress_rx) = mpsc::unbounded_channel::<RuntimeInput>();
    ingress_tx
        .send(RuntimeInput::Run(run_request))
        .expect("ingress channel just created");
    register_active_run(active_key.clone(), ingress_tx).await;

    let enc = AgUiProtocolEncoder::new();
    let sse_rx = wire_http_sse_relay(
        starter,
        enc,
        ingress_rx,
        thread_id,
        None,
        false,
        "ag-ui",
        move |_sse_tx| async move {
            remove_active_run(&active_key).await;
        },
    );

    Ok(sse_response(sse_body_stream(sse_rx)))
}
