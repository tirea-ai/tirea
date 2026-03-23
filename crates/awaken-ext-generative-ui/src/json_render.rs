//! JSON Render format preset.

/// Activity type for JSON Render streaming events.
pub const ACTIVITY_TYPE: &str = "generative-ui.json-render";

/// Returns a system prompt instructing the sub-agent to output JSON Render format.
pub fn system_prompt(catalog_json: &str) -> String {
    format!(
        "You are a UI generation agent. Output ONLY a JSON Render document.\n\n\
         ## Format\n\
         ```json\n\
         {{\n  \"root\": \"elementId\",\n  \"elements\": {{\n    \
         \"elementId\": {{\n      \"type\": \"ComponentName\",\n      \
         \"props\": {{}},\n      \"children\": []\n    }}\n  }}\n}}\n\
         ```\n\n\
         ## Available Components\n\
         {catalog_json}\n\n\
         Output ONLY valid JSON. No markdown, no explanation."
    )
}
