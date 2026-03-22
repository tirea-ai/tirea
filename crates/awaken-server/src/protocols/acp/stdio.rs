//! JSON-RPC 2.0 stdio server for ACP protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 notification envelope (no id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response.
    pub fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
            id,
        }
    }

    /// Method not found error (-32601).
    pub fn method_not_found(id: Option<Value>) -> Self {
        Self::error(id, -32601, "Method not found")
    }

    /// Invalid params error (-32602).
    pub fn invalid_params(id: Option<Value>, message: impl Into<String>) -> Self {
        Self::error(id, -32602, message)
    }

    /// Internal error (-32603).
    pub fn internal_error(id: Option<Value>, message: impl Into<String>) -> Self {
        Self::error(id, -32603, message)
    }
}

impl JsonRpcNotification {
    /// Create a notification.
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            method: method.into(),
            params: Some(params),
        }
    }
}

/// Parse a JSON-RPC 2.0 request from a line.
pub fn parse_request(line: &str) -> Result<JsonRpcRequest, String> {
    serde_json::from_str(line).map_err(|e| format!("invalid JSON-RPC request: {e}"))
}

/// Serialize a JSON-RPC 2.0 response to a line.
pub fn serialize_response(response: &JsonRpcResponse) -> String {
    serde_json::to_string(response).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"serialization error"},"id":null}"#
            .to_string()
    })
}

/// Serialize a JSON-RPC 2.0 notification to a line.
pub fn serialize_notification(notification: &JsonRpcNotification) -> String {
    serde_json::to_string(notification)
        .unwrap_or_else(|_| r#"{"jsonrpc":"2.0","method":"error","params":null}"#.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_valid_request() {
        let line = r#"{"jsonrpc":"2.0","method":"session/start","params":{"agentId":"a1"},"id":1}"#;
        let req = parse_request(line).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "session/start");
        assert_eq!(req.id, Some(json!(1)));
    }

    #[test]
    fn parse_notification_without_id() {
        let line = r#"{"jsonrpc":"2.0","method":"session/update","params":{"text":"hi"}}"#;
        let req = parse_request(line).unwrap();
        assert!(req.id.is_none());
    }

    #[test]
    fn parse_invalid_json() {
        let result = parse_request("not json");
        assert!(result.is_err());
    }

    #[test]
    fn success_response_serde() {
        let resp = JsonRpcResponse::success(Some(json!(1)), json!({"ok": true}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn error_response_serde() {
        let resp = JsonRpcResponse::error(Some(json!(1)), -32600, "Invalid Request");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32600"));
        assert!(json.contains("Invalid Request"));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn method_not_found_response() {
        let resp = JsonRpcResponse::method_not_found(Some(json!(1)));
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
    }

    #[test]
    fn invalid_params_response() {
        let resp = JsonRpcResponse::invalid_params(Some(json!(1)), "missing field");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert_eq!(err.message, "missing field");
    }

    #[test]
    fn notification_serde() {
        let notif = JsonRpcNotification::new("session/update", json!({"text": "hello"}));
        let json = serialize_notification(&notif);
        assert!(json.contains("session/update"));
        assert!(json.contains("hello"));
    }

    #[test]
    fn serialize_response_handles_all_cases() {
        let success = serialize_response(&JsonRpcResponse::success(None, json!(42)));
        assert!(success.contains("42"));

        let error = serialize_response(&JsonRpcResponse::internal_error(None, "boom"));
        assert!(error.contains("boom"));
    }

    #[test]
    fn roundtrip_request() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: "test/method".into(),
            params: Some(json!({"key": "val"})),
            id: Some(json!("req-1")),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.method, "test/method");
        assert_eq!(parsed.id, Some(json!("req-1")));
    }
}
