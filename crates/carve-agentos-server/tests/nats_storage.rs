//! Integration tests for NatsBufferedStorage using testcontainers.
//!
//! Requires Docker. Run with:
//! ```bash
//! cargo test --package carve-agentos-server --test nats_storage -- --nocapture
//! ```

use carve_agent::{
    CheckpointReason, MemoryStorage, Message, Thread, ThreadDelta, ThreadStore,
};
use carve_agentos_server::nats_storage::NatsBufferedStorage;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::nats::Nats;

async fn start_nats_js() -> (testcontainers::ContainerAsync<Nats>, String) {
    let container = Nats::default()
        .with_cmd(["-js"]) // Enable JetStream
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

async fn make_storage(nats_url: &str) -> (Arc<MemoryStorage>, NatsBufferedStorage) {
    let inner = Arc::new(MemoryStorage::new());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let js = async_nats::jetstream::new(nats_client);
    let storage = NatsBufferedStorage::new(inner.clone(), js).await.unwrap();
    (inner, storage)
}

#[tokio::test]
async fn test_create_delegates_to_inner() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    let thread = Thread::new("t1");
    storage.create(&thread).await.unwrap();

    let loaded = inner.load("t1").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().thread.id, "t1");
}

#[tokio::test]
async fn test_append_does_not_write_to_inner() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    let thread = Thread::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    let delta = ThreadDelta {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("world"))],
        patches: vec![],
        snapshot: None,
    };

    // append() publishes to NATS, not to inner storage
    storage.append("t1", &delta).await.unwrap();

    // Inner should still have only the original message
    let loaded = inner.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.thread.messages.len(), 1);
    assert_eq!(loaded.thread.messages[0].content, "hello");
}

#[tokio::test]
async fn test_save_flushes_to_inner_and_purges_nats() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    let thread = Thread::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    let delta = ThreadDelta {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("world"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta).await.unwrap();

    // Build final thread with both messages
    let final_thread = Thread::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("world"));

    // save() should write to inner storage
    storage.save(&final_thread).await.unwrap();

    let loaded = inner.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.thread.messages.len(), 2);
    assert_eq!(loaded.thread.messages[0].content, "hello");
    assert_eq!(loaded.thread.messages[1].content, "world");
}

#[tokio::test]
async fn test_load_delegates_to_inner() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    assert!(storage.load("nonexistent").await.unwrap().is_none());

    let thread = Thread::new("t1");
    inner.create(&thread).await.unwrap();

    let loaded = storage.load("t1").await.unwrap();
    assert!(loaded.is_some());
}

#[tokio::test]
async fn test_delete_delegates_to_inner() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    let thread = Thread::new("t1");
    inner.create(&thread).await.unwrap();

    storage.delete("t1").await.unwrap();

    assert!(inner.load("t1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_recover_replays_unacked_deltas() {
    let (_container, url) = start_nats_js().await;
    let (inner, storage) = make_storage(&url).await;

    // Create thread in inner storage
    let thread = Thread::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    // Publish deltas via append (these go to NATS)
    let delta1 = ThreadDelta {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("response 1"))],
        patches: vec![],
        snapshot: None,
    };
    let delta2 = ThreadDelta {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![Arc::new(Message::assistant("response 2"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta1).await.unwrap();
    storage.append("t1", &delta2).await.unwrap();

    // Simulate crash — don't call save(), just recover
    let recovered = storage.recover().await.unwrap();

    assert_eq!(recovered, 2);

    // Inner storage should now have all messages
    let loaded = inner.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.thread.messages.len(), 3); // hello + response 1 + response 2
}
