use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Generic suspension request for client-side actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suspension {
    /// Unique suspension ID.
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

impl Suspension {
    /// Create a new suspension with id and action.
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

/// Generic suspension response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionResponse {
    /// The suspension target ID this response is for.
    pub target_id: String,
    /// Result value (structure defined by the action type).
    pub result: Value,
}

impl SuspensionResponse {
    fn deny_string_token(value: &str) -> bool {
        matches!(
            value,
            "false"
                | "no"
                | "denied"
                | "deny"
                | "reject"
                | "rejected"
                | "cancel"
                | "canceled"
                | "cancelled"
                | "abort"
                | "aborted"
        )
    }

    fn object_deny_flag(obj: &serde_json::Map<String, Value>) -> bool {
        [
            "denied",
            "reject",
            "rejected",
            "cancel",
            "canceled",
            "cancelled",
            "abort",
            "aborted",
        ]
        .iter()
        .any(|key| obj.get(*key).and_then(Value::as_bool).unwrap_or(false))
            || ["status", "decision", "action"].iter().any(|key| {
                obj.get(*key)
                    .and_then(Value::as_str)
                    .map(|v| Self::deny_string_token(&v.trim().to_lowercase()))
                    .unwrap_or(false)
            })
    }

    /// Create a new suspension response.
    pub fn new(target_id: impl Into<String>, result: Value) -> Self {
        Self {
            target_id: target_id.into(),
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
                let lower = s.trim().to_lowercase();
                Self::deny_string_token(&lower)
            }
            Value::Object(obj) => {
                obj.get("approved")
                    .and_then(|v| v.as_bool())
                    .map(|v| !v)
                    .unwrap_or(false)
                    || Self::object_deny_flag(obj)
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
/// Replaces the `Suspension` struct for frontend tool call tracking. Each
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

    /// Convert to an `Suspension` for backward compatibility with the
    /// existing event system during the transition period.
    pub fn to_suspension(&self) -> Suspension {
        Suspension::new(&self.call_id, format!("tool:{}", self.tool_name))
            .with_parameters(self.arguments.clone())
    }
}

/// Where a frontend tool invocation originated from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InvocationOrigin {
    /// A backend tool call was intercepted by a gate/policy plugin.
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
    /// Used when an external decision allows replaying the intercepted
    /// backend tool call.
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
    use super::{ResponseRouting, SuspensionResponse};
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
                "path": ["__legacy", "state_patches", "call_1"],
                "value": true
            }]
        });
        let routing: ResponseRouting =
            serde_json::from_value(value).expect("deserialize legacy replay routing");
        assert_eq!(routing, ResponseRouting::ReplayOriginalTool);
    }

    #[test]
    fn suspension_response_treats_cancel_variants_as_denied() {
        let denied_cases = [
            json!("cancelled"),
            json!("canceled"),
            json!({"status":"cancelled"}),
            json!({"decision":"abort"}),
            json!({"canceled": true}),
            json!({"cancelled": true}),
        ];
        for case in denied_cases {
            assert!(
                SuspensionResponse::is_denied(&case),
                "expected denied for case: {case}"
            );
        }
    }
}
