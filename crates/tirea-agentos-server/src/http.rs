use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use tirea_agentos::contracts::storage::{MessagePage, ThreadListPage, ThreadListQuery};
use tirea_agentos::contracts::thread::Thread;

use crate::protocol;
use crate::service::{parse_message_query, ApiError, MessageQueryParams};

pub use crate::service::AppState;

/// Health endpoint path.
pub const HEALTH_PATH: &str = "/health";
/// Canonical thread list endpoint path.
pub const THREADS_PATH: &str = "/v1/threads";
/// Canonical thread detail endpoint path.
pub const THREAD_PATH: &str = "/v1/threads/:id";
/// Canonical thread messages endpoint path.
pub const THREAD_MESSAGES_PATH: &str = "/v1/threads/:id/messages";

/// Build health routes.
pub fn health_routes() -> Router<AppState> {
    Router::new().route(HEALTH_PATH, get(health))
}

/// Build canonical thread query routes.
pub fn thread_routes() -> Router<AppState> {
    Router::new()
        .route(THREADS_PATH, get(list_threads))
        .route(THREAD_PATH, get(get_thread))
        .route(THREAD_MESSAGES_PATH, get(get_thread_messages))
}

/// Deprecated combined router.
///
/// Prefer explicit assembly in application code:
/// - `health_routes()`
/// - `thread_routes()`
/// - `protocol::ag_ui::http::routes()`
/// - `protocol::ai_sdk_v6::http::routes()`
#[deprecated(
    since = "0.1.1",
    note = "assemble routes explicitly with health_routes/thread_routes and protocol::*::http::routes"
)]
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health_routes())
        .merge(thread_routes())
        .nest("/v1/ag-ui", protocol::ag_ui::http::routes())
        .nest("/v1/ai-sdk", protocol::ai_sdk_v6::http::routes())
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
        .load_thread(&id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        return Err(ApiError::ThreadNotFound(id));
    };
    Ok(Json(thread))
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
            tirea_agentos::contracts::storage::ThreadStoreError::NotFound(_) => {
                ApiError::ThreadNotFound(id)
            }
            other => ApiError::Internal(other.to_string()),
        })
}
