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
use carve_agent::phase::Phase;
use carve_agent::plugin::AgentPlugin;
use carve_agent::{AgentDefinition, AgentOsBuilder, MemoryStorage, StepContext, Storage};
use carve_agentos_server::nats::NatsGateway;
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

    async fn on_phase(&self, phase: Phase, step: &mut StepContext<'_>) {
        if phase == Phase::BeforeInference {
            step.skip_inference = true;
        }
    }
}

fn make_os() -> carve_agent::AgentOs {
    let def = AgentDefinition {
        id: "test".to_string(),
        plugins: vec![Arc::new(SkipInferencePlugin)],
        ..Default::default()
    };

    AgentOsBuilder::new()
        .with_agent("test", def)
        .build()
        .unwrap()
}

async fn start_nats() -> (testcontainers::ContainerAsync<Nats>, String) {
    let container = Nats::default()
        .start()
        .await
        .expect("failed to start NATS container");
    let host = container.get_host().await.expect("failed to get host");
    let port = container
        .get_host_port_ipv4(4222)
        .await
        .expect("failed to get port");
    let url = format!("{host}:{port}");
    (container, url)
}

/// Spawn the gateway and return the NATS client for publishing test requests.
async fn setup_gateway(nats_url: &str) -> (Arc<dyn Storage>, async_nats::Client) {
    let os = Arc::new(make_os());
    let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

    let gateway = NatsGateway::connect(os, storage.clone(), nats_url)
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
    let (_container, nats_url) = start_nats().await;
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

    let saved = storage.load("nats-agui-1").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
    let saved = saved.unwrap();
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.content.contains("hello via nats")),
        "user message not found in persisted session"
    );
}

// ============================================================================
// AI SDK over NATS — happy path
// ============================================================================

#[tokio::test]
async fn test_nats_aisdk_happy_path() {
    let (_container, nats_url) = start_nats().await;
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
    assert!(
        all.contains("\"type\":\"text-start\""),
        "missing text-start in: {all}"
    );
    assert!(
        all.contains("\"type\":\"text-end\""),
        "missing text-end in: {all}"
    );
    assert!(
        all.contains("\"type\":\"finish\""),
        "missing finish in: {all}"
    );

    // Wait for checkpoint persistence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let saved = storage.load("nats-sdk-1").await.unwrap();
    assert!(saved.is_some(), "session not persisted");
    let saved = saved.unwrap();
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.content.contains("hi from nats sdk")),
        "user message not found in persisted session"
    );
}

// ============================================================================
// Error paths
// ============================================================================

#[tokio::test]
async fn test_nats_agui_agent_not_found() {
    let (_container, nats_url) = start_nats().await;
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
    let (_container, nats_url) = start_nats().await;
    let (_storage, client) = setup_gateway(&nats_url).await;

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
}

#[tokio::test]
async fn test_nats_agui_bad_json() {
    let (_container, nats_url) = start_nats().await;
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
    let (_container, nats_url) = start_nats().await;
    let (_storage, client) = setup_gateway(&nats_url).await;

    client
        .publish("agentos.run.aisdk", bytes::Bytes::from_static(b"{bad"))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

#[tokio::test]
async fn test_nats_aisdk_empty_input() {
    let (_container, nats_url) = start_nats().await;
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
    let (_container, nats_url) = start_nats().await;
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
    let (_container, nats_url) = start_nats().await;
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
