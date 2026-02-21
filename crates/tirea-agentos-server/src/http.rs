use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tirea_agentos::contracts::storage::{
    AgentStateListPage, AgentStateListQuery, AgentStateReader, MessagePage, MessageQuery, SortOrder,
};
use tirea_agentos::contracts::thread::{Thread, Visibility};
use tirea_agentos::contracts::AgentEvent;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError, RunStream};
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::{ProtocolHistoryEncoder, ProtocolInputAdapter, ProtocolOutputEncoder};
use tirea_protocol_ag_ui::{
    apply_agui_extensions, AgUiHistoryEncoder, AgUiInputAdapter, AgUiProtocolEncoder, RunAgentInput,
};
use tirea_protocol_ai_sdk_v6::{
    apply_ai_sdk_extensions, AiSdkV6HistoryEncoder, AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder,
    AiSdkV6RunRequest, AI_SDK_VERSION,
};
use tracing::warn;

use crate::transport::pump_encoded_stream;

#[derive(Clone)]
pub struct AppState {
    pub os: Arc<AgentOs>,
    pub read_store: Arc<dyn AgentStateReader>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("thread not found: {0}")]
    ThreadNotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self {
            ApiError::AgentNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::ThreadNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(serde_json::json!({ "error": msg }));
        (code, body).into_response()
    }
}

impl From<AgentOsRunError> for ApiError {
    fn from(e: AgentOsRunError) -> Self {
        match e {
            AgentOsRunError::Resolve(
                tirea_agentos::orchestrator::AgentOsResolveError::AgentNotFound(id),
            ) => ApiError::AgentNotFound(id),
            AgentOsRunError::Resolve(other) => ApiError::BadRequest(other.to_string()),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        // Generic thread resources
        .route("/v1/threads", get(list_agent_states))
        .route("/v1/threads/:id", get(get_thread))
        .route("/v1/threads/:id/messages", get(get_thread_messages))
        // Protocol subtrees
        .nest("/v1/ag-ui", ag_ui_router())
        .nest("/v1/ai-sdk", ai_sdk_router())
        .with_state(state)
}

fn ag_ui_router() -> Router<AppState> {
    Router::new()
        .route("/agents/:agent_id/runs", post(run_ag_ui_sse))
        .route("/threads/:id/messages", get(get_thread_messages_agui))
}

fn ai_sdk_router() -> Router<AppState> {
    Router::new()
        .route("/agents/:agent_id/runs", post(run_ai_sdk_sse))
        .route("/threads/:id/messages", get(get_thread_messages_ai_sdk))
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
}

fn default_thread_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
struct ThreadListParams {
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default = "default_thread_limit")]
    limit: usize,
    #[serde(default)]
    parent_thread_id: Option<String>,
}

async fn list_agent_states(
    State(st): State<AppState>,
    Query(params): Query<ThreadListParams>,
) -> Result<Json<AgentStateListPage>, ApiError> {
    let query = AgentStateListQuery {
        offset: params.offset.unwrap_or(0),
        limit: params.limit.clamp(1, 200),
        resource_id: None,
        parent_thread_id: params.parent_thread_id,
    };
    st.read_store
        .list_paginated(&query)
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn get_thread(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Thread>, ApiError> {
    let Some(thread) = st
        .read_store
        .load_agent_state(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Err(ApiError::ThreadNotFound(id));
    };
    Ok(Json(thread))
}

fn default_message_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
struct MessageQueryParams {
    #[serde(default)]
    after: Option<i64>,
    #[serde(default)]
    before: Option<i64>,
    #[serde(default = "default_message_limit")]
    limit: usize,
    #[serde(default)]
    order: Option<String>,
    /// Filter by visibility: "all" (default, user-visible only), "internal", or omit for no filter.
    #[serde(default)]
    visibility: Option<String>,
    /// Filter by run ID.
    #[serde(default)]
    run_id: Option<String>,
}

async fn get_thread_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<Json<MessagePage>, ApiError> {
    let query = parse_message_query(&params);
    st.read_store
        .load_messages(&id, &query)
        .await
        .map(Json)
        .map_err(|e| match e {
            tirea_agentos::contracts::storage::AgentStateStoreError::NotFound(_) => {
                ApiError::ThreadNotFound(id)
            }
            other => ApiError::Internal(other.to_string()),
        })
}

/// Response type for protocol-encoded message pages.
#[derive(Debug, Serialize)]
struct EncodedMessagePage<M: Serialize> {
    messages: Vec<M>,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_cursor: Option<i64>,
}

fn parse_message_query(params: &MessageQueryParams) -> MessageQuery {
    let limit = params.limit.clamp(1, 200);
    let order = match params.order.as_deref() {
        Some("desc") => SortOrder::Desc,
        _ => SortOrder::Asc,
    };
    let visibility = match params.visibility.as_deref() {
        Some("internal") => Some(Visibility::Internal),
        Some("none") => None,
        _ => Some(Visibility::All),
    };
    MessageQuery {
        after: params.after,
        before: params.before,
        limit,
        order,
        visibility,
        run_id: params.run_id.clone(),
    }
}

async fn load_message_page(
    read_store: &Arc<dyn AgentStateReader>,
    thread_id: &str,
    params: &MessageQueryParams,
) -> Result<MessagePage, ApiError> {
    let query = parse_message_query(params);
    read_store
        .load_messages(thread_id, &query)
        .await
        .map_err(|e| match e {
            tirea_agentos::contracts::storage::AgentStateStoreError::NotFound(_) => {
                ApiError::ThreadNotFound(thread_id.to_string())
            }
            other => ApiError::Internal(other.to_string()),
        })
}

fn encode_message_page<E: ProtocolHistoryEncoder>(
    page: MessagePage,
) -> EncodedMessagePage<E::HistoryMessage> {
    EncodedMessagePage {
        messages: page
            .messages
            .iter()
            .map(|m| E::encode_message(&m.message))
            .collect(),
        has_more: page.has_more,
        next_cursor: page.next_cursor,
        prev_cursor: page.prev_cursor,
    }
}

async fn get_thread_messages_agui(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.read_store, &id, &params).await?;
    let encoded = encode_message_page::<AgUiHistoryEncoder>(page);
    Ok(Json(encoded))
}

async fn get_thread_messages_ai_sdk(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.read_store, &id, &params).await?;
    let encoded = encode_message_page::<AiSdkV6HistoryEncoder>(page);
    Ok(Json(encoded))
}

fn encoded_sse_body<E>(
    run: RunStream,
    encoder: E,
    cancellation_token: RunCancellationToken,
    trailer: Option<&'static str>,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static
where
    E: ProtocolOutputEncoder<InputEvent = AgentEvent> + Send + 'static,
    E::Event: Serialize + Send + 'static,
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
                if closed.load(Ordering::Relaxed) {
                    // Client connection is already gone. Keep draining run events
                    // so loop cancellation/finalization can complete cleanly.
                    return Ok(());
                }

                let json = match serde_json::to_string(&event) {
                    Ok(json) => json,
                    Err(err) => {
                        warn!(error = %err, "failed to serialize SSE protocol event");
                        closed.store(true, Ordering::Relaxed);
                        token.cancel();
                        return Ok(());
                    }
                };
                match tx.send(Bytes::from(format!("data: {json}\n\n"))).await {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        // Receiver dropped (client disconnected / aborted request).
                        // Cancel run cooperatively so loop/runtime are transport-agnostic.
                        closed.store(true, Ordering::Relaxed);
                        token.cancel();
                        Ok(())
                    }
                }
            }
        })
        .await;

        if let Some(trailer) = trailer {
            if tx.send(Bytes::from(trailer)).await.is_err() {
                cancellation_token.cancel();
            }
        }
    });

    async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, Infallible>(chunk);
        }
    }
}

async fn run_ai_sdk_sse(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AiSdkV6RunRequest>,
) -> Result<Response, ApiError> {
    if req.thread_id.trim().is_empty() {
        return Err(ApiError::BadRequest("id cannot be empty".to_string()));
    }
    if !req.has_user_input() && !req.has_interaction_responses() {
        return Err(ApiError::BadRequest(
            "request must include user input or interaction responses".to_string(),
        ));
    }

    let mut resolved = st.os.resolve(&agent_id).map_err(AgentOsRunError::from)?;
    apply_ai_sdk_extensions(&mut resolved, &req);
    let run_request = AiSdkV6InputAdapter::to_run_request(agent_id, req);
    let cancellation_token = RunCancellationToken::new();
    let prepared = st.os.prepare_run(run_request, resolved).await?;
    let run =
        AgentOs::execute_prepared(prepared.with_cancellation_token(cancellation_token.clone()))?;
    let enc = AiSdkV6ProtocolEncoder::new(run.run_id.clone(), Some(run.thread_id.clone()));
    let body_stream = encoded_sse_body(run, enc, cancellation_token, Some("data: [DONE]\n\n"));

    Ok(ai_sdk_sse_response(body_stream))
}

async fn run_ag_ui_sse(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<RunAgentInput>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let mut resolved = st.os.resolve(&agent_id).map_err(AgentOsRunError::from)?;
    apply_agui_extensions(&mut resolved, &req);
    let run_request = AgUiInputAdapter::to_run_request(agent_id, req);
    let cancellation_token = RunCancellationToken::new();
    let prepared = st.os.prepare_run(run_request, resolved).await?;
    let run =
        AgentOs::execute_prepared(prepared.with_cancellation_token(cancellation_token.clone()))?;
    let enc = AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone());
    let body_stream = encoded_sse_body(run, enc, cancellation_token, None);

    Ok(sse_response(body_stream))
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

fn ai_sdk_sse_response<S>(stream: S) -> Response
where
    S: futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static,
{
    let mut response = sse_response(stream);
    response.headers_mut().insert(
        header::HeaderName::from_static("x-vercel-ai-ui-message-stream"),
        HeaderValue::from_static("v1"),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-tirea-ai-sdk-version"),
        HeaderValue::from_static(AI_SDK_VERSION),
    );
    response
}
