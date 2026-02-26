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

#[cfg(test)]
mod tests {
    use super::SuspensionResponse;
    use serde_json::json;

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
