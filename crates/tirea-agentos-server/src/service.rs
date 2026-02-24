use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tirea_agentos::contracts::storage::{
    MessagePage, MessageQuery, SortOrder, ThreadReader, ThreadStore, ThreadStoreError,
};
use tirea_agentos::contracts::thread::Visibility;
use tirea_agentos::contracts::ToolCallDecision;
use tirea_agentos::orchestrator::{AgentOs, AgentOsRunError};
use tirea_contract::ProtocolHistoryEncoder;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub os: Arc<AgentOs>,
    pub read_store: Arc<dyn ThreadReader>,
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

#[derive(Default)]
struct ActiveRunRegistry {
    decisions: RwLock<HashMap<String, tokio::sync::mpsc::UnboundedSender<ToolCallDecision>>>,
}

impl ActiveRunRegistry {
    async fn register(
        &self,
        key: String,
        decision_tx: tokio::sync::mpsc::UnboundedSender<ToolCallDecision>,
    ) {
        self.decisions.write().await.insert(key, decision_tx);
    }

    async fn decision_tx_for(
        &self,
        key: &str,
    ) -> Option<tokio::sync::mpsc::UnboundedSender<ToolCallDecision>> {
        self.decisions.read().await.get(key).cloned()
    }

    async fn remove(&self, key: &str) {
        self.decisions.write().await.remove(key);
    }
}

static ACTIVE_RUN_REGISTRY: OnceLock<ActiveRunRegistry> = OnceLock::new();

fn active_run_registry() -> &'static ActiveRunRegistry {
    ACTIVE_RUN_REGISTRY.get_or_init(ActiveRunRegistry::default)
}

pub fn active_run_key(protocol: &str, agent_id: &str, thread_id: &str) -> String {
    format!("{protocol}:{agent_id}:{thread_id}")
}

pub async fn register_active_run(
    key: String,
    decision_tx: tokio::sync::mpsc::UnboundedSender<ToolCallDecision>,
) {
    active_run_registry().register(key, decision_tx).await;
}

pub async fn remove_active_run(key: &str) {
    active_run_registry().remove(key).await;
}

pub async fn try_forward_decisions_to_active_run(
    active_key: &str,
    decisions: Vec<ToolCallDecision>,
) -> bool {
    if decisions.is_empty() {
        return false;
    }

    let Some(decision_tx) = active_run_registry().decision_tx_for(active_key).await else {
        return false;
    };

    for decision in decisions {
        if decision_tx.send(decision).is_err() {
            active_run_registry().remove(active_key).await;
            return false;
        }
    }

    true
}

fn default_message_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
pub struct MessageQueryParams {
    #[serde(default)]
    pub after: Option<i64>,
    #[serde(default)]
    pub before: Option<i64>,
    #[serde(default = "default_message_limit")]
    pub limit: usize,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

pub fn parse_message_query(params: &MessageQueryParams) -> MessageQuery {
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

pub async fn load_message_page(
    read_store: &Arc<dyn ThreadReader>,
    thread_id: &str,
    params: &MessageQueryParams,
) -> Result<MessagePage, ApiError> {
    let query = parse_message_query(params);
    read_store
        .load_messages(thread_id, &query)
        .await
        .map_err(|e| match e {
            ThreadStoreError::NotFound(_) => ApiError::ThreadNotFound(thread_id.to_string()),
            other => ApiError::Internal(other.to_string()),
        })
}

#[derive(Debug, Serialize)]
pub struct EncodedMessagePage<M: Serialize> {
    pub messages: Vec<M>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_cursor: Option<i64>,
}

pub fn encode_message_page<E: ProtocolHistoryEncoder>(
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

pub fn require_agent_state_store(os: &Arc<AgentOs>) -> Result<Arc<dyn ThreadStore>, ApiError> {
    os.agent_state_store()
        .cloned()
        .ok_or_else(|| ApiError::Internal("agent state store not configured".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_run_key_includes_protocol_agent_thread() {
        assert_eq!(
            active_run_key("ag_ui", "assistant", "thread-1"),
            "ag_ui:assistant:thread-1"
        );
    }

    #[test]
    fn parse_message_query_defaults_and_visibility() {
        let params = MessageQueryParams {
            after: None,
            before: None,
            limit: 999,
            order: None,
            visibility: None,
            run_id: None,
        };
        let query = parse_message_query(&params);
        assert_eq!(query.limit, 200);
        assert!(matches!(query.order, SortOrder::Asc));
        assert!(matches!(query.visibility, Some(Visibility::All)));

        let params = MessageQueryParams {
            after: None,
            before: None,
            limit: 1,
            order: Some("desc".to_string()),
            visibility: Some("internal".to_string()),
            run_id: Some("r1".to_string()),
        };
        let query = parse_message_query(&params);
        assert_eq!(query.limit, 1);
        assert!(matches!(query.order, SortOrder::Desc));
        assert!(matches!(query.visibility, Some(Visibility::Internal)));
        assert_eq!(query.run_id.as_deref(), Some("r1"));
    }
}
