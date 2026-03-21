use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;
use tirea_protocol_ag_ui::{AgUiHistoryEncoder, AgUiProtocolEncoder, Event, RunAgentInput};

use super::runtime::apply_agui_extensions;
use crate::protocol::http_dialog::{
    handle_dialog_run, DialogRelayPlan, DialogRunAdapter, ErrorFormatter, RelayDoneCallback,
};

use crate::service::{
    encode_message_page, load_message_page, ApiError, AppState, MessageQueryParams, PreparedHttpRun,
};
use crate::transport::http_sse::sse_response;

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
    handle_dialog_run::<AgUiDialogAdapter>(st, agent_id, req).await
}

struct AgUiDialogAdapter;

#[async_trait]
impl DialogRunAdapter for AgUiDialogAdapter {
    type Request = RunAgentInput;
    type Encoder = AgUiProtocolEncoder;

    fn validate(req: &Self::Request) -> Result<(), ApiError> {
        req.validate()
            .map_err(|error| ApiError::BadRequest(error.to_string()))
    }

    fn thread_id(req: &Self::Request) -> &str {
        &req.thread_id
    }

    fn has_user_input(req: &Self::Request) -> bool {
        req.has_user_input()
    }

    fn suspension_decisions(
        req: &Self::Request,
    ) -> Vec<tirea_agentos::contracts::ToolCallDecision> {
        req.suspension_decisions()
    }

    fn frontend_run_id(req: &Self::Request) -> Option<&str> {
        Some(req.run_id.as_str())
    }

    fn apply_extensions(resolved: &mut tirea_agentos::runtime::ResolvedRun, req: &Self::Request) {
        apply_agui_extensions(resolved, req);
    }

    fn into_run_request(
        req: Self::Request,
        agent_id: String,
    ) -> tirea_agentos::contracts::RunRequest {
        req.into_runtime_run_request(agent_id)
    }

    async fn build_relay_plan(
        _st: &AppState,
        _prepared: &PreparedHttpRun,
        req: &Self::Request,
    ) -> Result<DialogRelayPlan<Self::Encoder>, ApiError> {
        let frontend_run_id = req.run_id.clone();
        let on_relay_done: RelayDoneCallback = Box::new(move |_sse_tx| Box::pin(async move {}));
        let error_formatter: ErrorFormatter = Arc::new(|msg| {
            let json =
                serde_json::to_string(&Event::run_error(&msg, Some("RELAY_ERROR".to_string())))
                    .unwrap_or_default();
            Bytes::from(format!("data: {json}\n\n"))
        });

        Ok(DialogRelayPlan {
            encoder: AgUiProtocolEncoder::new_with_frontend_run_id(frontend_run_id),
            fanout: None,
            resumable_downstream: false,
            protocol_label: "ag-ui",
            on_relay_done,
            error_formatter,
        })
    }

    fn into_response<S>(stream: S) -> Response
    where
        S: futures::Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + 'static,
    {
        sse_response(stream)
    }

    fn forwarded_decision_response(
        forwarded: tirea_agentos::runtime::ForwardedDecision,
        frontend_run_id: Option<&str>,
    ) -> Response {
        (
            StatusCode::ACCEPTED,
            Json(json!({
                "status": "decision_forwarded",
                "threadId": forwarded.thread_id,
                "runId": frontend_run_id,
            })),
        )
            .into_response()
    }
}
