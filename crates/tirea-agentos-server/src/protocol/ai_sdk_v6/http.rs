use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tirea_agentos::contracts::thread::Thread;
use tirea_agentos::contracts::AgentEvent;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError, RunStream};
use tirea_agentos::runtime::loop_runner::RunCancellationToken;
use tirea_contract::{ProtocolInputAdapter, ProtocolOutputEncoder};
use tirea_protocol_ai_sdk_v6::{
    apply_ai_sdk_extensions, AiSdkTrigger, AiSdkV6HistoryEncoder, AiSdkV6InputAdapter,
    AiSdkV6ProtocolEncoder, AiSdkV6RunRequest, AI_SDK_VERSION,
};
use tokio::sync::{broadcast, RwLock};
use tracing::warn;

use crate::service::{
    active_run_key, encode_message_page, load_message_page, register_active_run, remove_active_run,
    require_agent_state_store, try_forward_decisions_to_active_run, ApiError, AppState,
    MessageQueryParams,
};
use crate::transport::pump_encoded_stream;

/// AI SDK v6 run endpoint path (to be nested under protocol root).
pub const RUN_PATH: &str = "/agents/:agent_id/runs";
/// AI SDK v6 stream resume path (to be nested under protocol root).
pub const RESUME_STREAM_PATH: &str = "/agents/:agent_id/runs/:chat_id/stream";
/// AI SDK v6 history endpoint path (to be nested under protocol root).
pub const THREAD_MESSAGES_PATH: &str = "/threads/:id/messages";

/// Build AI SDK v6 HTTP routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(RUN_PATH, post(run))
        .route(RESUME_STREAM_PATH, get(resume_stream))
        .route(THREAD_MESSAGES_PATH, get(thread_messages))
}

#[derive(Default)]
struct StreamRegistry {
    streams: RwLock<HashMap<String, broadcast::Sender<Bytes>>>,
}

impl StreamRegistry {
    async fn register(&self, key: String) -> broadcast::Sender<Bytes> {
        let (tx, _) = broadcast::channel(128);
        self.streams.write().await.insert(key, tx.clone());
        tx
    }

    async fn subscribe(&self, key: &str) -> Option<broadcast::Receiver<Bytes>> {
        self.streams
            .read()
            .await
            .get(key)
            .map(|sender| sender.subscribe())
    }

    async fn remove(&self, key: &str) {
        self.streams.write().await.remove(key);
    }
}

static STREAM_REGISTRY: OnceLock<StreamRegistry> = OnceLock::new();

fn stream_registry() -> &'static StreamRegistry {
    STREAM_REGISTRY.get_or_init(StreamRegistry::default)
}

fn stream_key(agent_id: &str, chat_id: &str) -> String {
    format!("{agent_id}:{chat_id}")
}

async fn thread_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.read_store, &id, &params).await?;
    let encoded = encode_message_page::<AiSdkV6HistoryEncoder>(page);
    Ok(Json(encoded))
}

async fn run(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AiSdkV6RunRequest>,
) -> Result<Response, ApiError> {
    if req.thread_id.trim().is_empty() {
        return Err(ApiError::BadRequest("id cannot be empty".to_string()));
    }
    if req.trigger == Some(AiSdkTrigger::RegenerateMessage) {
        let message_id = req.message_id.as_deref().ok_or_else(|| {
            ApiError::BadRequest("messageId is required for regenerate-message".to_string())
        })?;
        if message_id.trim().is_empty() {
            return Err(ApiError::BadRequest(
                "messageId cannot be empty for regenerate-message".to_string(),
            ));
        }
        if !req.contains_message_id(message_id) {
            return Err(ApiError::BadRequest(
                "messageId must reference a message in messages for regenerate-message".to_string(),
            ));
        }
    }
    if !req.has_user_input() && !req.has_suspension_decisions() {
        return Err(ApiError::BadRequest(
            "request must include user input or suspension decisions".to_string(),
        ));
    }

    let suspension_decisions = req.suspension_decisions();
    let decision_only = !req.has_user_input() && !suspension_decisions.is_empty();
    if decision_only {
        let key = active_run_key("ai_sdk", &agent_id, &req.thread_id);
        if try_forward_decisions_to_active_run(&key, suspension_decisions).await {
            return Ok((
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "status": "decision_forwarded",
                    "threadId": req.thread_id,
                })),
            )
                .into_response());
        }
    }

    let mut resolved = st.os.resolve(&agent_id).map_err(AgentOsRunError::from)?;
    apply_ai_sdk_extensions(&mut resolved, &req);

    sync_thread_snapshot(&st, &req).await?;

    let mut run_request = AiSdkV6InputAdapter::to_run_request(agent_id.clone(), req);
    run_request.messages.clear();
    let cancellation_token = RunCancellationToken::new();
    let prepared = st.os.prepare_run(run_request, resolved).await?;
    let run =
        AgentOs::execute_prepared(prepared.with_cancellation_token(cancellation_token.clone()))?;

    let stream_key = stream_key(&agent_id, &run.thread_id);
    let active_key = active_run_key("ai_sdk", &agent_id, &run.thread_id);
    register_active_run(active_key.clone(), run.decision_tx.clone()).await;

    let fanout = stream_registry().register(stream_key.clone()).await;
    let encoder = AiSdkV6ProtocolEncoder::new(run.run_id.clone(), Some(run.thread_id.clone()));
    let body_stream = encoded_sse_body(
        run,
        encoder,
        cancellation_token,
        fanout,
        stream_key,
        active_key,
    );

    Ok(ai_sdk_sse_response(body_stream))
}

async fn resume_stream(Path((agent_id, chat_id)): Path<(String, String)>) -> Response {
    let key = stream_key(&agent_id, &chat_id);
    let Some(mut receiver) = stream_registry().subscribe(&key).await else {
        return StatusCode::NO_CONTENT.into_response();
    };

    let stream = async_stream::stream! {
        loop {
            match receiver.recv().await {
                Ok(chunk) => yield Ok::<Bytes, Infallible>(chunk),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    ai_sdk_sse_response(stream)
}

async fn sync_thread_snapshot(st: &AppState, req: &AiSdkV6RunRequest) -> Result<(), ApiError> {
    let store = require_agent_state_store(&st.os)?;

    let mut thread = match store
        .load(&req.thread_id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?
    {
        Some(head) => head.thread,
        None => Thread::new(req.thread_id.clone()),
    };

    thread.messages = req
        .to_thread_message_snapshot()
        .into_iter()
        .map(Arc::new)
        .collect();
    upsert_protocol_state(&mut thread.state, req.to_protocol_snapshot());

    store
        .save(&thread)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))
}

fn upsert_protocol_state(state: &mut Value, snapshot: Value) {
    if !state.is_object() {
        *state = Value::Object(serde_json::Map::new());
    }
    let Some(root) = state.as_object_mut() else {
        return;
    };

    let protocol_entry = root
        .entry("protocol".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !protocol_entry.is_object() {
        *protocol_entry = Value::Object(serde_json::Map::new());
    }
    let Some(protocol_map) = protocol_entry.as_object_mut() else {
        return;
    };
    protocol_map.insert("ai_sdk".to_string(), snapshot);
}

fn encoded_sse_body<E>(
    run: RunStream,
    encoder: E,
    cancellation_token: RunCancellationToken,
    fanout: broadcast::Sender<Bytes>,
    stream_key: String,
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
        let fanout_events = fanout.clone();

        pump_encoded_stream(run.events, encoder, move |event| {
            let tx = tx_events.clone();
            let token = event_cancel_token.clone();
            let closed = send_state.clone();
            let fanout = fanout_events.clone();
            async move {
                let json = match serde_json::to_string(&event) {
                    Ok(json) => json,
                    Err(err) => {
                        warn!(error = %err, "failed to serialize SSE protocol event");
                        closed.store(true, Ordering::Relaxed);
                        return Ok(());
                    }
                };
                let chunk = Bytes::from(format!("data: {json}\n\n"));
                let _ = fanout.send(chunk.clone());

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

        let trailer = Bytes::from("data: [DONE]\n\n");
        let _ = fanout.send(trailer.clone());
        if tx.send(trailer).await.is_err() {
            cancellation_token.cancel();
        }

        stream_registry().remove(&stream_key).await;
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
