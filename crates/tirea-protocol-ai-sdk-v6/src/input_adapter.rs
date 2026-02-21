use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tirea_contract::{InteractionResponse, Message, ProtocolInputAdapter, RunRequest};

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "AiSdkV6MessagesRunRequest")]
pub struct AiSdkV6RunRequest {
    pub thread_id: String,
    pub input: String,
    pub run_id: Option<String>,
    interaction_responses: Vec<InteractionResponse>,
}

#[derive(Debug, Clone, Deserialize)]
struct AiSdkV6MessagesRunRequest {
    #[serde(default)]
    id: Option<String>,
    // Legacy fields are rejected for AI SDK v6 UI transport.
    #[serde(rename = "sessionId", default)]
    legacy_session_id: Option<String>,
    #[serde(rename = "input", default)]
    legacy_input: Option<String>,
    #[serde(default)]
    messages: Vec<AiSdkIncomingMessage>,
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AiSdkIncomingMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    parts: Vec<Value>,
}

impl TryFrom<AiSdkV6MessagesRunRequest> for AiSdkV6RunRequest {
    type Error = String;

    fn try_from(req: AiSdkV6MessagesRunRequest) -> Result<Self, Self::Error> {
        if req.legacy_session_id.is_some() || req.legacy_input.is_some() {
            return Err(
                "legacy AI SDK payload shape is no longer supported; use id/messages".to_string(),
            );
        }

        let thread_id = req.id.unwrap_or_default();
        let input = extract_last_user_text(&req.messages).unwrap_or_default();
        let interaction_responses = extract_interaction_responses(&req.messages);
        Ok(Self {
            thread_id,
            input,
            run_id: req.run_id,
            interaction_responses,
        })
    }
}

impl AiSdkV6RunRequest {
    /// Build a request from explicit thread/input values (non-UI transport path).
    pub fn from_thread_input(
        thread_id: impl Into<String>,
        input: impl Into<String>,
        run_id: Option<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            input: input.into(),
            run_id,
            interaction_responses: Vec::new(),
        }
    }

    /// Whether the incoming request includes a non-empty user input message.
    pub fn has_user_input(&self) -> bool {
        !self.input.trim().is_empty()
    }

    /// Whether the incoming request includes any interaction responses.
    pub fn has_interaction_responses(&self) -> bool {
        !self.interaction_responses.is_empty()
    }

    /// Interaction responses extracted from incoming UI messages.
    pub fn interaction_responses(&self) -> Vec<InteractionResponse> {
        self.interaction_responses.clone()
    }
}

fn extract_last_user_text(messages: &[AiSdkIncomingMessage]) -> Option<String> {
    for message in messages.iter().rev() {
        let Some(role) = message.role.as_deref() else {
            continue;
        };
        if !role.eq_ignore_ascii_case("user") {
            continue;
        }

        if let Some(Value::String(content)) = &message.content {
            return Some(content.clone());
        }

        if !message.parts.is_empty() {
            let text = extract_text_from_parts(&message.parts);
            if !text.is_empty() {
                return Some(text);
            }
        }

        if let Some(Value::Array(content_parts)) = &message.content {
            let text = extract_text_from_parts(content_parts);
            if !text.is_empty() {
                return Some(text);
            }
        }
    }

    None
}

fn extract_interaction_responses(messages: &[AiSdkIncomingMessage]) -> Vec<InteractionResponse> {
    let mut latest_by_id: HashMap<String, (usize, Value)> = HashMap::new();
    let mut ordinal = 0usize;

    for message in messages {
        let Some(role) = message.role.as_deref() else {
            continue;
        };
        if !role.eq_ignore_ascii_case("assistant") {
            continue;
        }

        for part in message_parts(message) {
            if let Some((interaction_id, result)) = parse_interaction_response_part(part) {
                latest_by_id.insert(interaction_id, (ordinal, result));
                ordinal += 1;
            }
        }
    }

    let mut responses: Vec<(usize, InteractionResponse)> = latest_by_id
        .into_iter()
        .map(|(interaction_id, (idx, result))| {
            (idx, InteractionResponse::new(interaction_id, result))
        })
        .collect();
    responses.sort_by_key(|(idx, _)| *idx);
    responses
        .into_iter()
        .map(|(_, response)| response)
        .collect()
}

fn message_parts(message: &AiSdkIncomingMessage) -> Vec<&Value> {
    if !message.parts.is_empty() {
        return message.parts.iter().collect();
    }
    if let Some(Value::Array(parts)) = &message.content {
        return parts.iter().collect();
    }
    Vec::new()
}

fn parse_interaction_response_part(part: &Value) -> Option<(String, Value)> {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
    let state = part
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let tool_call_id = part
        .get("toolCallId")
        .or_else(|| part.get("tool_call_id"))
        .and_then(Value::as_str)
        .map(str::to_string);

    if part_type == "tool-approval-response" {
        let approval_id = part
            .get("approvalId")
            .and_then(Value::as_str)
            .map(str::to_string)?;
        let approved = part
            .get("approved")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let reason = part
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        return Some((approval_id, approval_response_value(approved, reason)));
    }

    match state {
        "approval-responded" => {
            let approval = part.get("approval");
            let interaction_id = approval
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(tool_call_id)?;
            let approved = approval
                .and_then(|v| v.get("approved"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let reason = approval
                .and_then(|v| v.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string);
            Some((interaction_id, approval_response_value(approved, reason)))
        }
        "output-available" => {
            let interaction_id = tool_call_id?;
            let output = part.get("output").cloned().unwrap_or(Value::Null);
            Some((interaction_id, output))
        }
        "output-denied" => {
            let interaction_id = tool_call_id?;
            Some((interaction_id, Value::Bool(false)))
        }
        "output-error" => {
            let interaction_id = tool_call_id?;
            let error = part
                .get("errorText")
                .and_then(Value::as_str)
                .unwrap_or("tool output error");
            Some((
                interaction_id,
                serde_json::json!({
                    "approved": false,
                    "error": error,
                }),
            ))
        }
        _ => None,
    }
}

fn approval_response_value(approved: bool, reason: Option<String>) -> Value {
    let mut result = serde_json::Map::new();
    result.insert("approved".to_string(), Value::Bool(approved));
    if let Some(reason) = reason {
        result.insert("reason".to_string(), Value::String(reason));
    }
    Value::Object(result)
}

fn extract_text_from_parts(parts: &[Value]) -> String {
    let mut text = String::new();
    for part in parts {
        let Some(part_type) = part.get("type").and_then(Value::as_str) else {
            continue;
        };
        if part_type != "text" {
            continue;
        }
        if let Some(segment) = part.get("text").and_then(Value::as_str) {
            text.push_str(segment);
        }
    }
    text
}

pub struct AiSdkV6InputAdapter;

impl ProtocolInputAdapter for AiSdkV6InputAdapter {
    type Request = AiSdkV6RunRequest;

    fn to_run_request(agent_id: String, request: Self::Request) -> RunRequest {
        let mut messages = Vec::new();
        if request.has_user_input() {
            messages.push(Message::user(request.input));
        }
        RunRequest {
            agent_id,
            thread_id: if request.thread_id.trim().is_empty() {
                None
            } else {
                Some(request.thread_id)
            },
            run_id: request.run_id,
            parent_run_id: None,
            resource_id: None,
            state: None,
            messages,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_legacy_request_shape() {
        let err = serde_json::from_value::<AiSdkV6RunRequest>(json!({
            "sessionId": "thread-1",
            "input": "hello",
            "runId": "run-1"
        }))
        .expect_err("legacy payload must be rejected");
        assert!(
            err.to_string()
                .contains("legacy AI SDK payload shape is no longer supported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn deserializes_messages_request_shape_using_last_user_text() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "thread-from-id",
            "messages": [
                { "role": "user", "parts": [{ "type": "text", "text": "first" }] },
                { "role": "assistant", "parts": [{ "type": "text", "text": "ignored" }] },
                { "role": "user", "parts": [{ "type": "text", "text": "final" }, { "type": "file", "url": "u" }] }
            ],
            "runId": "run-2"
        }))
        .expect("messages payload should deserialize");

        assert_eq!(req.thread_id, "thread-from-id");
        assert_eq!(req.input, "final");
        assert_eq!(req.run_id.as_deref(), Some("run-2"));
    }

    #[test]
    fn id_is_used_as_thread_id_in_messages_shape() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "thread-id",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("messages payload should deserialize");

        assert_eq!(req.thread_id, "thread-id");
        assert_eq!(req.input, "hello");
    }

    #[test]
    fn missing_user_text_in_messages_shape_defaults_to_empty_input() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "thread-1",
            "messages": [{ "role": "assistant", "content": "no-user" }]
        }))
        .expect("messages payload should deserialize");

        assert_eq!(req.thread_id, "thread-1");
        assert_eq!(req.input, "");
    }

    #[test]
    fn extracts_approval_responded_parts_as_interaction_responses() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t1",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-echo",
                        "toolCallId": "call_echo_1",
                        "state": "approval-responded",
                        "approval": {
                            "id": "fc_perm_1",
                            "approved": true,
                            "reason": "looks safe"
                        }
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "fc_perm_1");
        assert_eq!(responses[0].result["approved"], true);
        assert_eq!(responses[0].result["reason"], "looks safe");
    }

    #[test]
    fn extracts_tool_approval_response_parts_as_interaction_responses() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t1b",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-approval-response",
                        "approvalId": "fc_perm_7",
                        "approved": false,
                        "reason": "denied by user"
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "fc_perm_7");
        assert_eq!(responses[0].result["approved"], false);
        assert_eq!(responses[0].result["reason"], "denied by user");
    }

    #[test]
    fn extracts_output_available_parts_as_interaction_responses() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t2",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-askUserQuestion",
                        "toolCallId": "ask_call_1",
                        "state": "output-available",
                        "output": {"answer":"blue"}
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "ask_call_1");
        assert_eq!(responses[0].result["answer"], "blue");
    }

    #[test]
    fn output_denied_part_maps_to_denied_response() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t3",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "dynamic-tool",
                        "toolCallId": "call_1",
                        "state": "output-denied"
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "call_1");
        assert_eq!(responses[0].result, Value::Bool(false));
    }

    #[test]
    fn output_error_part_maps_to_error_response() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t4",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "dynamic-tool",
                        "toolCallId": "call_err_1",
                        "state": "output-error",
                        "errorText": "frontend failed"
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "call_err_1");
        assert_eq!(responses[0].result["approved"], false);
        assert_eq!(responses[0].result["error"], "frontend failed");
    }

    #[test]
    fn output_error_without_error_text_uses_default_message() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t4b",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "dynamic-tool",
                        "toolCallId": "call_err_default",
                        "state": "output-error"
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "call_err_default");
        assert_eq!(responses[0].result["approved"], false);
        assert_eq!(responses[0].result["error"], "tool output error");
    }

    #[test]
    fn approval_responded_without_approval_id_falls_back_to_tool_call_id() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t5",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-echo",
                        "toolCallId": "fc_perm_fallback",
                        "state": "approval-responded",
                        "approval": {
                            "approved": true
                        }
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "fc_perm_fallback");
        assert_eq!(responses[0].result["approved"], true);
    }

    #[test]
    fn tool_approval_response_without_reason_only_contains_approved_field() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t5b",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-approval-response",
                        "approvalId": "fc_perm_10",
                        "approved": true
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "fc_perm_10");
        assert_eq!(responses[0].result["approved"], true);
        assert!(responses[0].result.get("reason").is_none());
    }

    #[test]
    fn latest_interaction_response_wins_for_same_interaction_id() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "t6",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-PermissionConfirm",
                        "toolCallId": "fc_perm_9",
                        "state": "approval-responded",
                        "approval": {
                            "id": "fc_perm_9",
                            "approved": true
                        }
                    }]
                },
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-PermissionConfirm",
                        "toolCallId": "fc_perm_9",
                        "state": "approval-responded",
                        "approval": {
                            "id": "fc_perm_9",
                            "approved": false,
                            "reason": "user changed mind"
                        }
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        let responses = req.interaction_responses();
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].interaction_id, "fc_perm_9");
        assert_eq!(responses[0].result["approved"], false);
        assert_eq!(responses[0].result["reason"], "user changed mind");
    }

    #[test]
    fn interaction_only_messages_generate_empty_run_messages() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "thread-int-only",
            "messages": [
                {
                    "role": "assistant",
                    "parts": [{
                        "type": "tool-askUserQuestion",
                        "toolCallId": "ask_1",
                        "state": "output-available",
                        "output": {"message":"blue"}
                    }]
                }
            ]
        }))
        .expect("messages payload should deserialize");

        assert!(!req.has_user_input());
        assert!(req.has_interaction_responses());
        let run_request = AiSdkV6InputAdapter::to_run_request("agent".to_string(), req);
        assert!(run_request.messages.is_empty());
    }
}
