use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use carve_agent::ag_ui::{AGUIMessage, MessageRole, RunAgentRequest};
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::{
    AgentOs, AgentOsRunError, Message, MessagePage, MessageQuery, Role, RunRequest, SortOrder,
    Thread, ThreadListPage, ThreadListQuery, ThreadQuery, Visibility,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use crate::protocol::{AgUiEncoder, AiSdkEncoder};

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

/// Convert AG-UI messages to internal format, filtering out assistant messages
/// (which are echoed back by AG-UI clients and would cause duplicates).
fn convert_agui_messages(messages: &[AGUIMessage]) -> Vec<Message> {
    messages
        .iter()
        .filter(|m| m.role != MessageRole::Assistant)
        .map(|m| {
            let role = match m.role {
                MessageRole::System | MessageRole::Developer => Role::System,
                MessageRole::User => Role::User,
                MessageRole::Assistant => Role::Assistant,
                MessageRole::Tool => Role::Tool,
            };
            Message {
                id: m.id.clone(),
                role,
                content: m.content.clone(),
                tool_calls: None,
                tool_call_id: m.tool_call_id.clone(),
                visibility: Visibility::default(),
                metadata: None,
            }
        })
        .collect()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/threads", get(list_threads))
        .route("/v1/threads/:id", get(get_thread))
        .route("/v1/threads/:id/messages", get(get_thread_messages))
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
    let limit = params.limit.clamp(1, 200);
    let order = match params.order.as_deref() {
        Some("desc") => SortOrder::Desc,
        _ => SortOrder::Asc,
    };
    let visibility = match params.visibility.as_deref() {
        Some("internal") => Some(Visibility::Internal),
        Some("none") => None,
        // Default: only user-visible messages
        _ => Some(Visibility::All),
    };
    let query = MessageQuery {
        after: params.after,
        before: params.before,
        limit,
        order,
        visibility,
        run_id: params.run_id,
    };
    st.storage
        .load_messages(&id, &query)
        .await
        .map(Json)
        .map_err(|e| match e {
            carve_agent::StorageError::NotFound(_) => ApiError::ThreadNotFound(id),
            other => ApiError::Internal(other.to_string()),
        })
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSdkRunRequest {
    #[serde(rename = "sessionId")]
    pub thread_id: String,
    pub input: String,
    #[serde(rename = "runId")]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RunInfo {
    #[serde(rename = "threadId")]
    thread_id: String,
    #[serde(rename = "runId")]
    run_id: String,
}

async fn run_ai_sdk_sse(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AiSdkRunRequest>,
) -> Result<Response, ApiError> {
    if req.input.trim().is_empty() {
        return Err(ApiError::BadRequest("input cannot be empty".to_string()));
    }
    if req.thread_id.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }

    let run_request = RunRequest {
        agent_id,
        thread_id: if req.thread_id.trim().is_empty() {
            None
        } else {
            Some(req.thread_id)
        },
        run_id: req.run_id,
        resource_id: None,
        initial_state: None,
        messages: vec![Message::user(req.input)],
        runtime: HashMap::new(),
    };

    let run = st.os.run_stream(run_request).await?;

    let thread_id = run.thread_id.clone();
    let run_id = run.run_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let mut events = run.events;
        let mut output_closed = false;
        let mut enc = AiSdkEncoder::new(run_id.clone());

        // AI SDK v6 protocol framing: message-start.
        for e in enc.prologue() {
            if output_closed {
                break;
            }
            if let Ok(json) = serde_json::to_string(&e) {
                if tx
                    .send(Bytes::from(format!("data: {}\n\n", json)))
                    .await
                    .is_err()
                {
                    output_closed = true;
                }
            }
        }
        if !output_closed {
            let run_info = UIStreamEvent::data(
                "run-info",
                serde_json::to_value(RunInfo {
                    thread_id,
                    run_id,
                })
                .unwrap_or_default(),
            );
            if let Ok(json) = serde_json::to_string(&run_info) {
                if tx
                    .send(Bytes::from(format!("data: {}\n\n", json)))
                    .await
                    .is_err()
                {
                    output_closed = true;
                }
            }
        }

        while let Some(ev) = events.next().await {
            if output_closed {
                break;
            }
            for ui in enc.on_agent_event(&ev) {
                if let Ok(json) = serde_json::to_string(&ui) {
                    if tx
                        .send(Bytes::from(format!("data: {}\n\n", json)))
                        .await
                        .is_err()
                    {
                        output_closed = true;
                        break;
                    }
                }
            }
        }

        if !output_closed {
            let _ = tx.send(Bytes::from("data: [DONE]\n\n")).await;
        }
    });

    let body_stream = async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, Infallible>(chunk);
        }
    };

    Ok(sse_response(body_stream))
}

async fn run_ag_ui_sse(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<RunAgentRequest>,
) -> Result<Response, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Convert AG-UI protocol → internal RunRequest
    let mut runtime = HashMap::new();
    if let Some(ref parent_run_id) = req.parent_run_id {
        runtime.insert(
            "parent_run_id".to_string(),
            serde_json::Value::String(parent_run_id.clone()),
        );
    }

    let run_request = RunRequest {
        agent_id,
        thread_id: Some(req.thread_id.clone()),
        run_id: Some(req.run_id.clone()),
        resource_id: None,
        initial_state: req.state.clone(),
        messages: convert_agui_messages(&req.messages),
        runtime,
    };

    let run = st.os.run_stream(run_request).await?;

    let thread_id = run.thread_id.clone();
    let run_id = run.run_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);

    tokio::spawn(async move {
        let mut events = run.events;
        let mut output_closed = false;
        let mut enc = AgUiEncoder::new(thread_id.clone(), run_id.clone());

        while let Some(ev) = events.next().await {
            if output_closed {
                break;
            }
            for ag in enc.on_agent_event(&ev) {
                if let Ok(json) = serde_json::to_string(&ag) {
                    if tx
                        .send(Bytes::from(format!("data: {}\n\n", json)))
                        .await
                        .is_err()
                    {
                        output_closed = true;
                        break;
                    }
                }
            }
        }

        if !output_closed {
            for fallback in enc.fallback_finished(&thread_id, &run_id) {
                if let Ok(json) = serde_json::to_string(&fallback) {
                    let _ = tx.send(Bytes::from(format!("data: {}\n\n", json))).await;
                }
            }
        }
    });

    let body_stream = async_stream::stream! {
        while let Some(chunk) = rx.recv().await {
            yield Ok::<Bytes, Infallible>(chunk);
        }
    };

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
    headers.insert(
        header::HeaderName::from_static("x-vercel-ai-ui-message-stream"),
        HeaderValue::from_static("v1"),
    );

    (headers, Body::from_stream(stream)).into_response()
}
