//! /v1/ai-sdk routes.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use awaken_contract::contract::content::ContentBlock;
use awaken_contract::contract::message::Message;

use crate::app::AppState;
use crate::http_run::wire_sse_relay_resumable;
use crate::http_sse::{sse_body_stream, sse_response};
use crate::routes::ApiError;
use crate::transport::replay_buffer::EventReplayBuffer;
use awaken_runtime::RunRequest;

use super::encoder::AiSdkEncoder;

/// Build AI SDK v6 routes.
///
/// The resume route `{api}/{chatId}/stream` matches the AI SDK's
/// `HttpChatTransport.reconnectToStream()` URL pattern, where `chatId`
/// maps to awaken's `threadId`.
pub fn ai_sdk_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/ai-sdk/chat", post(ai_sdk_chat))
        .route(
            "/v1/ai-sdk/threads/:thread_id/runs",
            post(ai_sdk_chat_threaded),
        )
        .route(
            "/v1/ai-sdk/agents/:agent_id/runs",
            post(ai_sdk_chat_agent_scoped),
        )
        // AI SDK reconnect: GET {api}/{chatId}/stream → chatId = thread_id
        .route("/v1/ai-sdk/chat/:thread_id/stream", get(resume_stream))
        .route(
            "/v1/ai-sdk/threads/:thread_id/messages",
            get(thread_messages),
        )
}

#[derive(Debug, Deserialize)]
struct AiSdkChatRequest {
    #[serde(default)]
    messages: Vec<UIMessage>,
    #[serde(rename = "threadId", alias = "thread_id", default)]
    thread_id: Option<String>,
    #[serde(rename = "agentId", alias = "agent_id", default)]
    agent_id: Option<String>,
    #[serde(default)]
    state: Option<Value>,
}

// ── AI SDK v6 UIMessage types ────────────────────────────────────────
//
// Maps directly to the TypeScript `UIMessage` / `UIMessagePart` types
// from the `ai` npm package (v6). No legacy `content` field.
//
// Reference: https://ai-sdk.dev/docs/ai-sdk-ui/chatbot

/// A single part of a `UIMessage`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum UIPart {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    File {
        url: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
    SourceUrl {
        #[serde(rename = "sourceId")]
        source_id: String,
        url: String,
        #[serde(default)]
        title: Option<String>,
    },
    SourceDocument {
        #[serde(rename = "sourceId")]
        source_id: String,
        #[serde(rename = "mediaType")]
        media_type: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        filename: Option<String>,
    },
    StepStart,
    /// Catch-all for tool invocations (`tool-*`), dynamic tools, and data parts (`data-*`).
    /// These don't map to LLM content blocks — they are UI state, not input.
    #[serde(other)]
    Other,
}

/// AI SDK v6 `UIMessage` — the wire format sent by `DefaultChatTransport`.
#[derive(Debug, Deserialize)]
struct UIMessage {
    #[serde(default)]
    id: Option<String>,
    role: String,
    #[serde(default)]
    parts: Vec<UIPart>,
}

/// Parse a data-URI of the form `data:<media_type>;base64,<data>`.
fn parse_data_uri(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(",")?;
    let media_type = meta.strip_suffix(";base64")?;
    Some((media_type.to_string(), data.to_string()))
}

/// Convert a `UIPart` to a `ContentBlock`.
fn part_to_content_block(part: &UIPart) -> Option<ContentBlock> {
    match part {
        UIPart::Text { text } => Some(ContentBlock::text(text.as_str())),
        UIPart::File { url, media_type } => {
            if let Some((mime, data)) = parse_data_uri(url) {
                // Classify by MIME prefix
                if mime.starts_with("image/") {
                    Some(ContentBlock::image_base64(mime, data))
                } else if mime.starts_with("audio/") {
                    Some(ContentBlock::audio_base64(mime, data))
                } else if mime.starts_with("video/") {
                    Some(ContentBlock::video_base64(mime, data))
                } else {
                    Some(ContentBlock::document_base64(mime, data, None))
                }
            } else if media_type.starts_with("image/") {
                Some(ContentBlock::image_url(url.as_str()))
            } else if media_type.starts_with("audio/") {
                Some(ContentBlock::audio_url(url.as_str()))
            } else if media_type.starts_with("video/") {
                Some(ContentBlock::video_url(url.as_str()))
            } else {
                Some(ContentBlock::document_url(url.as_str(), None))
            }
        }
        // Reasoning, SourceUrl, SourceDocument, StepStart, Other — not LLM input
        _ => None,
    }
}

/// Convert `UIMessage` list to awaken `Message` list.
fn convert_messages(msgs: Vec<UIMessage>) -> Vec<Message> {
    msgs.into_iter()
        .filter_map(|m| {
            let blocks: Vec<ContentBlock> =
                m.parts.iter().filter_map(part_to_content_block).collect();
            if blocks.is_empty() {
                return None;
            }
            match m.role.as_str() {
                "user" => Some(Message::user_with_content(blocks)),
                "assistant" => Some(Message::assistant(
                    awaken_contract::contract::content::extract_text(&blocks),
                )),
                "system" => Some(Message::system(
                    awaken_contract::contract::content::extract_text(&blocks),
                )),
                _ => None,
            }
        })
        .collect()
}

/// Reconnect to an active thread's event stream.
///
/// Called by AI SDK's `HttpChatTransport.reconnectToStream()` which issues
/// `GET {api}/{chatId}/stream`. In awaken, `chatId` maps to `thread_id`.
///
/// Replays buffered frames after the client's `Last-Event-ID` and then
/// streams any new frames produced by the still-running agent.
///
/// Returns **204 No Content** if no active stream exists for the thread.
/// AI SDK treats 204 as "nothing to resume" and returns `null` to the caller.
async fn resume_stream(
    State(st): State<AppState>,
    Path(thread_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let last_event_id: u64 = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let buffer = st.replay_buffers.lock().get(&thread_id).cloned();

    let Some(buffer) = buffer else {
        // AI SDK expects 204 for "no active stream" — not 404.
        return axum::http::StatusCode::NO_CONTENT.into_response();
    };

    // Atomically replay + subscribe under a single lock hold.
    // This guarantees no duplicates and no gaps.
    let (replayed, live_rx) = buffer.subscribe_after(last_event_id);

    let replay_stream = futures::stream::iter(replayed.into_iter().map(Ok::<Bytes, Infallible>));
    let live_stream =
        tokio_stream::wrappers::UnboundedReceiverStream::new(live_rx).map(Ok::<Bytes, Infallible>);
    let combined = replay_stream.chain(live_stream);

    sse_response(combined)
}

async fn thread_messages(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<crate::query::MessageQueryParams>,
) -> Result<Json<Value>, ApiError> {
    let messages = st
        .store
        .load_messages(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .unwrap_or_default();

    let offset = params.offset_or_default();
    let limit = params.clamped_limit();
    let total = messages.len();

    let encoded: Vec<Value> = messages
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            serde_json::json!({
                "id": m.id,
                "role": match m.role {
                    awaken_contract::contract::message::Role::System => "system",
                    awaken_contract::contract::message::Role::User => "user",
                    awaken_contract::contract::message::Role::Assistant => "assistant",
                    awaken_contract::contract::message::Role::Tool => "tool",
                },
                "content": [{
                    "type": "text",
                    "text": m.content
                }],
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "messages": encoded,
        "total": total,
        "has_more": offset + encoded.len() < total,
    })))
}

async fn ai_sdk_chat(
    State(st): State<AppState>,
    Json(payload): Json<AiSdkChatRequest>,
) -> Result<Response, ApiError> {
    ai_sdk_chat_inner(st, payload).await
}

/// Thread-centric route: `POST /v1/ai-sdk/threads/:thread_id/runs`
async fn ai_sdk_chat_threaded(
    State(st): State<AppState>,
    Path(thread_id): Path<String>,
    Json(mut payload): Json<AiSdkChatRequest>,
) -> Result<Response, ApiError> {
    payload.thread_id = Some(thread_id);
    ai_sdk_chat_inner(st, payload).await
}

/// Agent-scoped route: `POST /v1/ai-sdk/agents/:agent_id/runs`
async fn ai_sdk_chat_agent_scoped(
    State(st): State<AppState>,
    Path(agent_id): Path<String>,
    Json(mut payload): Json<AiSdkChatRequest>,
) -> Result<Response, ApiError> {
    payload.agent_id = Some(agent_id);
    ai_sdk_chat_inner(st, payload).await
}

async fn ai_sdk_chat_inner(st: AppState, payload: AiSdkChatRequest) -> Result<Response, ApiError> {
    let agent_id = payload.agent_id;
    let messages = convert_messages(payload.messages);
    let (thread_id, messages) = crate::request::prepare_run_inputs(payload.thread_id, messages)?;
    let messages = crate::request::inject_frontend_context(messages, payload.state);

    let mut request = RunRequest::new(thread_id.clone(), messages);
    if let Some(id) = agent_id {
        request = request.with_agent_id(id);
    }
    let (_result, event_rx) = st
        .mailbox
        .submit(request)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let replay_buffer = Arc::new(EventReplayBuffer::new(st.config.replay_buffer_capacity));

    // Register buffer by thread_id (not run_id). External consumers only see
    // threads — runs are internal state. This matches AI SDK's reconnect URL
    // pattern: `{api}/{chatId}/stream` where chatId = threadId.
    st.replay_buffers
        .lock()
        .insert(thread_id.clone(), Arc::clone(&replay_buffer));

    let encoder = AiSdkEncoder::new();
    let sse_rx =
        wire_sse_relay_resumable(event_rx, encoder, st.config.sse_buffer_size, replay_buffer);

    // Spawn cleanup task: forward frames to client, but keep buffer alive for
    // the full run duration (not tied to client connection). This allows
    // reconnecting clients to use resume_stream even after the original client
    // disconnects.
    let buffers = Arc::clone(&st.replay_buffers);
    let replay_buf_for_cleanup = Arc::clone(st.replay_buffers.lock().get(&thread_id).unwrap());
    let cleanup_thread_id = thread_id;
    let mut sse_rx_forwarded = sse_rx;
    let (final_tx, final_rx) = tokio::sync::mpsc::channel::<Bytes>(st.config.sse_buffer_size);
    tokio::spawn(async move {
        while let Some(frame) = sse_rx_forwarded.recv().await {
            // Best-effort forward; ignore send errors (client gone).
            // Keep consuming so the relay task doesn't stall.
            let _ = final_tx.send(frame).await;
        }
        // Run is done — close subscribers so reconnected clients get EOF,
        // then remove the buffer from registry.
        replay_buf_for_cleanup.close_subscribers();
        buffers.lock().remove(&cleanup_thread_id);
    });

    Ok(sse_response(sse_body_stream(final_rx)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_contract::contract::content::ContentBlock;
    use serde_json::json;

    // ── UIMessage deserialization ──

    #[test]
    fn deserialize_user_text_message() {
        let raw = json!({
            "id": "msg-1",
            "role": "user",
            "parts": [{"type": "text", "text": "hello"}]
        });
        let msg: UIMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(msg.role, "user");
        assert_eq!(msg.parts.len(), 1);
        assert!(matches!(&msg.parts[0], UIPart::Text { text } if text == "hello"));
    }

    #[test]
    fn deserialize_message_with_file_part() {
        let raw = json!({
            "id": "msg-2",
            "role": "user",
            "parts": [
                {"type": "text", "text": "Look at this"},
                {"type": "file", "url": "https://example.com/img.png", "mediaType": "image/png"}
            ]
        });
        let msg: UIMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(msg.parts.len(), 2);
        assert!(
            matches!(&msg.parts[1], UIPart::File { url, media_type } if url == "https://example.com/img.png" && media_type == "image/png")
        );
    }

    #[test]
    fn deserialize_tool_parts_as_other() {
        let raw = json!({
            "id": "msg-3",
            "role": "assistant",
            "parts": [
                {"type": "text", "text": "Let me search"},
                {"type": "tool-search", "toolCallId": "c1", "state": "output-available", "input": {}, "output": "results"},
                {"type": "step-start"}
            ]
        });
        let msg: UIMessage = serde_json::from_value(raw).unwrap();
        assert_eq!(msg.parts.len(), 3);
        // tool-search → Other, step-start → StepStart
        assert!(matches!(&msg.parts[1], UIPart::Other));
        assert!(matches!(&msg.parts[2], UIPart::StepStart));
    }

    // ── convert_messages ──

    #[test]
    fn convert_user_text_message() {
        let msgs = vec![UIMessage {
            id: Some("m1".into()),
            role: "user".into(),
            parts: vec![UIPart::Text {
                text: "hello".into(),
            }],
        }];
        let converted = convert_messages(msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].text(), "hello");
    }

    #[test]
    fn convert_user_message_with_image_file() {
        let msgs = vec![UIMessage {
            id: Some("m2".into()),
            role: "user".into(),
            parts: vec![
                UIPart::Text {
                    text: "Describe".into(),
                },
                UIPart::File {
                    url: "https://example.com/img.png".into(),
                    media_type: "image/png".into(),
                },
            ],
        }];
        let converted = convert_messages(msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].content.len(), 2);
        assert_eq!(converted[0].content[0], ContentBlock::text("Describe"));
        assert_eq!(
            converted[0].content[1],
            ContentBlock::image_url("https://example.com/img.png")
        );
    }

    #[test]
    fn convert_user_message_with_base64_image() {
        let msgs = vec![UIMessage {
            id: None,
            role: "user".into(),
            parts: vec![UIPart::File {
                url: "data:image/png;base64,iVBOR".into(),
                media_type: "image/png".into(),
            }],
        }];
        let converted = convert_messages(msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(
            converted[0].content[0],
            ContentBlock::image_base64("image/png", "iVBOR")
        );
    }

    #[test]
    fn convert_user_message_with_audio_file() {
        let msgs = vec![UIMessage {
            id: None,
            role: "user".into(),
            parts: vec![UIPart::File {
                url: "https://example.com/audio.mp3".into(),
                media_type: "audio/mpeg".into(),
            }],
        }];
        let converted = convert_messages(msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(
            converted[0].content[0],
            ContentBlock::audio_url("https://example.com/audio.mp3")
        );
    }

    #[test]
    fn convert_skips_unknown_roles() {
        let msgs = vec![UIMessage {
            id: None,
            role: "function".into(),
            parts: vec![UIPart::Text {
                text: "result".into(),
            }],
        }];
        let converted = convert_messages(msgs);
        assert!(converted.is_empty());
    }

    #[test]
    fn convert_skips_messages_with_only_non_content_parts() {
        let msgs = vec![UIMessage {
            id: None,
            role: "assistant".into(),
            parts: vec![UIPart::StepStart, UIPart::Other],
        }];
        let converted = convert_messages(msgs);
        assert!(converted.is_empty());
    }

    #[test]
    fn convert_system_message() {
        let msgs = vec![UIMessage {
            id: None,
            role: "system".into(),
            parts: vec![UIPart::Text {
                text: "You are helpful".into(),
            }],
        }];
        let converted = convert_messages(msgs);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].text(), "You are helpful");
    }

    #[test]
    fn convert_empty_messages() {
        let converted = convert_messages(vec![]);
        assert!(converted.is_empty());
    }

    #[test]
    fn parse_data_uri_valid() {
        let (mime, data) = parse_data_uri("data:image/jpeg;base64,/9j/4AA").unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(data, "/9j/4AA");
    }

    #[test]
    fn parse_data_uri_invalid() {
        assert!(parse_data_uri("https://example.com/img.png").is_none());
        assert!(parse_data_uri("data:image/png,raw-data").is_none());
    }
}
