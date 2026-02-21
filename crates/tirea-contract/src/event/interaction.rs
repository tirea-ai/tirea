use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Generic interaction request for client-side actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Interaction {
    /// Unique interaction ID.
    pub id: String,
    /// Action identifier (freeform string, meaning defined by caller).
    pub action: String,
    /// Human-readable message/description.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Action-specific parameters.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub parameters: Value,
    /// Optional JSON Schema for expected response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
}

impl Interaction {
    /// Create a new interaction with id and action.
    pub fn new(id: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            action: action.into(),
            message: String::new(),
            parameters: Value::Null,
            response_schema: None,
        }
    }

    /// Set the message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Set the parameters.
    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = parameters;
        self
    }

    /// Set the response schema.
    pub fn with_response_schema(mut self, schema: Value) -> Self {
        self.response_schema = Some(schema);
        self
    }
}

/// Generic interaction response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    /// The interaction ID this response is for.
    pub interaction_id: String,
    /// Result value (structure defined by the action type).
    pub result: Value,
}

impl InteractionResponse {
    /// Create a new interaction response.
    pub fn new(interaction_id: impl Into<String>, result: Value) -> Self {
        Self {
            interaction_id: interaction_id.into(),
            result,
        }
    }

    /// Check if a result value indicates approval.
    pub fn is_approved(result: &Value) -> bool {
        match result {
            Value::Bool(b) => *b,
            Value::String(s) => {
                let lower = s.to_lowercase();
                matches!(
                    lower.as_str(),
                    "true" | "yes" | "approved" | "allow" | "confirm" | "ok" | "accept"
                )
            }
            Value::Object(obj) => {
                obj.get("approved")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || obj
                        .get("allowed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Check if a result value indicates denial.
    pub fn is_denied(result: &Value) -> bool {
        match result {
            Value::Bool(b) => !*b,
            Value::String(s) => {
                let lower = s.to_lowercase();
                matches!(
                    lower.as_str(),
                    "false" | "no" | "denied" | "deny" | "reject" | "cancel" | "abort"
                )
            }
            Value::Object(obj) => {
                obj.get("approved")
                    .and_then(|v| v.as_bool())
                    .map(|v| !v)
                    .unwrap_or(false)
                    || obj.get("denied").and_then(|v| v.as_bool()).unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Check if this response indicates approval.
    pub fn approved(&self) -> bool {
        Self::is_approved(&self.result)
    }

    /// Check if this response indicates denial.
    pub fn denied(&self) -> bool {
        Self::is_denied(&self.result)
    }
}

// ============================================================================
// Frontend Tool Invocation (first-class citizen model)
// ============================================================================

/// A frontend tool invocation record persisted to thread state.
///
/// Replaces the `Interaction` struct for frontend tool call tracking. Each
/// invocation captures the frontend tool being called, its origin context
/// (which plugin/tool triggered it), and the routing strategy for handling
/// the frontend's response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrontendToolInvocation {
    /// Unique ID for this frontend tool call (sent to frontend as `toolCallId`).
    pub call_id: String,
    /// Frontend tool name (e.g. "copyToClipboard", "PermissionConfirm").
    pub tool_name: String,
    /// Frontend tool arguments.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub arguments: Value,
    /// Where this invocation originated from.
    pub origin: InvocationOrigin,
    /// How to route the frontend's response.
    pub routing: ResponseRouting,
}

impl FrontendToolInvocation {
    /// Create a new frontend tool invocation.
    pub fn new(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
        origin: InvocationOrigin,
        routing: ResponseRouting,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
            origin,
            routing,
        }
    }

    /// Convert to an `Interaction` for backward compatibility with the
    /// existing event system during the transition period.
    pub fn to_interaction(&self) -> Interaction {
        Interaction::new(&self.call_id, format!("tool:{}", self.tool_name))
            .with_parameters(self.arguments.clone())
    }
}

/// Where a frontend tool invocation originated from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvocationOrigin {
    /// A backend tool call was intercepted (e.g. permission check).
    ToolCallIntercepted {
        /// The intercepted backend tool call ID.
        backend_call_id: String,
        /// The intercepted backend tool name.
        backend_tool_name: String,
        /// The intercepted backend tool arguments.
        backend_arguments: Value,
    },
    /// A plugin directly initiated the frontend tool call (no backend tool context).
    PluginInitiated {
        /// The plugin that initiated this call.
        plugin_id: String,
    },
}

/// How to route the frontend's response after it completes execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ResponseRoutingWire", into = "ResponseRoutingWire")]
pub enum ResponseRouting {
    /// Replay the original backend tool.
    ///
    /// Used for permission prompts where approval allows replaying the
    /// intercepted backend tool call.
    ReplayOriginalTool,
    /// The frontend result IS the tool result — inject it directly into
    /// the LLM message history as the tool call response.
    /// Used for direct frontend tools (e.g. copyToClipboard).
    UseAsToolResult,
    /// Pass the frontend result to the LLM as an independent message.
    PassToLLM,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
enum ResponseRoutingWire {
    /// Legacy shape accepted for backward compatibility.
    ReplayOriginalTool {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        state_patches: Vec<Value>,
    },
    /// The frontend result IS the tool result — inject it directly into
    /// the LLM message history as the tool call response.
    /// Used for direct frontend tools (e.g. copyToClipboard).
    UseAsToolResult,
    /// Pass the frontend result to the LLM as an independent message.
    PassToLLM,
}

impl From<ResponseRoutingWire> for ResponseRouting {
    fn from(value: ResponseRoutingWire) -> Self {
        match value {
            ResponseRoutingWire::ReplayOriginalTool { .. } => Self::ReplayOriginalTool,
            ResponseRoutingWire::UseAsToolResult => Self::UseAsToolResult,
            ResponseRoutingWire::PassToLLM => Self::PassToLLM,
        }
    }
}

impl From<ResponseRouting> for ResponseRoutingWire {
    fn from(value: ResponseRouting) -> Self {
        match value {
            ResponseRouting::ReplayOriginalTool => Self::ReplayOriginalTool {
                state_patches: Vec::new(),
            },
            ResponseRouting::UseAsToolResult => Self::UseAsToolResult,
            ResponseRouting::PassToLLM => Self::PassToLLM,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ResponseRouting;
    use serde_json::json;

    #[test]
    fn replay_original_tool_serializes_without_state_patches() {
        let value =
            serde_json::to_value(ResponseRouting::ReplayOriginalTool).expect("serialize routing");
        assert_eq!(value, json!({ "strategy": "replay_original_tool" }));
    }

    #[test]
    fn replay_original_tool_deserializes_legacy_state_patches_shape() {
        let value = json!({
            "strategy": "replay_original_tool",
            "state_patches": [{
                "op": "set",
                "path": ["permissions", "approved_calls", "call_1"],
                "value": true
            }]
        });
        let routing: ResponseRouting =
            serde_json::from_value(value).expect("deserialize legacy replay routing");
        assert_eq!(routing, ResponseRouting::ReplayOriginalTool);
    }
}
