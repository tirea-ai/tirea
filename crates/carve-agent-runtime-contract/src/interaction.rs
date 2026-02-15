use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runtime key carrying request-scoped frontend tool names.
pub const RUNTIME_INTERACTION_FRONTEND_TOOLS_KEY: &str = "interaction_frontend_tools";
/// Runtime key carrying request-scoped interaction responses.
pub const RUNTIME_INTERACTION_RESPONSES_KEY: &str = "interaction_responses";

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
