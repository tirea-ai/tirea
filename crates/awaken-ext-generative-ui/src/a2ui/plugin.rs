use std::sync::Arc;

use awaken_contract::StateError;
use awaken_contract::registry_spec::AgentSpec;

use awaken_runtime::plugins::{Plugin, PluginDescriptor, PluginRegistrar};
use awaken_runtime::state::MutationBatch;

use super::tool::A2uiRenderTool;
use super::{A2UI_PLUGIN_ID, A2UI_TOOL_ID};

/// A2UI plugin that provides the render tool and prompt instructions.
pub struct A2uiPlugin {
    instructions: String,
}

impl A2uiPlugin {
    /// Create with a specific catalog URI description for prompt guidance.
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

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_tool(A2UI_TOOL_ID, Arc::new(A2uiRenderTool::new()))?;
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

You have access to the `render_a2ui` tool. Pass ONE A2UI message key directly
as the tool argument. Use the official A2UI v0.8 message format and do NOT add
wrapper objects beyond the protocol itself.

Call the tool once per message in this order: surfaceUpdate → dataModelUpdate → beginRendering.

### 1. surfaceUpdate
```json
{"surfaceUpdate": {"surfaceId": "ops_request", "components": [
  {"id": "root", "component": {"Card": {"child": "content"}}},
  {"id": "content", "component": {"Column": {"children": {"explicitList": ["title", "requester", "notes", "priority_label", "priority", "submit_label", "submit_button"]}}}},
  {"id": "title", "component": {"Text": {"usageHint": "h2", "text": {"literalString": "Operations Request"}}}},
  {"id": "requester", "component": {"TextField": {"label": {"literalString": "Requester"}, "text": {"path": "/request/requester"}, "textFieldType": "shortText"}}},
  {"id": "notes", "component": {"TextField": {"label": {"literalString": "Request details"}, "text": {"path": "/request/notes"}, "textFieldType": "longText"}}},
  {"id": "priority_label", "component": {"Text": {"usageHint": "caption", "text": {"literalString": "Priority"}}}},
  {"id": "priority", "component": {"MultipleChoice": {"selections": {"path": "/request/priority"}, "options": [{"label": {"literalString": "Standard"}, "value": "standard"}, {"label": {"literalString": "Urgent"}, "value": "urgent"}], "maxAllowedSelections": 1}}},
  {"id": "submit_label", "component": {"Text": {"text": {"literalString": "Submit request"}}}},
  {"id": "submit_button", "component": {"Button": {"child": "submit_label", "primary": true, "action": {"name": "ops_request.submit", "context": [{"key": "requester", "value": {"path": "/request/requester"}}, {"key": "notes", "value": {"path": "/request/notes"}}, {"key": "priority", "value": {"path": "/request/priority"}}]}}}}
]}}
```

### 2. dataModelUpdate
```json
{"dataModelUpdate": {"surfaceId": "ops_request", "path": "/request", "contents": [
  {"key": "requester", "valueString": ""},
  {"key": "notes", "valueString": ""},
  {"key": "priority", "valueMap": [{"key": "0", "valueString": "standard"}]}
]}}
```

### 3. beginRendering
```json
{"beginRendering": {"surfaceId": "ops_request", "root": "root"}}
```

### Rules
- The client defaults to the standard v0.8 catalog: `{{CATALOG_ID}}`.
- Components are a flat list. Relationships are expressed by component IDs inside nested component props.
- Each component must have `"id"` and `"component"`, where `"component"` is an object like `{"Text": {...}}`.
- `Text` components must always nest copy under the `text` field, for example `{"Text":{"text":{"literalString":"Submit"}}}`.
- For container children, use `{"children": {"explicitList": ["id1", "id2"]}}`.
- For text input binding, v0.8 `TextField` uses the `text` property, not `value`.
- For selection inputs, v0.8 `MultipleChoice` uses `options`, `selections`, and `maxAllowedSelections`.
- Each `MultipleChoice.options` item must look like `{"label":{"literalString":"High"},"value":"high"}`.
- `MultipleChoice` does not have a `label` field. If you need a visible label, render a nearby `Text` component.
- `MultipleChoice.selections` binds to an array path. Initialize defaults with `valueMap`, for example `{"key":"priority","valueMap":[{"key":"0","valueString":"standard"}]}`.
- For `DateTimeInput` defaults, use browser-compatible local strings like `2026-04-10T09:00`. Do not include a trailing `Z` or timezone offset.
- For buttons, use `"action": {"name": "...", "context": [{"key":"field","value":{"path":"/request/field"}}]}`; do not use the v0.9 `event` wrapper.
- `dataModelUpdate.contents` must be an array of `{key, valueString|valueNumber|valueBoolean|valueMap}` entries.
- Call `beginRendering` only after the referenced root component already exists in `surfaceUpdate`.
- Use `deleteSurface` when the workflow is complete or should be dismissed.
- When updating an existing surface after a user interaction, resend the current values for any bound fields that remain visible on the surface. Do not send only the newly changed status fields if the next UI still displays older inputs.
- If the conversation includes an `A2UI action:` payload, treat its `context` object as the authoritative source for the user's current bound field values unless the user explicitly changed them in the new turn.

### Update reminder
If a follow-up state still shows the original form fields, the next `dataModelUpdate` should carry both the retained field values and the new status fields, for example:
```json
{"dataModelUpdate":{"surfaceId":"surface","path":"/request","contents":[
  {"key":"requester","valueString":"Jordan Patel"},
  {"key":"department","valueString":"Operations"},
  {"key":"status","valueString":"submitted"}
]}}
```
"#;
