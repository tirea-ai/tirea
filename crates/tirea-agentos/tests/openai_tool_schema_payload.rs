#![allow(missing_docs)]

use genai::chat::{ChatMessage, ChatRequest, Tool};
use serde_json::{json, Value};
use std::io::ErrorKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

async fn start_capture_server() -> Option<(
    String,
    oneshot::Receiver<Value>,
    tokio::task::JoinHandle<()>,
)> {
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return None,
        Err(err) => panic!("failed to bind local test listener: {err}"),
    };
    let addr = listener.local_addr().expect("listener addr");
    let (tx, rx) = oneshot::channel();

    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];

        loop {
            let read = socket.read(&mut chunk).await.expect("read request");
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let request = String::from_utf8(buf).expect("request should be utf8");
        let body_start = request
            .find("\r\n\r\n")
            .map(|idx| idx + 4)
            .expect("request should contain headers/body separator");
        let headers = &request[..body_start];
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                (name.eq_ignore_ascii_case("content-length")).then_some(value.trim())
            })
            .and_then(|v| v.parse::<usize>().ok())
            .expect("content-length header");

        let mut body_bytes = request.as_bytes()[body_start..].to_vec();
        while body_bytes.len() < content_length {
            let read = socket.read(&mut chunk).await.expect("read request body");
            if read == 0 {
                break;
            }
            body_bytes.extend_from_slice(&chunk[..read]);
        }

        let body: Value =
            serde_json::from_slice(&body_bytes[..content_length]).expect("body should be json");
        tx.send(body).expect("send captured body");

        let response_body = r#"{"model":"gpt-4","choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        let _ = socket.shutdown().await;
    });

    Some((format!("http://{addr}/v1/"), rx, handle))
}

fn assert_parameters_sanitized(parameters: &Value) {
    let pretty = serde_json::to_string(parameters).expect("parameters should serialize");
    assert_eq!(parameters["type"], "object");
    assert!(parameters["properties"].is_object());
    assert!(
        !pretty.contains("\"$defs\""),
        "parameters should not contain $defs: {pretty}"
    );
    assert!(
        !pretty.contains("\"$ref\""),
        "parameters should not contain $ref: {pretty}"
    );
    assert!(
        !pretty.contains("\"anyOf\""),
        "parameters should not contain anyOf: {pretty}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn test_openai_compatible_payload_sanitizes_final_tool_parameters() {
    let Some((base_url, body_rx, _server)) = start_capture_server().await else {
        eprintln!("skipping test: sandbox does not permit local TCP listeners");
        return;
    };

    let client = genai::Client::builder()
        .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
            t.endpoint = genai::resolver::Endpoint::from_owned(base_url.clone());
            t.auth = genai::resolver::AuthData::from_single("test-key");
            Ok(t)
        })
        .build();

    let chat_req = ChatRequest::new(vec![ChatMessage::user("hi")]).with_tools(vec![
        Tool::new("empty_object"),
        Tool::new("refs_tool").with_schema(json!({
            "$defs": {
                "Payload": {
                    "type": "object",
                    "properties": {
                        "primaryKey": { "type": "string" },
                        "basePath": { "type": "string" }
                    }
                }
            },
            "type": "object",
            "properties": {
                "payload": { "$ref": "#/$defs/Payload" }
            }
        })),
        Tool::new("union_tool").with_schema(json!({
            "type": "object",
            "properties": {
                "selection": {
                    "anyOf": [
                        {
                            "type": "object",
                            "properties": {
                                "kind": { "type": "string" },
                                "items": { "type": "array" }
                            }
                        },
                        { "type": "null" }
                    ]
                }
            }
        })),
    ]);

    client
        .exec_chat("gpt-4", chat_req, None)
        .await
        .expect("chat request should succeed");

    let body = body_rx.await.expect("captured request body");
    let tools = body["tools"].as_array().expect("tools should be array");
    assert_eq!(tools.len(), 3);

    let empty_parameters = &tools[0]["function"]["parameters"];
    assert_parameters_sanitized(empty_parameters);

    let refs_parameters = &tools[1]["function"]["parameters"];
    assert_parameters_sanitized(refs_parameters);
    assert!(refs_parameters["properties"]["payload"]["properties"].is_object());

    let union_parameters = &tools[2]["function"]["parameters"];
    assert_parameters_sanitized(union_parameters);
    assert!(union_parameters["properties"]["selection"]["properties"].is_object());
    assert!(
        union_parameters["properties"]["selection"]["properties"]["items"]["items"].is_object()
    );
}
