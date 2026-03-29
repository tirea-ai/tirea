use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single A2UI v0.8 server-to-client message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiMessage {
    /// Populate the component map for a surface.
    #[serde(
        rename = "surfaceUpdate",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub surface_update: Option<A2uiSurfaceUpdate>,
    /// Populate or mutate the surface data model.
    #[serde(
        rename = "dataModelUpdate",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub data_model_update: Option<A2uiDataModelUpdate>,
    /// Signal the root component for initial render.
    #[serde(
        rename = "beginRendering",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub begin_rendering: Option<A2uiBeginRendering>,
    /// Delete a surface.
    #[serde(
        rename = "deleteSurface",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub delete_surface: Option<A2uiDeleteSurface>,
}

/// Parameters for starting a surface render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiBeginRendering {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub styles: Option<HashMap<String, String>>,
}

/// Parameters for updating components on a surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiSurfaceUpdate {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub components: Vec<A2uiComponent>,
}

/// A single raw A2UI component definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiComponent {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// v0.8 component payload, e.g. `{ "Text": { ... } }`.
    pub component: Value,
}

/// Parameters for updating the data model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiDataModelUpdate {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub contents: Vec<A2uiDataModelEntry>,
}

/// A single data model entry in the v0.8 `contents` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiDataModelEntry {
    pub key: String,
    #[serde(
        rename = "valueString",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub value_string: Option<String>,
    #[serde(
        rename = "valueNumber",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub value_number: Option<f64>,
    #[serde(
        rename = "valueBoolean",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub value_boolean: Option<bool>,
    #[serde(rename = "valueMap", default, skip_serializing_if = "Option::is_none")]
    pub value_map: Option<Vec<A2uiDataModelEntry>>,
}

/// Parameters for deleting a surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiDeleteSurface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
}
