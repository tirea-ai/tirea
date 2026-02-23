#![allow(missing_docs)]

use async_trait::async_trait;
use futures::stream;
use futures::StreamExt;
use genai::chat::{
    ChatOptions, ChatRequest, ChatResponse, ChatStreamEvent, MessageContent, StreamChunk,
    StreamEnd, ToolChunk,
};
use mcp::transport::McpServerConnectionConfig;
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tirea_agent_loop::contracts::thread::{Message, Thread};
use tirea_agent_loop::contracts::{AgentEvent, RunConfig, RunContext, ToolRegistry};
use tirea_agent_loop::runtime::loop_runner::{run_loop_stream, AgentConfig, LlmExecutor};
use tirea_contract::ProtocolOutputEncoder;
use tirea_extension_mcp::McpToolRegistryManager;
use tirea_protocol_ai_sdk_v6::{AiSdkV6ProtocolEncoder, UIStreamEvent};

#[derive(Clone)]
struct MockResponse {
    text: String,
    tool_calls: Vec<genai::chat::ToolCall>,
}

impl MockResponse {
    fn text(text: &str) -> Self {
        Self {
            text: text.to_string(),
            tool_calls: Vec::new(),
        }
    }

    fn tool_call(call_id: &str, name: &str, args: Value) -> Self {
        Self {
            text: String::new(),
            tool_calls: vec![genai::chat::ToolCall {
                call_id: call_id.to_string(),
                fn_name: name.to_string(),
                fn_arguments: Value::String(args.to_string()),
                thought_signatures: None,
            }],
        }
    }
}

struct MockStreamProvider {
    responses: Mutex<Vec<MockResponse>>,
}

impl MockStreamProvider {
    fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl LlmExecutor for MockStreamProvider {
    async fn exec_chat_response(
        &self,
        _model: &str,
        _chat_req: ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<ChatResponse> {
        unimplemented!("stream-only provider")
    }

    async fn exec_chat_stream_events(
        &self,
        _model: &str,
        _chat_req: ChatRequest,
        _options: Option<&ChatOptions>,
    ) -> genai::Result<tirea_agent_loop::contracts::runtime::LlmEventStream> {
        let response = {
            let mut guard = self.responses.lock().expect("responses lock");
            if guard.is_empty() {
                MockResponse::text("done")
            } else {
                guard.remove(0)
            }
        };

        let mut events: Vec<genai::Result<ChatStreamEvent>> = Vec::new();
        events.push(Ok(ChatStreamEvent::Start));

        if !response.text.is_empty() {
            events.push(Ok(ChatStreamEvent::Chunk(StreamChunk {
                content: response.text.clone(),
            })));
        }

        for call in &response.tool_calls {
            events.push(Ok(ChatStreamEvent::ToolCallChunk(ToolChunk {
                tool_call: call.clone(),
            })));
        }

        let end = StreamEnd {
            captured_content: if response.tool_calls.is_empty() {
                None
            } else {
                Some(MessageContent::from_tool_calls(response.tool_calls))
            },
            ..Default::default()
        };
        events.push(Ok(ChatStreamEvent::End(end)));

        Ok(Box::pin(stream::iter(events)))
    }

    fn name(&self) -> &'static str {
        "mock_stream_provider"
    }
}

async fn collect_agent_events(
    stream: Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>>,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    let mut stream = stream;
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

#[tokio::test(flavor = "multi_thread")]
async fn real_mcp_progress_notifications_flow_to_ui_data_events() {
    let server_script = r#"
import json
import sys
import time

def send(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    message = json.loads(raw)
    method = message.get("method")
    msg_id = message.get("id")

    if method == "initialize":
        send({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "serverInfo": {"name": "progress-server", "version": "0.1.0"},
                "capabilities": {}
            }
        })
    elif method == "tools/list":
        send({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "tools": [{
                    "name": "echo_progress",
                    "title": "Echo Progress",
                    "description": "Echo text with progress notifications",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "message": {"type": "string"}
                        },
                        "required": ["message"]
                    }
                }]
            }
        })
    elif method == "tools/call":
        params = message.get("params") or {}
        meta = params.get("_meta") or {}
        token = meta.get("progressToken")
        if token is not None:
            send({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": token,
                    "progress": 1.0,
                    "total": 4.0
                }
            })
            time.sleep(0.01)
            send({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {
                    "progressToken": token,
                    "progress": 4.0,
                    "total": 4.0
                }
            })

        arguments = params.get("arguments") or {}
        text = arguments.get("message") or "ok"
        send({
            "jsonrpc": "2.0",
            "id": msg_id,
            "result": {
                "content": [{"type": "text", "text": text}]
            }
        })
    else:
        if msg_id is not None:
            send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
"#;

    let cfg = McpServerConnectionConfig::stdio(
        "progress_server",
        "python3",
        vec![
            "-u".to_string(),
            "-c".to_string(),
            server_script.to_string(),
        ],
    );

    let manager = McpToolRegistryManager::connect([cfg])
        .await
        .expect("connect MCP server");
    let registry = manager.registry();
    let tool_id = registry
        .ids()
        .into_iter()
        .find(|id| id.ends_with("__echo_progress"))
        .expect("discover echo_progress tool");

    let llm = MockStreamProvider::new(vec![
        MockResponse::tool_call("call_progress", &tool_id, json!({ "message": "hello" })),
        MockResponse::text("done"),
    ]);
    let config = AgentConfig::new("mock").with_llm_executor(Arc::new(llm) as Arc<dyn LlmExecutor>);

    let thread = Thread::new("thread-mcp-progress").with_message(Message::user("run"));
    let run_ctx = RunContext::from_thread(&thread, RunConfig::default()).expect("run context");
    let agent_events = collect_agent_events(run_loop_stream(
        config,
        registry.snapshot(),
        run_ctx,
        None,
        None,
        None,
    ))
    .await;

    assert!(agent_events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                ..
            } if message_id == "tool_call:call_progress"
                && activity_type == "progress"
                && content["progress"] == json!(0.25)
        )
    }));
    assert!(agent_events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ActivitySnapshot {
                message_id,
                activity_type,
                content,
                ..
            } if message_id == "tool_call:call_progress"
                && activity_type == "progress"
                && content["progress"] == json!(1.0)
        )
    }));

    let mut encoder = AiSdkV6ProtocolEncoder::new(
        "run_progress_e2e".to_string(),
        Some("thread-mcp-progress".to_string()),
    );
    let mut ui_events = encoder.prologue();
    for event in &agent_events {
        ui_events.extend(encoder.on_agent_event(event));
    }

    assert!(ui_events.iter().any(|event| {
        matches!(
            event,
            UIStreamEvent::Data { data_type, data, .. }
                if data_type == "data-activity-snapshot"
                    && data["messageId"] == json!("tool_call:call_progress")
                    && data["activityType"] == json!("progress")
                    && data["content"]["progress"] == json!(0.25)
        )
    }));
    assert!(ui_events.iter().any(|event| {
        matches!(
            event,
            UIStreamEvent::Data { data_type, data, .. }
                if data_type == "data-activity-snapshot"
                    && data["messageId"] == json!("tool_call:call_progress")
                    && data["activityType"] == json!("progress")
                    && data["content"]["progress"] == json!(1.0)
        )
    }));
}
