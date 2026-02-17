//! Integration tests for NatsBufferedThreadWriter using testcontainers.
//!
//! Requires Docker. Run with:
//! ```bash
//! cargo test --package carve-thread-store-adapters --features nats --test nats_buffered -- --nocapture
//! ```

#![cfg(feature = "nats")]

use carve_agent_contract::change::AgentChangeSet;
use carve_agent_contract::{
    AgentState, AgentStateReader, AgentStateWriter, CheckpointReason, Message, MessageQuery,
};
use carve_thread_store_adapters::{MemoryStore, NatsBufferedThreadWriter};
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use testcontainers_modules::nats::Nats;

async fn start_nats_js() -> Option<(testcontainers::ContainerAsync<Nats>, String)> {
    let container = match Nats::default().with_cmd(["-js"]).start().await {
        Ok(container) => container,
        Err(err) => {
            eprintln!("skipping nats_buffered test: unable to start NATS container ({err})");
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

async fn make_storage(nats_url: &str) -> (Arc<MemoryStore>, NatsBufferedThreadWriter) {
    let inner = Arc::new(MemoryStore::new());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let js = async_nats::jetstream::new(nats_client);
    let storage = NatsBufferedThreadWriter::new(inner.clone(), js)
        .await
        .unwrap();
    (inner, storage)
}

#[tokio::test]
async fn test_create_delegates_to_inner() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    let thread = AgentState::new("t1");
    storage.create(&thread).await.unwrap();

    let loaded = inner.load("t1").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().agent_state.id, "t1");
}

#[tokio::test]
async fn test_append_does_not_write_to_inner() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    let thread = AgentState::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    let delta = AgentChangeSet {
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
    assert_eq!(loaded.agent_state.messages.len(), 1);
    assert_eq!(loaded.agent_state.messages[0].content, "hello");
}

#[tokio::test]
async fn test_save_flushes_to_inner_and_purges_nats() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    let thread = AgentState::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    let delta = AgentChangeSet {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("world"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta).await.unwrap();

    // Build final thread with both messages
    let final_thread = AgentState::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("world"));

    // save() should write to inner storage
    storage.save(&final_thread).await.unwrap();

    let loaded = inner.load("t1").await.unwrap().unwrap();
    assert_eq!(loaded.agent_state.messages.len(), 2);
    assert_eq!(loaded.agent_state.messages[0].content, "hello");
    assert_eq!(loaded.agent_state.messages[1].content, "world");
}

#[tokio::test]
async fn test_load_delegates_to_inner() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    assert!(storage.load("nonexistent").await.unwrap().is_none());

    let thread = AgentState::new("t1");
    inner.create(&thread).await.unwrap();

    let loaded = storage.load("t1").await.unwrap();
    assert!(loaded.is_some());
}

#[tokio::test]
async fn test_delete_delegates_to_inner() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    let thread = AgentState::new("t1");
    inner.create(&thread).await.unwrap();

    storage.delete("t1").await.unwrap();

    assert!(inner.load("t1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_recover_replays_unacked_deltas() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    // Create thread in inner storage
    let thread = AgentState::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    // Publish deltas via append (these go to NATS)
    let delta1 = AgentChangeSet {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("response 1"))],
        patches: vec![],
        snapshot: None,
    };
    let delta2 = AgentChangeSet {
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
    assert_eq!(loaded.agent_state.messages.len(), 3); // hello + response 1 + response 2
}

// ============================================================================
// CQRS consistency tests — queries read from last-flushed snapshot
// ============================================================================

/// During an active run, load() returns the pre-run snapshot without any
/// buffered deltas.  This is the designed CQRS behaviour: real-time data
/// is delivered through the SSE event stream, queries read durable storage.
#[tokio::test]
async fn test_query_returns_last_flush_snapshot_during_active_run() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    // Simulate a completed first run: thread with 1 user + 1 assistant msg.
    let thread = AgentState::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("first reply"));
    inner.create(&thread).await.unwrap();

    // Second run starts — new deltas go to NATS only.
    let delta = AgentChangeSet {
        run_id: "r2".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("second reply"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta).await.unwrap();

    // Query via NatsBufferedThreadWriter.load() — should see first-run snapshot.
    let head = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(
        head.agent_state.messages.len(),
        2,
        "load() should return the pre-run snapshot (2 messages), not include buffered delta"
    );
    assert_eq!(head.agent_state.messages[0].content, "hello");
    assert_eq!(head.agent_state.messages[1].content, "first reply");
}

/// load_messages() (AgentStateReader default) also reads from the inner storage,
/// so during a run it returns the last-flushed message list.
#[tokio::test]
async fn test_load_messages_returns_last_flush_snapshot_during_active_run() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    let thread = AgentState::new("t1")
        .with_message(Message::user("msg-0"))
        .with_message(Message::assistant("msg-1"));
    inner.create(&thread).await.unwrap();

    // Buffer 2 new deltas via NATS.
    for i in 2..4 {
        let delta = AgentChangeSet {
            run_id: "r2".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::AssistantTurnCommitted,
            messages: vec![Arc::new(Message::assistant(format!("msg-{i}")))],
            patches: vec![],
            snapshot: None,
        };
        storage.append("t1", &delta).await.unwrap();
    }

    // Query messages through the inner storage (which NatsBufferedThreadWriter delegates to).
    let page = AgentStateReader::load_messages(inner.as_ref(), "t1", &MessageQuery::default())
        .await
        .unwrap();
    assert_eq!(
        page.messages.len(),
        2,
        "load_messages() should return 2 messages from last flush, not 4"
    );
}

/// After save() completes, queries immediately see the flushed data.
#[tokio::test]
async fn test_query_accurate_after_run_end_flush() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    // First run: create thread with 1 message.
    let thread = AgentState::new("t1").with_message(Message::user("hello"));
    inner.create(&thread).await.unwrap();

    // Run produces deltas buffered in NATS.
    let delta = AgentChangeSet {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("world"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta).await.unwrap();

    // Before flush: load sees 1 message.
    let pre = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(pre.agent_state.messages.len(), 1);

    // Run-end flush.
    let final_thread = AgentState::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("world"));
    storage.save(&final_thread).await.unwrap();

    // After flush: load sees 2 messages.
    let post = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(post.agent_state.messages.len(), 2);
    assert_eq!(post.agent_state.messages[1].content, "world");

    // load_messages also sees 2.
    let page = AgentStateReader::load_messages(inner.as_ref(), "t1", &MessageQuery::default())
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 2);
}

/// Across two sequential runs: during the second run, queries return the
/// first run's fully flushed state.
#[tokio::test]
async fn test_multi_run_query_sees_previous_run_data() {
    let Some((_container, url)) = start_nats_js().await else {
        return;
    };
    let (inner, storage) = make_storage(&url).await;

    // === Run 1 ===
    let thread = AgentState::new("t1").with_message(Message::user("q1"));
    inner.create(&thread).await.unwrap();

    let delta1 = AgentChangeSet {
        run_id: "r1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("a1"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta1).await.unwrap();

    // Flush run 1.
    let run1_thread = AgentState::new("t1")
        .with_message(Message::user("q1"))
        .with_message(Message::assistant("a1"));
    storage.save(&run1_thread).await.unwrap();

    // === Run 2 (in progress) ===
    let delta2 = AgentChangeSet {
        run_id: "r2".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("a2"))],
        patches: vec![],
        snapshot: None,
    };
    storage.append("t1", &delta2).await.unwrap();

    // Query during run 2: sees run 1's flushed state (2 messages), not run 2's delta.
    let head = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(
        head.agent_state.messages.len(),
        2,
        "during run 2, query should see run 1's flushed state (q1 + a1)"
    );
    assert_eq!(head.agent_state.messages[0].content, "q1");
    assert_eq!(head.agent_state.messages[1].content, "a1");

    // Flush run 2.
    let run2_thread = AgentState::new("t1")
        .with_message(Message::user("q1"))
        .with_message(Message::assistant("a1"))
        .with_message(Message::assistant("a2"));
    storage.save(&run2_thread).await.unwrap();

    // Now query sees all 3 messages.
    let head = storage.load("t1").await.unwrap().unwrap();
    assert_eq!(head.agent_state.messages.len(), 3);
}
