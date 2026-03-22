//! A2UI (Agent-to-UI) extension for awaken.
//!
//! Provides a tool-based interface for agents to emit declarative UI messages.
//! The LLM calls `A2uiRenderTool` with A2UI JSON payloads; the tool validates
//! the structure and returns the validated payload as its result, which flows
//! through the event stream to the frontend.
//!
//! # Protocol overview
//!
//! A2UI v0.9 defines four message types:
//! - `createSurface` — initialize a rendering surface
//! - `updateComponents` — define/update the component tree
//! - `updateDataModel` — populate or change data values
//! - `deleteSurface` — remove a surface

use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::plugins::{Plugin, PluginDescriptor, PluginRegistrar};
use crate::state::MutationBatch;
use awaken_contract::StateError;
use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};
use awaken_contract::registry_spec::AgentSpec;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const A2UI_TOOL_ID: &str = "render_a2ui";
const A2UI_TOOL_NAME: &str = "render_a2ui";
const A2UI_PLUGIN_ID: &str = "a2ui";
const SUPPORTED_VERSION: &str = "v0.9";
const MESSAGE_KEYS: &[&str] = &[
    "createSurface",
    "updateComponents",
    "updateDataModel",
    "deleteSurface",
];

// ---------------------------------------------------------------------------
// A2UI validation
// ---------------------------------------------------------------------------

/// A2UI validation error.
#[derive(Debug, Clone)]
pub struct A2uiValidationError {
    pub index: usize,
    pub message: String,
}

impl fmt::Display for A2uiValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "message[{}]: {}", self.index, self.message)
    }
}

/// Validate an array of A2UI messages.
///
/// Returns errors found (at most one per message).
/// Empty `Vec` means all messages are valid.
pub fn validate_a2ui_messages(messages: &[Value]) -> Vec<A2uiValidationError> {
    messages
        .iter()
        .enumerate()
        .filter_map(|(i, msg)| validate_single(i, msg))
        .collect()
}

fn validate_single(index: usize, msg: &Value) -> Option<A2uiValidationError> {
    let err = |m: &str| {
        Some(A2uiValidationError {
            index,
            message: m.to_string(),
        })
    };

    let obj = match msg.as_object() {
        Some(o) => o,
        None => return err("expected a JSON object"),
    };

    // version check
    match obj.get("version").and_then(Value::as_str) {
        Some(v) if v == SUPPORTED_VERSION => {}
        Some(v) => {
            return err(&format!(
                "unsupported version \"{v}\", expected \"{SUPPORTED_VERSION}\""
            ));
        }
        None => return err("missing required field \"version\""),
    }

    // exactly one message-type key
    let type_keys: Vec<&&str> = MESSAGE_KEYS
        .iter()
        .filter(|k| obj.contains_key(**k))
        .collect();
    match type_keys.len() {
        0 => {
            return err(&format!(
                "missing message type; expected one of: {}",
                MESSAGE_KEYS.join(", ")
            ));
        }
        1 => {}
        _ => {
            let names: Vec<&str> = type_keys.into_iter().copied().collect();
            return err(&format!(
                "multiple message types in one object: {}",
                names.join(", ")
            ));
        }
    }

    let type_key = type_keys[0];
    let body = match obj.get(*type_key).and_then(Value::as_object) {
        Some(b) => b,
        None => return err(&format!("\"{type_key}\" must be a JSON object")),
    };

    // surfaceId required on all message types
    match body.get("surfaceId").and_then(Value::as_str) {
        Some("") => return err(&format!("\"{type_key}.surfaceId\" must not be empty")),
        Some(_) => {}
        None => return err(&format!("\"{type_key}.surfaceId\" is required")),
    }

    // type-specific checks
    match *type_key {
        "createSurface" => {
            if body.get("catalogId").and_then(Value::as_str).is_none() {
                return err("\"createSurface.catalogId\" is required");
            }
        }
        "updateComponents" => match body.get("components").and_then(Value::as_array) {
            Some(arr) if arr.is_empty() => {
                return err("\"updateComponents.components\" must not be empty");
            }
            Some(arr) => {
                for (ci, comp) in arr.iter().enumerate() {
                    let comp_obj = match comp.as_object() {
                        Some(o) => o,
                        None => {
                            return err(&format!(
                                "\"updateComponents.components[{ci}]\" must be a JSON object"
                            ));
                        }
                    };
                    if comp_obj.get("id").and_then(Value::as_str).is_none() {
                        return err(&format!(
                            "\"updateComponents.components[{ci}].id\" is required"
                        ));
                    }
                    if comp_obj.get("component").and_then(Value::as_str).is_none() {
                        return err(&format!(
                            "\"updateComponents.components[{ci}].component\" is required"
                        ));
                    }
                }
            }
            None => {
                return err("\"updateComponents.components\" is required and must be an array");
            }
        },
        "updateDataModel" | "deleteSurface" => {
            // surfaceId already checked above
        }
        _ => unreachable!(),
    }

    None
}

// ---------------------------------------------------------------------------
// A2UI protocol types
// ---------------------------------------------------------------------------

/// A single A2UI v0.9 message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiMessage {
    /// Protocol version, must be "v0.9".
    pub version: String,
    /// Create a new surface.
    #[serde(
        rename = "createSurface",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub create_surface: Option<A2uiCreateSurface>,
    /// Update components on a surface.
    #[serde(
        rename = "updateComponents",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub update_components: Option<A2uiUpdateComponents>,
    /// Update the data model of a surface.
    #[serde(
        rename = "updateDataModel",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub update_data_model: Option<A2uiUpdateDataModel>,
    /// Delete a surface.
    #[serde(
        rename = "deleteSurface",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub delete_surface: Option<A2uiDeleteSurface>,
}

/// Parameters for creating a new A2UI surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiCreateSurface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    #[serde(rename = "catalogId")]
    pub catalog_id: String,
}

/// Parameters for updating components on a surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiUpdateComponents {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub components: Vec<A2uiComponent>,
}

/// A single A2UI component definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiComponent {
    pub id: String,
    pub component: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

/// Parameters for updating the data model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiUpdateDataModel {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub path: String,
    pub value: Value,
}

/// Parameters for deleting a surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2uiDeleteSurface {
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
}

// ---------------------------------------------------------------------------
// A2uiRenderTool
// ---------------------------------------------------------------------------

/// Tool for rendering A2UI declarative UI.
///
/// The LLM calls this tool with an array of A2UI messages (v0.9). The tool
/// validates the structural integrity and returns the validated payload.
pub struct A2uiRenderTool {
    _private: (),
}

impl A2uiRenderTool {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for A2uiRenderTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for A2uiRenderTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            A2UI_TOOL_ID,
            A2UI_TOOL_NAME,
            "Sends A2UI JSON to the client to render declarative UI. \
             Each message must be a v0.9 A2UI object with exactly one of: \
             createSurface, updateComponents, updateDataModel, or deleteSurface.",
        )
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        let messages = args
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ToolError::InvalidArguments("missing required field \"messages\"".into())
            })?;

        if messages.is_empty() {
            return Err(ToolError::InvalidArguments(
                "messages array must not be empty".into(),
            ));
        }

        let errors = validate_a2ui_messages(messages);
        if errors.is_empty() {
            Ok(())
        } else {
            let details: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            Err(ToolError::InvalidArguments(format!(
                "A2UI validation failed: {}",
                details.join("; ")
            )))
        }
    }

    async fn execute(&self, args: Value, _ctx: &ToolCallContext) -> Result<ToolResult, ToolError> {
        let messages = args
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        tracing::debug!(
            count = messages.len(),
            "A2UI render tool: validated {} message(s)",
            messages.len()
        );

        Ok(ToolResult::success(
            A2UI_TOOL_NAME,
            json!({
                "a2ui": messages,
                "rendered": true,
            }),
        ))
    }
}

// ---------------------------------------------------------------------------
// A2uiPlugin
// ---------------------------------------------------------------------------

/// A2UI plugin that provides the render tool and prompt instructions.
pub struct A2uiPlugin {
    instructions: String,
}

impl A2uiPlugin {
    /// Create with a specific catalog ID.
    pub fn with_catalog_id(catalog_id: &str) -> Self {
        Self {
            instructions: build_instructions(catalog_id, None),
        }
    }

    /// Create with a catalog ID and custom examples.
    pub fn with_catalog_and_examples(catalog_id: &str, examples: &str) -> Self {
        Self {
            instructions: build_instructions(catalog_id, Some(examples)),
        }
    }

    /// Create with fully custom instructions.
    pub fn with_custom_instructions(instructions: String) -> Self {
        Self { instructions }
    }

    /// Returns the instructions that will be injected.
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

impl Plugin for A2uiPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: A2UI_PLUGIN_ID,
        }
    }

    fn register(&self, _registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        // A2UI plugin is stateless — instructions are injected via the phase hook system
        Ok(())
    }

    fn on_activate(
        &self,
        _agent_spec: &AgentSpec,
        _patch: &mut MutationBatch,
    ) -> Result<(), StateError> {
        Ok(())
    }
}

fn build_instructions(catalog_id: &str, examples: Option<&str>) -> String {
    let mut s = String::from(A2UI_SCHEMA_INSTRUCTIONS);
    s = s.replace("{{CATALOG_ID}}", catalog_id);
    if let Some(examples) = examples {
        s.push_str("\n\n---BEGIN A2UI EXAMPLES---\n");
        s.push_str(examples);
        s.push_str("\n---END A2UI EXAMPLES---");
    }
    s
}

const A2UI_SCHEMA_INSTRUCTIONS: &str = r#"
## A2UI Declarative UI

You have access to the `render_a2ui` tool to send declarative UI to the client.
Call this tool with an array of A2UI v0.9 messages. Each message must be a JSON
object with `"version": "v0.9"` and exactly one of:

### Message Types

1. **createSurface** — Initialize a UI surface:
   ```json
   {"version": "v0.9", "createSurface": {"surfaceId": "<id>", "catalogId": "{{CATALOG_ID}}"}}
   ```

2. **updateComponents** — Define the component tree (flat adjacency list):
   ```json
   {"version": "v0.9", "updateComponents": {"surfaceId": "<id>", "components": [
     {"id": "root", "component": "Card", "child": "col"},
     {"id": "col", "component": "Column", "children": ["title", "input"]},
     {"id": "title", "component": "Text", "text": "Hello"},
     {"id": "input", "component": "TextField", "label": "Name", "value": {"path": "/name"}}
   ]}}
   ```

3. **updateDataModel** — Populate data for the surface:
   ```json
   {"version": "v0.9", "updateDataModel": {"surfaceId": "<id>", "path": "/", "value": {"name": ""}}}
   ```

4. **deleteSurface** — Remove a surface:
   ```json
   {"version": "v0.9", "deleteSurface": {"surfaceId": "<id>"}}
   ```

### Rules
- Always send `createSurface` first, then `updateComponents`, then `updateDataModel`.
- Components are a flat list; use `children` (array of IDs) or `child` (single ID) to form a tree.
- One component must have `"id": "root"`.
- Each component needs `"id"` and `"component"` (the type name from the catalog).
- Use `{"path": "/key"}` for data binding in component properties.
- Use `"action": {"event": {"name": "...", "context": {...}}}` on Button for user interactions.
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CATALOG: &str = "https://a2ui.org/specification/v0_9/basic_catalog.json";

    // -- Validation tests --

    #[test]
    fn valid_create_surface() {
        let msgs = vec![json!({
            "version": "v0.9",
            "createSurface": {
                "surfaceId": "s1",
                "catalogId": TEST_CATALOG,
            }
        })];
        assert!(validate_a2ui_messages(&msgs).is_empty());
    }

    #[test]
    fn valid_update_components() {
        let msgs = vec![json!({
            "version": "v0.9",
            "updateComponents": {
                "surfaceId": "s1",
                "components": [
                    {"id": "root", "component": "Card", "child": "col"},
                    {"id": "col", "component": "Column", "children": ["title"]},
                    {"id": "title", "component": "Text", "text": "Hello"}
                ]
            }
        })];
        assert!(validate_a2ui_messages(&msgs).is_empty());
    }

    #[test]
    fn valid_update_data_model() {
        let msgs = vec![json!({
            "version": "v0.9",
            "updateDataModel": {
                "surfaceId": "s1",
                "path": "/user",
                "value": {"name": "Alice"}
            }
        })];
        assert!(validate_a2ui_messages(&msgs).is_empty());
    }

    #[test]
    fn valid_delete_surface() {
        let msgs = vec![json!({
            "version": "v0.9",
            "deleteSurface": {"surfaceId": "s1"}
        })];
        assert!(validate_a2ui_messages(&msgs).is_empty());
    }

    #[test]
    fn rejects_missing_version() {
        let msgs = vec![json!({
            "createSurface": {"surfaceId": "s1", "catalogId": "c"}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("version"));
    }

    #[test]
    fn rejects_wrong_version() {
        let msgs = vec![json!({
            "version": "v0.8",
            "createSurface": {"surfaceId": "s1", "catalogId": "c"}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("unsupported version"));
    }

    #[test]
    fn rejects_no_message_type() {
        let msgs = vec![json!({"version": "v0.9"})];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("missing message type"));
    }

    #[test]
    fn rejects_multiple_message_types() {
        let msgs = vec![json!({
            "version": "v0.9",
            "createSurface": {"surfaceId": "s1", "catalogId": "c"},
            "deleteSurface": {"surfaceId": "s1"}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("multiple message types"));
    }

    #[test]
    fn rejects_missing_surface_id() {
        let msgs = vec![json!({
            "version": "v0.9",
            "deleteSurface": {}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("surfaceId"));
    }

    #[test]
    fn rejects_empty_surface_id() {
        let msgs = vec![json!({
            "version": "v0.9",
            "createSurface": {"surfaceId": "", "catalogId": "c"}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("must not be empty"));
    }

    #[test]
    fn rejects_missing_catalog_id() {
        let msgs = vec![json!({
            "version": "v0.9",
            "createSurface": {"surfaceId": "s1"}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("catalogId"));
    }

    #[test]
    fn rejects_empty_components() {
        let msgs = vec![json!({
            "version": "v0.9",
            "updateComponents": {"surfaceId": "s1", "components": []}
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("must not be empty"));
    }

    #[test]
    fn rejects_component_missing_id() {
        let msgs = vec![json!({
            "version": "v0.9",
            "updateComponents": {
                "surfaceId": "s1",
                "components": [{"component": "Text", "text": "hi"}]
            }
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("components[0].id"));
    }

    #[test]
    fn rejects_component_missing_component_type() {
        let msgs = vec![json!({
            "version": "v0.9",
            "updateComponents": {
                "surfaceId": "s1",
                "components": [{"id": "root"}]
            }
        })];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("components[0].component"));
    }

    #[test]
    fn reports_per_message_errors() {
        let msgs = vec![
            json!({"version": "v0.9", "deleteSurface": {"surfaceId": "ok"}}),
            json!({"version": "v0.9"}),
            json!({"version": "v0.9", "deleteSurface": {"surfaceId": "ok"}}),
            json!(42),
        ];
        let errs = validate_a2ui_messages(&msgs);
        assert_eq!(errs.len(), 2);
        assert_eq!(errs[0].index, 1);
        assert_eq!(errs[1].index, 3);
    }

    // -- Tool tests --

    #[test]
    fn tool_descriptor_has_correct_id() {
        let tool = A2uiRenderTool::new();
        let desc = tool.descriptor();
        assert_eq!(desc.id, A2UI_TOOL_ID);
        assert_eq!(desc.name, A2UI_TOOL_NAME);
        assert!(desc.description.contains("A2UI"));
    }

    #[test]
    fn tool_validates_missing_messages() {
        let tool = A2uiRenderTool::new();
        assert!(tool.validate_args(&json!({})).is_err());
    }

    #[test]
    fn tool_validates_empty_messages() {
        let tool = A2uiRenderTool::new();
        let err = tool.validate_args(&json!({"messages": []})).unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[test]
    fn tool_validates_valid_messages() {
        let tool = A2uiRenderTool::new();
        let args = json!({"messages": [
            {
                "version": "v0.9",
                "createSurface": {
                    "surfaceId": "s1",
                    "catalogId": TEST_CATALOG,
                }
            }
        ]});
        assert!(tool.validate_args(&args).is_ok());
    }

    #[test]
    fn tool_validates_invalid_a2ui() {
        let tool = A2uiRenderTool::new();
        let args = json!({"messages": [{"version": "v0.9"}]});
        let err = tool.validate_args(&args).unwrap_err();
        assert!(err.to_string().contains("missing message type"));
    }

    #[tokio::test]
    async fn tool_execute_returns_validated_payload() {
        let tool = A2uiRenderTool::new();
        let ctx = ToolCallContext::test_default();
        let args = json!({"messages": [
            {
                "version": "v0.9",
                "createSurface": {"surfaceId": "s1", "catalogId": TEST_CATALOG}
            },
            {
                "version": "v0.9",
                "updateComponents": {
                    "surfaceId": "s1",
                    "components": [{"id": "root", "component": "Card"}]
                }
            }
        ]});

        let result = tool.execute(args, &ctx).await.unwrap();
        assert_eq!(result.tool_name, A2UI_TOOL_NAME);
        assert_eq!(result.data["rendered"], true);
        assert!(result.data["a2ui"].is_array());
        assert_eq!(result.data["a2ui"].as_array().unwrap().len(), 2);
    }

    // -- Plugin tests --

    #[test]
    fn plugin_id() {
        let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
        assert_eq!(plugin.descriptor().name, A2UI_PLUGIN_ID);
    }

    #[test]
    fn plugin_instructions_contain_catalog() {
        let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
        assert!(plugin.instructions().contains(TEST_CATALOG));
        assert!(!plugin.instructions().contains("{{CATALOG_ID}}"));
    }

    #[test]
    fn plugin_with_examples() {
        let plugin = A2uiPlugin::with_catalog_and_examples(TEST_CATALOG, "example content");
        assert!(plugin.instructions().contains("---BEGIN A2UI EXAMPLES---"));
        assert!(plugin.instructions().contains("example content"));
        assert!(plugin.instructions().contains("---END A2UI EXAMPLES---"));
    }

    #[test]
    fn plugin_custom_instructions() {
        let plugin = A2uiPlugin::with_custom_instructions("Use A2UI.".into());
        assert_eq!(plugin.instructions(), "Use A2UI.");
    }

    #[test]
    fn plugin_instructions_mention_all_message_types() {
        let plugin = A2uiPlugin::with_catalog_id(TEST_CATALOG);
        let text = plugin.instructions();
        assert!(text.contains("render_a2ui"));
        assert!(text.contains("createSurface"));
        assert!(text.contains("updateComponents"));
        assert!(text.contains("updateDataModel"));
        assert!(text.contains("deleteSurface"));
    }
}
