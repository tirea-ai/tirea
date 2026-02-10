use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use carve_agent::ui_stream::UIStreamEvent;
use carve_agent::{
    apply_agui_request_to_session, AgentOs, Message, RunAgentRequest, RunContext, Session, Storage,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::sync::Arc;
use tracing;
use uuid::Uuid;

use crate::protocol::{AgUiEncoder, AiSdkEncoder};

#[derive(Clone)]
pub struct AppState {
    pub os: Arc<AgentOs>,
    pub storage: Arc<dyn Storage>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, msg) = match &self {
            ApiError::AgentNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::SessionNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(serde_json::json!({ "error": msg }));
        (code, body).into_response()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/sessions", get(list_sessions))
        .route("/v1/sessions/:id", get(get_session))
        .route("/v1/sessions/:id/messages", get(get_session_messages))
        .route("/v1/agents/:agent_id/runs/ai-sdk/sse", post(run_ai_sdk_sse))
        .route("/v1/agents/:agent_id/runs/ag-ui/sse", post(run_ag_ui_sse))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
}

async fn list_sessions(State(st): State<AppState>) -> Result<Json<Vec<String>>, ApiError> {
    st.storage
        .list()
        .await
        .map(Json)
        .map_err(|e| ApiError::Internal(e.to_string()))
}

async fn get_session(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Session>, ApiError> {
    let Some(session) = st
        .storage
        .load(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Err(ApiError::SessionNotFound(id));
    };
    Ok(Json(session))
}

async fn get_session_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Message>>, ApiError> {
    let Some(session) = st
        .storage
        .load(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Err(ApiError::SessionNotFound(id));
    };
    Ok(Json(session.messages.clone()))
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSdkRunRequest {
    #[serde(rename = "sessionId")]
    pub session_id: String,
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
    if req.session_id.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "sessionId cannot be empty".to_string(),
        ));
    }
    if req.input.trim().is_empty() {
        return Err(ApiError::BadRequest("input cannot be empty".to_string()));
    }

    // Validate agent exists before mutating session state.
    st.os.validate_agent(&agent_id).map_err(|e| match e {
        carve_agent::AgentOsResolveError::AgentNotFound(id) => ApiError::AgentNotFound(id),
        other => ApiError::BadRequest(other.to_string()),
    })?;

    let session_id = req.session_id.clone();
    let input = req.input.clone();

    // Prepare session.
    let mut session = st
        .storage
        .load(&session_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| Session::new(session_id.clone()));
    session = session.with_message(Message::user(input));

    // Industry-common: persist the user message immediately so it isn't lost if the run crashes.
    st.storage
        .save(&session)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let run_id = req
        .run_id
        .unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let run_ctx = RunContext {
        run_id: Some(run_id.clone()),
        parent_run_id: None,
    };

    let stream_with_checkpoints = st
        .os
        .run_stream_with_checkpoints(&agent_id, session, run_ctx)
        .map_err(|e| match e {
            carve_agent::AgentOsResolveError::AgentNotFound(id) => ApiError::AgentNotFound(id),
            other => ApiError::BadRequest(other.to_string()),
        })?;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);
    let storage = st.storage.clone();

    tokio::spawn(async move {
        let mut events = stream_with_checkpoints.events;
        let mut output_closed = false;
        let mut enc = AiSdkEncoder::new(run_id.clone());

        // Stream prologue required by AI SDK UI stream protocol.
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
                    thread_id: session_id.clone(),
                    run_id: run_id.clone(),
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

        // AI SDK v6 expects a [DONE] sentinel to cleanly terminate the stream.
        if !output_closed {
            let _ = tx.send(Bytes::from("data: [DONE]\n\n")).await;
        }
    });

    // Persist intermediate checkpoints and the final session.
    {
        let mut checkpoints = stream_with_checkpoints.checkpoints;
        let final_session = stream_with_checkpoints.final_session;
        let storage = storage.clone();
        tokio::spawn(async move {
            while let Some(cp) = checkpoints.recv().await {
                if let Err(e) = storage.save(&cp.session).await {
                    tracing::error!(session_id = %cp.session.id, error = %e, "failed to save checkpoint");
                }
            }
            if let Ok(final_session) = final_session.await {
                if let Err(e) = storage.save(&final_session).await {
                    tracing::error!(session_id = %final_session.id, error = %e, "failed to save final session");
                }
            }
        });
    }

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

    // Validate agent exists before mutating session state.
    st.os.validate_agent(&agent_id).map_err(|e| match e {
        carve_agent::AgentOsResolveError::AgentNotFound(id) => ApiError::AgentNotFound(id),
        other => ApiError::BadRequest(other.to_string()),
    })?;

    let base = st
        .storage
        .load(&req.thread_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_else(|| {
            if let Some(state) = req.state.clone() {
                Session::with_initial_state(req.thread_id.clone(), state)
            } else {
                Session::new(req.thread_id.clone())
            }
        });
    if base.id != req.thread_id {
        return Err(ApiError::BadRequest(
            "stored session id does not match threadId".to_string(),
        ));
    }

    // Industry-common: apply inbound messages (user/tool responses) and persist before running.
    let before_messages = base.messages.len();
    let before_patches = base.patches.len();
    let before_state = base.state.clone();
    let session = apply_agui_request_to_session(base, &req);
    if session.messages.len() != before_messages
        || session.patches.len() != before_patches
        || session.state != before_state
    {
        st.storage
            .save(&session)
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
    }

    let (client, cfg, tools, session) = st.os.resolve(&agent_id, session).map_err(|e| match e {
        carve_agent::AgentOsResolveError::AgentNotFound(id) => ApiError::AgentNotFound(id),
        other => ApiError::BadRequest(other.to_string()),
    })?;

    let stream_with_checkpoints = carve_agent::run_agent_events_with_request_checkpoints(
        client,
        cfg,
        session,
        tools,
        req.clone(),
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Bytes>(64);
    let storage = st.storage.clone();

    tokio::spawn(async move {
        let mut inner = stream_with_checkpoints.events;
        let mut output_closed = false;
        let mut enc = AgUiEncoder::new(req.thread_id.clone(), req.run_id.clone());

        while let Some(ev) = inner.next().await {
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
            if let Some(fallback) = enc.fallback_finished(&req.thread_id, &req.run_id) {
                if let Ok(json) = serde_json::to_string(&fallback) {
                    let _ = tx.send(Bytes::from(format!("data: {}\n\n", json))).await;
                }
            }
        }
    });

    // Persist intermediate checkpoints and the final session.
    {
        let mut checkpoints = stream_with_checkpoints.checkpoints;
        let final_session = stream_with_checkpoints.final_session;
        let storage = storage.clone();
        tokio::spawn(async move {
            while let Some(cp) = checkpoints.recv().await {
                if let Err(e) = storage.save(&cp.session).await {
                    tracing::error!(session_id = %cp.session.id, error = %e, "failed to save checkpoint");
                }
            }
            if let Ok(final_session) = final_session.await {
                if let Err(e) = storage.save(&final_session).await {
                    tracing::error!(session_id = %final_session.id, error = %e, "failed to save final session");
                }
            }
        });
    }

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
