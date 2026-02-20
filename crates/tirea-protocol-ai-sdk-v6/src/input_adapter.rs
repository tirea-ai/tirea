use serde::Deserialize;
use serde_json::Value;
use tirea_contract::{Message, ProtocolInputAdapter, RunRequest};

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "AiSdkV6RunRequestWire")]
pub struct AiSdkV6RunRequest {
    pub thread_id: String,
    pub input: String,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum AiSdkV6RunRequestWire {
    Legacy(AiSdkV6LegacyRunRequest),
    Messages(AiSdkV6MessagesRunRequest),
}

#[derive(Debug, Clone, Deserialize)]
struct AiSdkV6LegacyRunRequest {
    #[serde(rename = "sessionId")]
    thread_id: String,
    input: String,
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AiSdkV6MessagesRunRequest {
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "sessionId", default)]
    session_id: Option<String>,
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

impl TryFrom<AiSdkV6RunRequestWire> for AiSdkV6RunRequest {
    type Error = String;

    fn try_from(value: AiSdkV6RunRequestWire) -> Result<Self, Self::Error> {
        match value {
            AiSdkV6RunRequestWire::Legacy(req) => Ok(Self {
                thread_id: req.thread_id,
                input: req.input,
                run_id: req.run_id,
            }),
            AiSdkV6RunRequestWire::Messages(req) => {
                let thread_id = req.session_id.or(req.id).unwrap_or_default();
                let input = extract_last_user_text(&req.messages).unwrap_or_default();
                Ok(Self {
                    thread_id,
                    input,
                    run_id: req.run_id,
                })
            }
        }
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
            messages: vec![Message::user(request.input)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_legacy_request_shape() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "sessionId": "thread-1",
            "input": "hello",
            "runId": "run-1"
        }))
        .expect("legacy payload should deserialize");

        assert_eq!(req.thread_id, "thread-1");
        assert_eq!(req.input, "hello");
        assert_eq!(req.run_id.as_deref(), Some("run-1"));
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
    fn session_id_takes_precedence_over_id_in_messages_shape() {
        let req: AiSdkV6RunRequest = serde_json::from_value(json!({
            "id": "thread-id",
            "sessionId": "thread-session",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("messages payload should deserialize");

        assert_eq!(req.thread_id, "thread-session");
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
}
