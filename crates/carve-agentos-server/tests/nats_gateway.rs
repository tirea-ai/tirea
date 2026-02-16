//! Integration tests for the NATS gateway using testcontainers.
//!
//! These tests spin up a real NATS server in Docker and verify that
//! `NatsGateway::serve()` correctly handles AG-UI and AI SDK requests
//! published over NATS.
//!
//! Requires Docker to be running. Run with:
//! ```bash
//! cargo test --package carve-agentos-server --test nats_gateway -- --nocapture
//! ```

use async_trait::async_trait;
use carve_agent::contracts::agent_plugin::AgentPlugin;
use carve_agent::contracts::phase::Phase;
use carve_agent::contracts::phase::StepContext;
use carve_agent::contracts::storage::{ThreadReader, ThreadStore};
use carve_agent::orchestrator::AgentOsBuilder;
use carve_agent::runtime::loop_runner::AgentDefinition;
use carve_agentos_server::nats::NatsGateway;
use carve_thread_store_adapters::MemoryStore;
use futures::StreamExt;
use serde_json::json;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::nats::Nats;

struct SkipInferencePlugin;

#[async_trait]
impl AgentPlugin for SkipInferencePlugin {
    fn id(&self) -> &str {
        "skip_inference_test"
    }

    async fn on_phase(
        &self,
        phase: Phase,
        step: &mut StepContext<'_>,
        _ctx: &carve_agent::prelude::Context<'_>,
    ) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os(storage: Arc<dyn ThreadStore>) -> carve_agent::orchestrator::AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .with_thread_store(storage)
        .build()
        .unwrap()
}

async fn start_nats() -> Option<(testcontainers::ContainerAsync<Nats>, String)> {
    let container = match Nats::default().start().await {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping nats_gateway test: unable to start NATS container ({err})");
            return None;
        }
    };
    let host = container.get_host().await.expect("failed to get host");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("failed to get port");
    let url = format!("{host}:{port}");
    Some((container, url))
}

/// Spawn the gateway and return the NATS client for publishing test requests.
async fn setup_gateway(nats_url: &str) -> (Arc<MemoryStore>, async_nats::Client) {
    let storage = Arc::new(MemoryStore::new());
    let os = Arc::new(make_os(storage.clone()));

    let gateway = NatsGateway::connect(os, nats_url)
        .await
        .expect("failed to connect gateway to NATS");

    // Spawn serve in background — it loops until subscriptions close.
    tokio::spawn(async move {
        let _ = gateway.serve().await;
    });

    // Give the gateway a moment to subscribe.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let client = async_nats::connect(nats_url)
        .await
        .expect("failed to connect test client to NATS");

    (storage, client)
}

// ============================================================================
// AG-UI over NATS — happy path
// ============================================================================

#[tokio::test]
async fn test_nats_agui_happy_path() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (storage, client) = setup_gateway(&nats_url).await;

    let reply_subject = "test.reply.agui.1";
    let mut sub = client.subscribe(reply_subject).await.unwrap();

    let payload = json!({
        "agentId": "test",
        "replySubject": reply_subject,
        "request": {
            "threadId": "nats-agui-1",
            "runId": "r1",
            "messages": [
                {"role": "user", "content": "hello via nats"}
            ],
            "tools": []
        }
    });

    client
        .publish(
            "agentos.run.agui",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    // Collect reply events with a timeout.
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::select! {
            msg = sub.next() => {
                match msg {
                    Some(m) => {
                        let text = String::from_utf8_lossy(&m.payload).to_string();
                        events.push(text);
                        // RUN_FINISHED signals end.
                        if events.last().is_some_and(|e| e.contains("RUN_FINISHED")) {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                panic!("timed out waiting for NATS agui reply events; got {} so far: {:?}", events.len(), events);
            }
        }
    }

    let all = events.join("\n");
    assert!(all.contains("RUN_STARTED"), "missing RUN_STARTED in: {all}");
    assert!(
        all.contains("RUN_FINISHED"),
        "missing RUN_FINISHED in: {all}"
    );

    // Wait for checkpoint persistence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let saved = storage.load_thread("nats-agui-1").await.unwrap();
    assert!(saved.is_some(), "thread not persisted");
    let saved = saved.unwrap();
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.content.contains("hello via nats")),
        "user message not found in persisted thread"
    );
}

// ============================================================================
// AI SDK over NATS — happy path
// ============================================================================

#[tokio::test]
async fn test_nats_aisdk_happy_path() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (storage, client) = setup_gateway(&nats_url).await;

    let reply_subject = "test.reply.aisdk.1";
    let mut sub = client.subscribe(reply_subject).await.unwrap();

    let payload = json!({
        "agentId": "test",
        "sessionId": "nats-sdk-1",
        "input": "hi from nats sdk",
        "runId": "r1",
        "replySubject": reply_subject
    });

    client
        .publish(
            "agentos.run.aisdk",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    // Collect reply events.
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::select! {
            msg = sub.next() => {
                match msg {
                    Some(m) => {
                        let text = String::from_utf8_lossy(&m.payload).to_string();
                        events.push(text);
                        if events.last().is_some_and(|e| e.contains("\"type\":\"finish\"")) {
                            break;
                        }
                    }
                    None => break,
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                panic!("timed out waiting for NATS aisdk reply events; got {} so far: {:?}", events.len(), events);
            }
        }
    }

    let all = events.join("\n");
    assert!(
        all.contains("\"type\":\"start\""),
        "missing start in: {all}"
    );
    // text-start/text-end are lazy — only emitted when TextDelta events occur.
    // This test skips inference, so no text is produced.
    assert!(
        all.contains("\"type\":\"finish\""),
        "missing finish in: {all}"
    );

    // Wait for checkpoint persistence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let saved = storage.load_thread("nats-sdk-1").await.unwrap();
    assert!(saved.is_some(), "thread not persisted");
    let saved = saved.unwrap();
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.content.contains("hi from nats sdk")),
        "user message not found in persisted thread"
    );
}

// ============================================================================
// Error paths
// ============================================================================

#[tokio::test]
async fn test_nats_agui_agent_not_found() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    let reply_subject = "test.reply.agui.err.1";
    let mut sub = client.subscribe(reply_subject).await.unwrap();

    let payload = json!({
        "agentId": "no_such_agent",
        "replySubject": reply_subject,
        "request": {
            "threadId": "t1",
            "runId": "r1",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": []
        }
    });

    client
        .publish(
            "agentos.run.agui",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    tokio::select! {
        msg = sub.next() => {
            let m = msg.expect("expected a reply");
            let text = String::from_utf8_lossy(&m.payload).to_string();
            assert!(
                text.contains("RUN_ERROR") || text.contains("not found"),
                "expected error reply for unknown agent: {text}"
            );
        }
        _ = tokio::time::sleep_until(deadline) => {
            panic!("timed out waiting for error reply");
        }
    }
}

#[tokio::test]
async fn test_nats_aisdk_agent_not_found() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (storage, client) = setup_gateway(&nats_url).await;

    let reply_subject = "test.reply.aisdk.err.1";
    let mut sub = client.subscribe(reply_subject).await.unwrap();

    let payload = json!({
        "agentId": "no_such_agent",
        "sessionId": "s1",
        "input": "hi",
        "replySubject": reply_subject
    });

    client
        .publish(
            "agentos.run.aisdk",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    tokio::select! {
        msg = sub.next() => {
            let m = msg.expect("expected a reply");
            let text = String::from_utf8_lossy(&m.payload).to_string();
            assert!(
                text.contains("error") || text.contains("not found"),
                "expected error reply for unknown agent: {text}"
            );
        }
        _ = tokio::time::sleep_until(deadline) => {
            panic!("timed out waiting for error reply");
        }
    }

    // Unknown agent should fail fast without persisting user input.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let saved = storage.load_thread("s1").await.unwrap();
    assert!(
        saved.is_none(),
        "thread should not be persisted when agent is missing"
    );
}

#[tokio::test]
async fn test_nats_agui_bad_json() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    // Bad JSON — handler should return Err (logged), no reply expected.
    client
        .publish("agentos.run.agui", bytes::Bytes::from_static(b"not json"))
        .await
        .unwrap();

    // Give it a moment to process; no panic = success.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_nats_aisdk_bad_json() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    client
        .publish("agentos.run.aisdk", bytes::Bytes::from_static(b"{bad"))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_nats_aisdk_empty_input() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    // Valid JSON but empty input — should error out (no reply since no reply subject).
    let payload = json!({
        "agentId": "test",
        "sessionId": "s1",
        "input": "  "
    });

    client
        .publish(
            "agentos.run.aisdk",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_nats_agui_missing_reply_subject() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    // No reply subject in payload or NATS message header.
    let payload = json!({
        "agentId": "test",
        "request": {
            "threadId": "t1",
            "runId": "r1",
            "messages": [],
            "tools": []
        }
    });

    client
        .publish(
            "agentos.run.agui",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    // Handler should error with "missing reply subject", no panic.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_nats_aisdk_missing_reply_subject() {
    let Some((_container, nats_url)) = start_nats().await else {
        return;
    };
    let (_storage, client) = setup_gateway(&nats_url).await;

    let payload = json!({
        "agentId": "test",
        "sessionId": "s1",
        "input": "hi"
    });

    client
        .publish(
            "agentos.run.aisdk",
            serde_json::to_vec(&payload).unwrap().into(),
        )
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}
