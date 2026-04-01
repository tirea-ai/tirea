//! Config management API routes — `/v1/config/:namespace/:id`.
//!
//! A single set of routes handles all ConfigStore namespaces (agents, models,
//! providers, mcp-servers). See ADR-0021.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use awaken_contract::contract::config_store::ConfigStore;

use crate::app::AppState;
use crate::routes::ApiError;

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    100
}

pub fn config_routes() -> Router<AppState> {
    use axum::routing::delete;
    Router::new()
        .route(
            "/v1/config/:namespace",
            get(list_config).post(create_config),
        )
        .route(
            "/v1/config/:namespace/:id",
            get(get_config).put(put_config).delete(delete_config),
        )
        .route("/v1/config/:namespace/$schema", get(get_schema))
        // Capabilities discovery
        .route("/v1/capabilities", get(get_capabilities))
        // Convenience aliases
        .route("/v1/agents", get(list_agents))
        .route("/v1/agents/:id", get(get_agent))
}

/// GET /v1/capabilities — list available components for agent construction.
async fn get_capabilities(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let agent_ids = state.resolver.agent_ids();

    let mut tools_list: Vec<Value> = Vec::new();
    let mut plugins_list: Vec<Value> = Vec::new();
    let mut models_list: Vec<Value> = Vec::new();
    let mut providers_list: Vec<Value> = Vec::new();

    if let Some(regs) = state.runtime.registry_set() {
        // Tools
        for id in regs.tools.tool_ids() {
            if let Some(tool) = regs.tools.get_tool(&id) {
                let desc = tool.descriptor();
                tools_list.push(json!({
                    "id": desc.id,
                    "name": desc.name,
                    "description": desc.description,
                }));
            }
        }

        // Plugins (with config schemas)
        for id in regs.plugins.plugin_ids() {
            if let Some(plugin) = regs.plugins.get_plugin(&id) {
                let schemas: Vec<Value> = plugin
                    .config_schemas()
                    .into_iter()
                    .map(|s| json!({"key": s.key, "schema": s.json_schema}))
                    .collect();
                plugins_list.push(json!({
                    "id": plugin.descriptor().name,
                    "config_schemas": schemas,
                }));
            }
        }

        // Models
        for id in regs.models.model_ids() {
            if let Some(model) = regs.models.get_model(&id) {
                models_list.push(json!({
                    "id": model.id,
                    "provider": model.provider,
                    "model": model.model,
                }));
            }
        }

        // Providers
        for id in regs.providers.provider_ids() {
            providers_list.push(json!({"id": id}));
        }
    }

    let namespaces = vec![
        json!({"namespace": "agents", "schema": schemars::schema_for!(awaken_contract::AgentSpec)}),
        json!({"namespace": "models", "schema": schemars::schema_for!(awaken_contract::ModelSpec)}),
        json!({"namespace": "providers", "schema": schemars::schema_for!(awaken_contract::ProviderSpec)}),
        json!({"namespace": "mcp-servers", "schema": schemars::schema_for!(awaken_contract::McpServerSpec)}),
    ];

    Ok(Json(json!({
        "agents": agent_ids,
        "tools": tools_list,
        "plugins": plugins_list,
        "models": models_list,
        "providers": providers_list,
        "namespaces": namespaces,
    })))
}

/// GET /v1/config/:namespace/$schema — JSON Schema for this namespace.
async fn get_schema(Path(namespace): Path<String>) -> Result<impl IntoResponse, ApiError> {
    let schema = match namespace.as_str() {
        "agents" => serde_json::to_value(schemars::schema_for!(awaken_contract::AgentSpec)),
        "models" => serde_json::to_value(schemars::schema_for!(awaken_contract::ModelSpec)),
        "providers" => serde_json::to_value(schemars::schema_for!(awaken_contract::ProviderSpec)),
        "mcp-servers" => {
            serde_json::to_value(schemars::schema_for!(awaken_contract::McpServerSpec))
        }
        _ => {
            return Err(ApiError::NotFound(format!(
                "unknown namespace: {namespace}"
            )));
        }
    }
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(schema))
}

async fn list_agents(
    state: State<AppState>,
    query: Query<ListParams>,
) -> Result<impl IntoResponse, ApiError> {
    list_config(state, Path("agents".to_string()), query).await
}

async fn get_agent(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    get_config(state, Path(("agents".to_string(), id))).await
}

fn require_store(state: &AppState) -> Result<Arc<dyn ConfigStore>, ApiError> {
    state
        .config_store
        .clone()
        .ok_or_else(|| ApiError::BadRequest("config management API not enabled".into()))
}

/// GET /v1/config/:namespace — list entries
async fn list_config(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Query(params): Query<ListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let entries = store
        .list(&namespace, params.offset, params.limit)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let items: Vec<Value> = entries.into_iter().map(|(_, v)| v).collect();
    Ok(Json(json!({
        "namespace": namespace,
        "items": items,
        "offset": params.offset,
        "limit": params.limit,
    })))
}

/// POST /v1/config/:namespace — create entry (ID extracted from body)
async fn create_config(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::BadRequest("missing 'id' field in body".into()))?;

    if store
        .exists(&namespace, id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        return Err(ApiError::BadRequest(format!(
            "{namespace}/{id} already exists"
        )));
    }

    store
        .put(&namespace, id, &body)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(body)))
}

/// GET /v1/config/:namespace/:id — get entry
async fn get_config(
    State(state): State<AppState>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    let value = store
        .get(&namespace, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("{namespace}/{id}")))?;

    Ok(Json(value))
}

/// PUT /v1/config/:namespace/:id — update/replace entry
async fn put_config(
    State(state): State<AppState>,
    Path((namespace, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    store
        .put(&namespace, &id, &body)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(body))
}

/// DELETE /v1/config/:namespace/:id — delete entry
async fn delete_config(
    State(state): State<AppState>,
    Path((namespace, id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let store = require_store(&state)?;
    store
        .delete(&namespace, &id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
