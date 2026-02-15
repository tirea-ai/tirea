use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use carve_agent::ag_ui::{
    AgUiHistoryEncoder, AgUiInputAdapter, AgUiProtocolEncoder, RunAgentRequest,
};
use carve_agent::ai_sdk_v6::{
    AiSdkV6HistoryEncoder, AiSdkV6InputAdapter, AiSdkV6ProtocolEncoder, AiSdkV6RunRequest,
    AI_SDK_VERSION,
};
use carve_agent::protocol::{ProtocolHistoryEncoder, ProtocolInputAdapter, ProtocolOutputEncoder};
use carve_agent::{
    AgentOs, AgentOsRunError, MessagePage, MessageQuery, RunStream, SortOrder, Thread,
    ThreadListPage, ThreadListQuery, ThreadQuery, Visibility,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tracing::warn;

use crate::transport::pump_encoded_stream;

#[derive(Clone)]
pub struct AppState {
    pub os: Arc<AgentOs>,
    pub storage: Arc<dyn ThreadQuery>,
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
            AgentOsRunError::Resolve(carve_agent::AgentOsResolveError::AgentNotFound(id)) => {
                ApiError::AgentNotFound(id)
            }
            AgentOsRunError::Resolve(other) => ApiError::BadRequest(other.to_string()),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/threads", get(list_threads))
        .route("/v1/threads/:id", get(get_thread))
        .route("/v1/threads/:id/messages", get(get_thread_messages))
        .route(
            "/v1/threads/:id/messages/ag-ui",
            get(get_thread_messages_agui),
        )
        .route(
            "/v1/threads/:id/messages/ai-sdk",
            get(get_thread_messages_ai_sdk),
        )
        .route("/v1/agents/:agent_id/runs/ai-sdk/sse", post(run_ai_sdk_sse))
        .route("/v1/agents/:agent_id/runs/ag-ui/sse", post(run_ag_ui_sse))
        .with_state(state)
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

async fn list_threads(
    State(st): State<AppState>,
    Query(params): Query<ThreadListParams>,
) -> Result<Json<ThreadListPage>, ApiError> {
    let query = ThreadListQuery {
        offset: params.offset.unwrap_or(0),
        limit: params.limit.clamp(1, 200),
        resource_id: None,
        parent_thread_id: params.parent_thread_id,
    };
    st.storage
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
        .storage
        .load_thread(&id)
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
    st.storage
        .load_messages(&id, &query)
        .await
        .map(Json)
        .map_err(|e| match e {
            carve_agent::StorageError::NotFound(_) => ApiError::ThreadNotFound(id),
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
    storage: &Arc<dyn ThreadQuery>,
    thread_id: &str,
    params: &MessageQueryParams,
) -> Result<MessagePage, ApiError> {
    let query = parse_message_query(params);
    storage
        .load_messages(thread_id, &query)
        .await
        .map_err(|e| match e {
            carve_agent::StorageError::NotFound(_) => {
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
    let page = load_message_page(&st.storage, &id, &params).await?;
    let encoded = encode_message_page::<AgUiHistoryEncoder>(page);
    Ok(Json(encoded))
}

async fn get_thread_messages_ai_sdk(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.storage, &id, &params).await?;
    let encoded = encode_message_page::<AiSdkV6HistoryEncoder>(page);
    Ok(Json(encoded))
}

fn encoded_sse_body<E>(
    run: RunStream,
    encoder: E,
    trailer: Option<&'static str>,
) -> impl futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static
where
    E: ProtocolOutputEncoder + Send + 'static,
    E::Event: Serialize + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let tx_events = tx.clone();
        pump_encoded_stream(run.events, encoder, move |event| {
            let tx = tx_events.clone();
            async move {
                let json = match serde_json::to_string(&event) {
                    Ok(json) => json,
                    Err(err) => {
                        warn!(error = %err, "failed to serialize SSE protocol event");
                        return Err(());
                    }
                };
                tx.send(Bytes::from(format!("data: {json}\n\n")))
                    .await
                    .map_err(|_| ())
            }
        })
        .await;

        if let Some(trailer) = trailer {
            let _ = tx.send(Bytes::from(trailer)).await;
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
    if req.input.trim().is_empty() {
        return Err(ApiError::BadRequest("input cannot be empty".to_string()));
    }
    if req.thread_id.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }

    let run_request = AiSdkV6InputAdapter::to_run_request(agent_id, req);
    let run = st.os.run_stream(run_request).await?;
    let enc = AiSdkV6ProtocolEncoder::new(run.run_id.clone(), Some(run.thread_id.clone()));
    let body_stream = encoded_sse_body(run, enc, Some("data: [DONE]\n\n"));

    Ok(ai_sdk_sse_response(body_stream))
}

async fn run_ag_ui_sse(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<RunAgentRequest>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    let run_request = AgUiInputAdapter::to_run_request(agent_id, req);
    let run = st.os.run_stream(run_request).await?;
    let enc = AgUiProtocolEncoder::new(run.thread_id.clone(), run.run_id.clone());
    let body_stream = encoded_sse_body(run, enc, None);

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
        header::HeaderName::from_static("x-carve-ai-sdk-version"),
        HeaderValue::from_static(AI_SDK_VERSION),
    );
    response
}
