use async_trait::async_trait;
use serde_json::{Value, json};

use awaken_contract::contract::tool::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};
use awaken_contract::validate_against_schema;

use super::validation::validate_a2ui_messages;
use super::{A2UI_TOOL_ID, A2UI_TOOL_NAME, MESSAGE_KEYS};

/// Tool for rendering A2UI declarative UI.
///
/// The LLM passes A2UI v0.8 message keys directly as tool arguments. The tool
/// schema stays intentionally message-level and defers component-shape details
/// to prompt guidance plus server-side validation. This keeps the function
/// signature small enough for real models to use reliably.
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

/// Extract the A2UI messages array from LLM-provided args.
///
/// Accepts multiple input shapes:
/// 1. A2UI keys at top level → single-element array
///    `{"surfaceUpdate": {...}}` → `[{"surfaceUpdate": {...}}]`
/// 2. Legacy `messages` array → use directly
///    `{"messages": [{...}, {...}]}` → `[{...}, {...}]`
fn extract_messages(args: &Value) -> Vec<Value> {
    if let Some(arr) = args.get("messages").and_then(Value::as_array) {
        return arr.clone();
    }

    if let Some(obj) = args.as_object() {
        if MESSAGE_KEYS.iter().any(|key| obj.contains_key(*key)) {
            return vec![args.clone()];
        }
    }

    vec![]
}

fn non_empty_string_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1
    })
}

fn component_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "component"],
        "properties": {
            "id": non_empty_string_schema(),
            "weight": { "type": "number" },
            "component": {
                "type": "object",
                "minProperties": 1,
                "maxProperties": 1,
                "additionalProperties": {
                    "type": "object"
                },
                "description": "A single official A2UI v0.8 component payload such as {\"Text\": {...}}."
            }
        }
    })
}

fn data_model_entry_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["key"],
        "properties": {
            "key": non_empty_string_schema(),
            "valueString": { "type": "string" },
            "valueNumber": { "type": "number" },
            "valueBoolean": { "type": "boolean" },
            "valueMap": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": true
                }
            }
        }
    })
}

fn surface_update_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["surfaceId", "components"],
        "properties": {
            "surfaceId": non_empty_string_schema(),
            "components": {
                "type": "array",
                "minItems": 1,
                "items": component_schema(),
                "description": "Flat component list. Every item must include id and component."
            }
        }
    })
}

fn data_model_update_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["surfaceId", "contents"],
        "properties": {
            "surfaceId": non_empty_string_schema(),
            "path": { "type": "string" },
            "contents": {
                "type": "array",
                "minItems": 1,
                "items": data_model_entry_schema()
            }
        }
    })
}

fn begin_rendering_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["surfaceId", "root"],
        "properties": {
            "surfaceId": non_empty_string_schema(),
            "root": non_empty_string_schema(),
            "styles": {
                "type": "object",
                "additionalProperties": { "type": "string" }
            }
        }
    })
}

fn delete_surface_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["surfaceId"],
        "properties": {
            "surfaceId": non_empty_string_schema()
        }
    })
}

fn message_variant_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["surfaceUpdate"],
                "properties": { "surfaceUpdate": surface_update_schema() }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["dataModelUpdate"],
                "properties": { "dataModelUpdate": data_model_update_schema() }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["beginRendering"],
                "properties": { "beginRendering": begin_rendering_schema() }
            },
            {
                "type": "object",
                "additionalProperties": false,
                "required": ["deleteSurface"],
                "properties": { "deleteSurface": delete_surface_schema() }
            }
        ]
    })
}

fn tool_parameters_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "minProperties": 1,
        "properties": {
            "surfaceUpdate": surface_update_schema(),
            "dataModelUpdate": data_model_update_schema(),
            "beginRendering": begin_rendering_schema(),
            "deleteSurface": delete_surface_schema(),
            "messages": {
                "type": "array",
                "minItems": 1,
                "items": message_variant_schema(),
                "description": "Optional batch format. Prefer the flat single-message shape above."
            }
        },
        "description": "Provide exactly one of surfaceUpdate, dataModelUpdate, beginRendering, deleteSurface, or messages."
    })
}

#[async_trait]
impl Tool for A2uiRenderTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            A2UI_TOOL_ID,
            A2UI_TOOL_NAME,
            "Sends A2UI v0.8 declarative UI to the client. Pass exactly one of \
             surfaceUpdate, dataModelUpdate, beginRendering, or deleteSurface \
             as a top-level key.",
        )
        .with_parameters(tool_parameters_schema())
    }

    fn validate_args(&self, args: &Value) -> Result<(), ToolError> {
        let messages = extract_messages(args);

        if messages.is_empty() {
            return Err(ToolError::InvalidArguments(
                "expected at least one A2UI message key (surfaceUpdate, dataModelUpdate, beginRendering, or deleteSurface)"
                    .into(),
            ));
        }

        validate_against_schema(&tool_parameters_schema(), args)?;

        let errors = validate_a2ui_messages(&messages);
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
        let messages = extract_messages(&args);

        tracing::debug!(
            count = messages.len(),
            "A2UI render tool: validated {} message(s)",
            messages.len()
        );

        Ok(ToolResult::success(
            A2UI_TOOL_NAME,
            json!({ "rendered": true, "count": messages.len() }),
        ))
    }
}
