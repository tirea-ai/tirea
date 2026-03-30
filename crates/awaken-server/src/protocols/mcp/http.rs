//! MCP HTTP transport: Axum routes for MCP Streamable HTTP.
//!
//! Implements the MCP Streamable HTTP transport specification:
//! - `POST /v1/mcp` — accepts a JSON-RPC request, returns a JSON-RPC response.
//! - `GET  /v1/mcp` — SSE endpoint for server-initiated messages (notifications).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;
use tokio::sync::mpsc;

use mcp::protocol::{ClientInbound, JsonRpcId, JsonRpcRequest, ServerOutbound};
use mcp::server::McpServer;

use crate::app::AppState;

/// Shared state for the MCP protocol layer.
///
/// Held in an `Arc` inside `AppState` (via extension) so that all MCP routes
/// share the same server instance and channels.
pub struct McpState {
    pub server: Arc<McpServer>,
    pub inbound_tx: mpsc::Sender<ClientInbound>,
}

/// Build MCP routes.
///
/// The MCP server is lazily created from `AppState.runtime` on first access.
/// We store it as an Axum extension for subsequent requests.
pub fn mcp_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/mcp", post(mcp_post))
        .route("/v1/mcp", get(mcp_sse))
}

/// POST /v1/mcp — JSON-RPC request/response.
///
/// Accepts a single JSON-RPC 2.0 message, dispatches it through the MCP server,
/// and returns the response synchronously.
async fn mcp_post(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, McpApiError> {
    // Parse the JSON-RPC request.
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let id = body
        .get("id")
        .map(|v| {
            if let Some(n) = v.as_i64() {
                JsonRpcId::Number(n)
            } else if let Some(s) = v.as_str() {
                JsonRpcId::String(s.to_string())
            } else {
                JsonRpcId::Null
            }
        })
        .unwrap_or(JsonRpcId::Null);

    let params = body.get("params").cloned();

    if method.is_empty() {
        return Err(McpApiError::BadRequest("missing 'method' field".into()));
    }

    // Check if this is a notification (no id field in original body).
    let is_notification = body.get("id").is_none_or(|v| v.is_null());

    // Create a dedicated McpServer per request pair to avoid channel contention.
    // For a production-grade implementation, you'd use a session-based approach.
    let (_server, mut channels) = super::create_mcp_server(&st.runtime);

    if is_notification {
        // Notifications don't expect a response.
        let notification = mcp::JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        };
        let _ = channels
            .inbound_tx
            .send(ClientInbound::Notification(notification))
            .await;
        return Ok(StatusCode::ACCEPTED.into_response());
    }

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: id.clone(),
        method,
        params,
    };

    channels
        .inbound_tx
        .send(ClientInbound::Request(request))
        .await
        .map_err(|_| McpApiError::Internal("server channel closed".into()))?;

    // Read the response from the outbound channel.
    // Skip notifications (progress, logs) and wait for the actual Response.
    loop {
        let outbound = channels
            .outbound_rx
            .recv()
            .await
            .ok_or_else(|| McpApiError::Internal("no response from MCP server".into()))?;

        match outbound {
            ServerOutbound::Response(resp) => {
                let json = serde_json::to_value(&resp)
                    .map_err(|e| McpApiError::Internal(format!("serialization error: {e}")))?;
                return Ok(Json(json).into_response());
            }
            ServerOutbound::Notification(_) | ServerOutbound::Request(_) => {
                // Skip notifications/server-requests, keep waiting for the response.
                continue;
            }
        }
    }
}

/// GET /v1/mcp — SSE endpoint for server-initiated messages.
///
/// Returns a simple informational response for now (SSE streaming for
/// server-initiated notifications can be added when needed).
async fn mcp_sse(State(_st): State<AppState>) -> Response {
    // MCP Streamable HTTP: the GET endpoint is for server-initiated messages.
    // For tool-call-only scenarios (no server push), return 405.
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(serde_json::json!({
            "error": "SSE endpoint not yet implemented; use POST /v1/mcp for requests"
        })),
    )
        .into_response()
}

/// MCP-specific API error.
#[derive(Debug)]
pub enum McpApiError {
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for McpApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            McpApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            McpApiError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };
        (
            status,
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32600,
                    "message": message
                },
                "id": null
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awaken_runtime::{AgentResolver, AgentRuntime, ResolvedAgent, RuntimeError};
    use awaken_stores::InMemoryMailboxStore;
    use awaken_stores::memory::InMemoryStore;
    use serde_json::json;

    struct StubResolver;
    impl AgentResolver for StubResolver {
        fn resolve(&self, agent_id: &str) -> Result<ResolvedAgent, RuntimeError> {
            Err(RuntimeError::AgentNotFound {
                agent_id: agent_id.to_string(),
            })
        }
        fn agent_ids(&self) -> Vec<String> {
            vec!["echo-agent".into()]
        }
    }

    fn make_app_state() -> AppState {
        let runtime = Arc::new(AgentRuntime::new(Arc::new(StubResolver)));
        let store = Arc::new(InMemoryStore::new());
        let mailbox_store = Arc::new(InMemoryMailboxStore::new());
        let mailbox = Arc::new(crate::mailbox::Mailbox::new(
            Arc::clone(&runtime),
            mailbox_store,
            "test".to_string(),
            crate::mailbox::MailboxConfig::default(),
        ));
        AppState::new(
            runtime,
            mailbox,
            store,
            Arc::new(StubResolver),
            crate::app::ServerConfig::default(),
        )
    }

    #[tokio::test]
    async fn post_initialize_returns_protocol_version() {
        let app = Router::new()
            .merge(mcp_routes())
            .with_state(make_app_state());

        let body = json!({
            "jsonrpc": "2.0",
            "method": "initialize",
            "id": 1
        });

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/mcp")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert!(json["result"]["protocolVersion"].is_string());
    }

    #[tokio::test]
    async fn post_tools_list_returns_agent_tools() {
        let app = Router::new()
            .merge(mcp_routes())
            .with_state(make_app_state());

        let body = json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 2
        });

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/mcp")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let tools = json["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "echo-agent");
    }

    #[tokio::test]
    async fn post_notification_returns_accepted() {
        let app = Router::new()
            .merge(mcp_routes())
            .with_state(make_app_state());

        let body = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/mcp")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn post_missing_method_returns_bad_request() {
        let app = Router::new()
            .merge(mcp_routes())
            .with_state(make_app_state());

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1
        });

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/mcp")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();

        let response = tower::ServiceExt::oneshot(app, request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
