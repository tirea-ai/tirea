use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use std::convert::Infallible;
use std::sync::Arc;
use tirea_protocol_ai_sdk_v6::{
    AiSdkEncoder, AiSdkTrigger, AiSdkV6HistoryEncoder, AiSdkV6RunRequest, UIStreamEvent,
    AI_SDK_VERSION,
};

use super::runtime::apply_ai_sdk_extensions;
use crate::protocol::http_dialog::{
    handle_dialog_run, DialogRelayPlan, DialogRunAdapter, ErrorFormatter, RelayDoneCallback,
};
use tokio::sync::broadcast;

use crate::service::{
    current_run_id_for_thread, encode_message_page, load_message_page, truncate_thread_at_message,
    ApiError, AppState, MessageQueryParams, PreparedHttpRun,
};
use crate::transport::http_sse::sse_response;

const RUN_PATH: &str = "/agents/:agent_id/runs";
const RESUME_STREAM_PATH: &str = "/agents/:agent_id/chats/:chat_id/stream";
/// Legacy path kept for backward-compatibility with AI SDK clients that reconnect
/// via `/runs/:chat_id/stream` after a network drop.
const LEGACY_RESUME_STREAM_PATH: &str = "/agents/:agent_id/runs/:chat_id/stream";
const THREAD_MESSAGES_PATH: &str = "/threads/:id/messages";

/// Build AI SDK v6 HTTP routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(RUN_PATH, post(run))
        .route(RESUME_STREAM_PATH, get(resume_stream))
        .route(LEGACY_RESUME_STREAM_PATH, get(resume_stream))
        .route(THREAD_MESSAGES_PATH, get(thread_messages))
}

async fn thread_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<MessageQueryParams>,
) -> Result<impl IntoResponse, ApiError> {
    let page = load_message_page(&st.read_store, &id, &params).await?;
    let encoded = encode_message_page(page, AiSdkV6HistoryEncoder::encode_message);
    Ok(Json(encoded))
}

async fn run(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<AiSdkV6RunRequest>,
) -> Result<Response, ApiError> {
    req.validate().map_err(ApiError::BadRequest)?;
    if req.trigger == Some(AiSdkTrigger::RegenerateMessage) {
        truncate_thread_at_message(&st.os, &req.thread_id, req.message_id.as_deref().unwrap())
            .await?;
    }

    handle_dialog_run::<AiSdkDialogAdapter>(st, agent_id, req).await
}

async fn resume_stream(
    State(st): State<AppState>,
    Path((agent_id, chat_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let Some(run_id) =
        current_run_id_for_thread(&st.os, &agent_id, &chat_id, st.read_store.as_ref()).await?
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let Some(mut receiver) = st.os.subscribe_thread_run_stream(&run_id).await else {
        return Ok(StatusCode::NO_CONTENT.into_response());
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
    Ok(ai_sdk_sse_response(stream))
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

struct AiSdkDialogAdapter;

#[async_trait]
impl DialogRunAdapter for AiSdkDialogAdapter {
    type Request = AiSdkV6RunRequest;
    type Encoder = AiSdkEncoder;

    fn validate(req: &Self::Request) -> Result<(), ApiError> {
        req.validate().map_err(ApiError::BadRequest)
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

    fn apply_extensions(resolved: &mut tirea_agentos::runtime::ResolvedRun, req: &Self::Request) {
        apply_ai_sdk_extensions(resolved, req);
    }

    fn into_run_request(
        req: Self::Request,
        agent_id: String,
    ) -> tirea_agentos::contracts::RunRequest {
        req.into_runtime_run_request(agent_id)
    }

    async fn build_relay_plan(
        st: &AppState,
        prepared: &PreparedHttpRun,
        _req: &Self::Request,
    ) -> Result<DialogRelayPlan<Self::Encoder>, ApiError> {
        let run_id = prepared.run_id.clone();
        let (fanout, _) = broadcast::channel::<Bytes>(128);
        if !st
            .os
            .bind_thread_run_stream_fanout(&run_id, fanout.clone())
            .await
        {
            return Err(ApiError::Internal(format!(
                "active run handle missing for run '{run_id}'"
            )));
        }
        let run_id_for_cleanup = run_id;
        let os_for_cleanup = st.os.clone();
        let fanout_for_done = fanout.clone();

        let on_relay_done: RelayDoneCallback = Box::new(move |sse_tx| {
            Box::pin(async move {
                let trailer = Bytes::from("data: [DONE]\n\n");
                let _ = fanout_for_done.send(trailer.clone());
                if sse_tx.send(trailer).await.is_err() {
                    let _ = os_for_cleanup
                        .cancel_active_run_by_id(&run_id_for_cleanup)
                        .await;
                }
            })
        });
        let error_formatter: ErrorFormatter = Arc::new(|msg| {
            let json = serde_json::to_string(&UIStreamEvent::error(&msg)).unwrap_or_default();
            Bytes::from(format!("data: {json}\n\n"))
        });

        Ok(DialogRelayPlan {
            encoder: AiSdkEncoder::new(),
            fanout: Some(fanout),
            resumable_downstream: true,
            protocol_label: "ai-sdk",
            on_relay_done,
            error_formatter,
        })
    }

    fn into_response<S>(stream: S) -> Response
    where
        S: futures::Stream<Item = Result<Bytes, std::convert::Infallible>> + Send + 'static,
    {
        ai_sdk_sse_response(stream)
    }

    fn forwarded_decision_response(
        forwarded: tirea_agentos::runtime::ForwardedDecision,
        _frontend_run_id: Option<&str>,
    ) -> Response {
        (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "status": "decision_forwarded",
                "threadId": forwarded.thread_id,
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::http_run::{wire_http_sse_relay, HttpSseRelayConfig};
    use crate::transport::runtime_endpoint::RunStarter;
    use std::pin::Pin;
    use tirea_agentos::contracts::{AgentEvent, RunRequest, ToolCallDecision};
    use tirea_agentos::runtime::RunStream;
    use tirea_contract::RunOrigin;
    use tirea_contract::RuntimeInput;
    use tokio::sync::mpsc;

    fn test_run_request() -> RunRequest {
        RunRequest {
            agent_id: "test".into(),
            thread_id: None,
            run_id: None,
            parent_run_id: None,
            parent_thread_id: None,
            resource_id: None,
            origin: RunOrigin::default(),
            state: None,
            messages: vec![],
            initial_decisions: vec![],
            source_mailbox_entry_id: None,
        }
    }

    fn fake_run(events: Vec<AgentEvent>) -> RunStream {
        let (decision_tx, _decision_rx) = mpsc::unbounded_channel::<ToolCallDecision>();
        let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(16);

        tokio::spawn(async move {
            for event in events {
                let _ = event_tx.send(event).await;
            }
        });

        let stream: Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>> =
            Box::pin(async_stream::stream! {
                let mut rx = event_rx;
                while let Some(item) = rx.recv().await {
                    yield item;
                }
            });

        RunStream {
            thread_id: "thread-ai-sdk".to_string(),
            run_id: "run-ai-sdk".to_string(),
            decision_tx,
            events: stream,
        }
    }

    fn ai_sdk_error_chunk(msg: &str) -> Bytes {
        let json =
            serde_json::to_string(&UIStreamEvent::error(msg)).expect("serialize ai-sdk error");
        Bytes::from(format!("data: {json}\n\n"))
    }

    #[test]
    fn ai_sdk_error_chunk_matches_ui_message_stream_schema() {
        let chunk = ai_sdk_error_chunk("Web stream error for model 'openai::gemini-2.5-flash '");
        let text = String::from_utf8(chunk.to_vec()).expect("utf-8 sse");
        let payload = text.trim().strip_prefix("data: ").expect("sse payload");
        let event: UIStreamEvent = serde_json::from_str(payload).expect("valid ai-sdk event");

        match event {
            UIStreamEvent::Error { error_text } => {
                assert!(error_text.contains("Web stream error for model"));
                assert!(!payload.contains("recoverable"));
                assert!(!payload.contains("\"message\""));
            }
            other => panic!("expected ai-sdk error event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn runtime_error_event_streams_as_valid_ai_sdk_error_chunk() {
        let starter: RunStarter = Box::new(move |_request| {
            Box::pin(async move {
                Ok(fake_run(vec![AgentEvent::Error {
                    message: "provider stream failed".to_string(),
                    code: Some("PROVIDER_ERROR".to_string()),
                }]))
            })
        });
        let (ingress_tx, ingress_rx) = mpsc::unbounded_channel::<RuntimeInput>();
        ingress_tx
            .send(RuntimeInput::Run(test_run_request()))
            .expect("send run request");
        drop(ingress_tx);

        let mut sse_rx = wire_http_sse_relay(
            starter,
            AiSdkEncoder::new(),
            ingress_rx,
            HttpSseRelayConfig {
                thread_id: "thread-ai-sdk".to_string(),
                fanout: None,
                resumable_downstream: true,
                protocol_label: "ai-sdk",
                on_relay_done: |_sse_tx| async move {},
                error_formatter: |msg: String| ai_sdk_error_chunk(&msg),
            },
        );

        let chunks: Vec<Bytes> = async {
            let mut out = Vec::new();
            while let Some(chunk) = sse_rx.recv().await {
                out.push(chunk);
            }
            out
        }
        .await;

        let payloads: Vec<&str> = chunks
            .iter()
            .filter_map(|chunk| std::str::from_utf8(chunk).ok())
            .filter_map(|text| text.trim().strip_prefix("data: "))
            .collect();

        assert_eq!(
            payloads.len(),
            1,
            "unexpected ai-sdk payloads: {payloads:?}"
        );
        let event: UIStreamEvent =
            serde_json::from_str(payloads[0]).expect("valid ai-sdk runtime error event");
        assert!(matches!(
            event,
            UIStreamEvent::Error { ref error_text } if error_text == "provider stream failed"
        ));
    }

    #[tokio::test]
    async fn ai_sdk_sse_response_sets_protocol_headers() {
        let response = ai_sdk_sse_response(futures::stream::empty::<Result<Bytes, Infallible>>());
        let headers = response.headers();

        assert_eq!(
            headers.get("content-type").and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        assert_eq!(
            headers
                .get("x-vercel-ai-ui-message-stream")
                .and_then(|v| v.to_str().ok()),
            Some("v1")
        );
        assert_eq!(
            headers
                .get("x-tirea-ai-sdk-version")
                .and_then(|v| v.to_str().ok()),
            Some(AI_SDK_VERSION)
        );
    }
}
