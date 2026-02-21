use async_trait::async_trait;
use axum::body::to_bytes;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tirea_agentos::contracts::tool::{Tool, ToolDescriptor, ToolError, ToolResult};
use tirea_agentos::contracts::ToolCallContext;
use tower::ServiceExt;

/// A deterministic calculator tool for E2E tests.
pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "calculator",
            "Calculator",
            "Perform basic arithmetic. Supports add, subtract, multiply, divide.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "subtract", "multiply", "divide"],
                    "description": "The arithmetic operation to perform"
                },
                "a": { "type": "number", "description": "First operand" },
                "b": { "type": "number", "description": "Second operand" }
            },
            "required": ["operation", "a", "b"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let op = args["operation"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("missing operation".into()))?;
        let a = args["a"]
            .as_f64()
            .ok_or_else(|| ToolError::InvalidArguments("missing a".into()))?;
        let b = args["b"]
            .as_f64()
            .ok_or_else(|| ToolError::InvalidArguments("missing b".into()))?;

        let result = match op {
            "add" => a + b,
            "subtract" => a - b,
            "multiply" => a * b,
            "divide" if b != 0.0 => a / b,
            "divide" => return Err(ToolError::ExecutionFailed("division by zero".into())),
            _ => return Err(ToolError::InvalidArguments(format!("unknown op: {op}"))),
        };

        Ok(ToolResult::success(
            "calculator",
            json!({ "result": result }),
        ))
    }
}

/// Send a POST request and return `(status, body_text)`.
pub async fn post_sse(app: axum::Router, uri: &str, payload: Value) -> (StatusCode, String) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(payload.to_string()))
                .expect("request build should succeed"),
        )
        .await
        .expect("app should handle request");

    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("response body must be utf-8");
    (status, text)
}

/// Send a GET request and return `(status, body_text)`.
#[allow(dead_code)]
pub async fn get_json_text(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(axum::body::Body::empty())
                .expect("request build should succeed"),
        )
        .await
        .expect("app should handle request");

    let status = resp.status();
    let body = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("response body should be readable");
    let text = String::from_utf8(body.to_vec()).expect("response body must be utf-8");
    (status, text)
}

/// Build an AI SDK v6 run payload using canonical `id/messages` fields.
pub fn ai_sdk_messages_payload(thread_id: &str, input: &str, run_id: Option<&str>) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("id".to_string(), Value::String(thread_id.to_string()));
    payload.insert(
        "messages".to_string(),
        json!([{"role": "user", "content": input}]),
    );
    if let Some(run_id) = run_id {
        payload.insert("runId".to_string(), Value::String(run_id.to_string()));
    }
    Value::Object(payload)
}

pub fn extract_ai_sdk_text(sse: &str) -> String {
    sse.lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("text-delta"))
        .filter_map(|v| v.get("delta").and_then(|t| t.as_str()).map(String::from))
        .collect()
}

pub fn extract_agui_text(sse: &str) -> String {
    sse.lines()
        .filter(|l| l.starts_with("data: "))
        .filter_map(|l| serde_json::from_str::<Value>(&l[6..]).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("TEXT_MESSAGE_CONTENT"))
        .filter_map(|v| v.get("delta").and_then(|d| d.as_str()).map(String::from))
        .collect()
}
