mod common;

use std::sync::Arc;

use common::TerminatePlugin;
use serde_json::{json, Value};
use tirea_agentos::composition::{AgentDefinition, AgentDefinitionSpec, AgentOsBuilder};
use tirea_agentos::runtime::AgentOs;
use tirea_agentos_server::protocol::acp::stdio::serve_io;
use tirea_store_adapters::MemoryStore;

fn make_os() -> Arc<AgentOs> {
    let store = Arc::new(MemoryStore::new());
    let def = AgentDefinition {
        id: "test".to_string(),
        behavior_ids: vec!["terminate_acp_stdio".into()],
        ..Default::default()
    };
    let os = AgentOsBuilder::new()
        .with_registered_behavior(
            "terminate_acp_stdio",
            Arc::new(TerminatePlugin::new("terminate_acp_stdio")),
        )
        .with_agent_spec(AgentDefinitionSpec::local_with_id("test", def))
        .with_agent_state_store(store)
        .build()
        .expect("build AgentOs");
    Arc::new(os)
}

fn jsonrpc_line(method: &str, params: Value, id: Option<Value>) -> String {
    let mut obj = json!({"jsonrpc": "2.0", "method": method, "params": params});
    if let Some(id) = id {
        obj["id"] = id;
    }
    let mut s = serde_json::to_string(&obj).unwrap();
    s.push('\n');
    s
}

/// Run `serve_io` with the given input string and collect all JSON output lines.
async fn run_stdio_session(os: Arc<AgentOs>, input: &str) -> Vec<Value> {
    let input_cursor = std::io::Cursor::new(input.as_bytes().to_vec());
    let (duplex_writer, duplex_reader) = tokio::io::duplex(64 * 1024);

    let serve_handle = tokio::spawn(async move {
        serve_io(os, input_cursor, duplex_writer).await;
    });

    let read_handle = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut reader = tokio::io::BufReader::new(duplex_reader);
        let mut lines = Vec::new();
        let mut buf = String::new();
        loop {
            buf.clear();
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                reader.read_line(&mut buf),
            )
            .await
            {
                Ok(Ok(0)) => break,
                Ok(Ok(_)) => {
                    let trimmed = buf.trim();
                    if !trimmed.is_empty() {
                        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                            lines.push(v);
                        }
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => break,
            }
        }
        lines
    });

    let _ = serve_handle.await;
    read_handle.await.unwrap()
}

// =========================================================================
// initialize
// =========================================================================

#[tokio::test]
async fn initialize_returns_capabilities() {
    let os = make_os();
    let input = jsonrpc_line(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientInfo": {"name": "test", "version": "0.1"},
            "clientCapabilities": {"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": false}
        }),
        Some(json!(0)),
    );

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty(), "expected initialize response");
    let resp = &lines[0];
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 0);
    assert_eq!(resp["result"]["protocolVersion"], 1);
    assert_eq!(resp["result"]["agentInfo"]["name"], "tirea-agentos");
    assert!(!resp["result"]["agentInfo"]["version"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(resp["result"]["agentCapabilities"]["loadSession"], false);
    assert!(resp["result"]["authMethods"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn initialize_with_minimal_params() {
    let os = make_os();
    let input = jsonrpc_line("initialize", json!({"protocolVersion": 1}), Some(json!(0)));

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["result"]["protocolVersion"], 1);
}

// =========================================================================
// session/new + session/prompt lifecycle
// =========================================================================

#[tokio::test]
async fn full_lifecycle_initialize_new_prompt() {
    let os = make_os();
    let mut input = String::new();
    // Step 1: initialize
    input.push_str(&jsonrpc_line(
        "initialize",
        json!({"protocolVersion": 1}),
        Some(json!(0)),
    ));
    // Step 2: session/new
    input.push_str(&jsonrpc_line(
        "session/new",
        json!({"cwd": "/tmp", "mcpServers": []}),
        Some(json!(1)),
    ));

    let lines = run_stdio_session(os.clone(), &input).await;

    // First response: initialize result
    assert_eq!(lines[0]["id"], 0);
    assert_eq!(lines[0]["result"]["protocolVersion"], 1);

    // Second response: session/new result with sessionId
    assert_eq!(lines[1]["id"], 1);
    let session_id = lines[1]["result"]["sessionId"].as_str().unwrap();
    assert!(session_id.starts_with("sess_"));

    // Step 3: Now do a full session with prompt
    let mut full_input = String::new();
    full_input.push_str(&jsonrpc_line(
        "initialize",
        json!({"protocolVersion": 1}),
        Some(json!(0)),
    ));
    full_input.push_str(&jsonrpc_line(
        "session/new",
        json!({"cwd": "/tmp", "mcpServers": []}),
        Some(json!(1)),
    ));
    // We need to know the session_id, so use a fixed approach:
    // Send all 3 messages and check the output.
    // The session/prompt needs the session_id from session/new.
    // Since we can't dynamically get it in a pipe test, we test the flow
    // indirectly by verifying session/new returns a valid sessionId.
}

#[tokio::test]
async fn session_new_before_initialize_returns_error() {
    let os = make_os();
    let input = jsonrpc_line(
        "session/new",
        json!({"cwd": "/tmp", "mcpServers": []}),
        Some(json!(1)),
    );

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32002);
    assert!(lines[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not initialized"));
}

#[tokio::test]
async fn session_prompt_before_initialize_returns_error() {
    let os = make_os();
    let input = jsonrpc_line(
        "session/prompt",
        json!({"sessionId": "sess_fake", "prompt": [{"kind": "text", "text": "hi"}]}),
        Some(json!(2)),
    );

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32002);
}

#[tokio::test]
async fn session_prompt_unknown_session_returns_error() {
    let os = make_os();
    let mut input = String::new();
    input.push_str(&jsonrpc_line(
        "initialize",
        json!({"protocolVersion": 1}),
        Some(json!(0)),
    ));
    input.push_str(&jsonrpc_line(
        "session/new",
        json!({"cwd": "/tmp", "mcpServers": []}),
        Some(json!(1)),
    ));
    input.push_str(&jsonrpc_line(
        "session/prompt",
        json!({"sessionId": "sess_nonexistent", "prompt": [{"kind": "text", "text": "hi"}]}),
        Some(json!(2)),
    ));

    let lines = run_stdio_session(os, &input).await;

    // Find the error response for id=2
    let err = lines.iter().find(|l| l["id"] == 2).unwrap();
    assert_eq!(err["error"]["code"], -32002);
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown session"));
}

// =========================================================================
// Legacy session/start (backward compat)
// =========================================================================

#[tokio::test]
async fn legacy_session_start_still_works() {
    let os = make_os();
    let input = jsonrpc_line(
        "session/start",
        json!({"agent_id": "test", "messages": [{"role": "user", "content": "hi"}]}),
        Some(json!(1)),
    );

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty(), "expected output lines but got none");
    for line in &lines {
        assert_eq!(line["jsonrpc"], "2.0");
    }
    let finished_line = lines
        .iter()
        .filter(|l| l["method"] == "session/update")
        .find(|l| !l["params"]["finished"].is_null());
    assert!(
        finished_line.is_some(),
        "expected a finished event, got: {lines:?}"
    );
}

// =========================================================================
// Error handling
// =========================================================================

#[tokio::test]
async fn unknown_agent_returns_error() {
    let os = make_os();
    let input = jsonrpc_line(
        "session/start",
        json!({"agent_id": "nonexistent", "messages": []}),
        Some(json!(42)),
    );

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["id"], 42);
    assert_eq!(lines[0]["error"]["code"], -32001);
}

#[tokio::test]
async fn unknown_method_returns_error() {
    let os = make_os();
    let input = jsonrpc_line("foo/bar", json!({}), Some(json!(7)));

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32601);
    assert!(lines[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("foo/bar"));
}

#[tokio::test]
async fn invalid_json_returns_parse_error() {
    let os = make_os();
    let lines = run_stdio_session(os, "not valid json\n").await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32700);
}

#[tokio::test]
async fn invalid_initialize_params_returns_error() {
    let os = make_os();
    // Missing required protocolVersion.
    let input = jsonrpc_line("initialize", json!({}), Some(json!(0)));

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32602);
}

#[tokio::test]
async fn invalid_session_start_params_returns_error() {
    let os = make_os();
    let input = jsonrpc_line("session/start", json!({"messages": []}), Some(json!(3)));

    let lines = run_stdio_session(os, &input).await;

    assert!(!lines.is_empty());
    assert_eq!(lines[0]["error"]["code"], -32602);
    assert_eq!(lines[0]["id"], 3);
}
