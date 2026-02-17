use carve_agent_contract::state::AgentChangeSet;
use carve_agent_contract::storage::{
    AgentStateReader, AgentStateStore, AgentStateStoreError, AgentStateSync, AgentStateWriter,
    VersionPrecondition,
};
use carve_agent_contract::{
    AgentState, AgentStateListQuery, CheckpointReason, Message, MessageMetadata, MessageQuery, Role,
};
use carve_state::{path, Op, Patch, TrackedPatch};
use carve_thread_store_adapters::MemoryStore;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_memory_storage_save_load() {
    let storage = MemoryStore::new();
    let thread = AgentState::new("test-1").with_message(Message::user("Hello"));

    storage.save(&thread).await.unwrap();
    let loaded = storage.load_agent_state("test-1").await.unwrap();

    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "test-1");
    assert_eq!(loaded.message_count(), 1);
}

#[tokio::test]
async fn test_memory_storage_load_not_found() {
    let storage = MemoryStore::new();
    let loaded = storage.load_agent_state("nonexistent").await.unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn test_memory_storage_delete() {
    let storage = MemoryStore::new();
    let thread = AgentState::new("test-1");

    storage.save(&thread).await.unwrap();
    assert!(storage.load_agent_state("test-1").await.unwrap().is_some());

    storage.delete("test-1").await.unwrap();
    assert!(storage.load_agent_state("test-1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_memory_storage_list() {
    let storage = MemoryStore::new();

    storage.save(&AgentState::new("thread-1")).await.unwrap();
    storage.save(&AgentState::new("thread-2")).await.unwrap();

    let mut ids = storage.list().await.unwrap();
    ids.sort();

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"thread-1".to_string()));
    assert!(ids.contains(&"thread-2".to_string()));
}

#[tokio::test]
async fn test_memory_storage_update_session() {
    let storage = MemoryStore::new();

    // Save initial session
    let thread = AgentState::new("test-1").with_message(Message::user("Hello"));
    storage.save(&thread).await.unwrap();

    // Update session
    let thread = thread.with_message(Message::assistant("Hi!"));
    storage.save(&thread).await.unwrap();

    // Load and verify
    let loaded = storage.load_agent_state("test-1").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 2);
}

#[tokio::test]
async fn test_memory_storage_with_state_and_patches() {
    let storage = MemoryStore::new();

    let thread = AgentState::with_initial_state("test-1", json!({"counter": 0}))
        .with_message(Message::user("Increment"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));

    storage.save(&thread).await.unwrap();

    let loaded = storage.load_agent_state("test-1").await.unwrap().unwrap();
    assert_eq!(loaded.patch_count(), 1);
    assert_eq!(loaded.state["counter"], 0);

    // Rebuild state should apply patches
    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["counter"], 5);
}

#[tokio::test]
async fn test_memory_storage_delete_nonexistent() {
    let storage = MemoryStore::new();
    // Deleting non-existent session should not error
    storage.delete("nonexistent").await.unwrap();
}

#[tokio::test]
async fn test_memory_storage_list_empty() {
    let storage = MemoryStore::new();
    let ids = storage.list().await.unwrap();
    assert!(ids.is_empty());
}

#[tokio::test]
async fn test_memory_storage_concurrent_access() {
    use std::sync::Arc;

    let storage = Arc::new(MemoryStore::new());

    // Spawn multiple tasks
    let handles: Vec<_> = (0..10)
        .map(|i| {
            let storage = Arc::clone(&storage);
            tokio::spawn(async move {
                let thread = AgentState::new(format!("thread-{}", i));
                storage.save(&thread).await.unwrap();
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 10);
}

// ========================================================================
// Pagination tests
// ========================================================================

fn make_messages(n: usize) -> Vec<std::sync::Arc<Message>> {
    (0..n)
        .map(|i| std::sync::Arc::new(Message::user(format!("msg-{}", i))))
        .collect()
}

fn make_thread_with_messages(thread_id: &str, n: usize) -> AgentState {
    let mut thread = AgentState::new(thread_id);
    for msg in make_messages(n) {
        // Deref Arc to get Message for with_message
        thread = thread.with_message((*msg).clone());
    }
    thread
}

#[tokio::test]
async fn test_memory_storage_load_messages() {
    let storage = MemoryStore::new();
    let thread = make_thread_with_messages("test-1", 10);
    storage.save(&thread).await.unwrap();

    let query = MessageQuery {
        limit: 3,
        ..Default::default()
    };
    let page = AgentStateReader::load_messages(&storage, "test-1", &query)
        .await
        .unwrap();

    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.messages[0].message.content, "msg-0");
}

#[tokio::test]
async fn test_memory_storage_load_messages_not_found() {
    let storage = MemoryStore::new();
    let query = MessageQuery::default();
    let result = AgentStateReader::load_messages(&storage, "nonexistent", &query).await;
    assert!(matches!(result, Err(AgentStateStoreError::NotFound(_))));
}

#[tokio::test]
async fn test_memory_storage_message_count() {
    let storage = MemoryStore::new();
    let thread = make_thread_with_messages("test-1", 7);
    storage.save(&thread).await.unwrap();

    let count = storage.message_count("test-1").await.unwrap();
    assert_eq!(count, 7);
}

#[tokio::test]
async fn test_memory_storage_message_count_not_found() {
    let storage = MemoryStore::new();
    let result = storage.message_count("nonexistent").await;
    assert!(matches!(result, Err(AgentStateStoreError::NotFound(_))));
}

// ========================================================================
// Visibility tests
// ========================================================================

fn make_mixed_visibility_thread(thread_id: &str) -> AgentState {
    AgentState::new(thread_id)
        .with_message(Message::user("user-0"))
        .with_message(Message::assistant("assistant-1"))
        .with_message(Message::internal_system("reminder-2"))
        .with_message(Message::user("user-3"))
        .with_message(Message::internal_system("reminder-4"))
        .with_message(Message::assistant("assistant-5"))
}

#[tokio::test]
async fn test_memory_storage_load_messages_filters_visibility() {
    let storage = MemoryStore::new();
    let thread = make_mixed_visibility_thread("test-vis");
    storage.save(&thread).await.unwrap();

    // Default query (visibility = All)
    let page = AgentStateReader::load_messages(&storage, "test-vis", &MessageQuery::default())
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 4);

    // visibility = None (all messages)
    let query = MessageQuery {
        visibility: None,
        ..Default::default()
    };
    let page = AgentStateReader::load_messages(&storage, "test-vis", &query)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 6);
}

// ========================================================================
// Run ID filtering tests
// ========================================================================

fn make_multi_run_thread(thread_id: &str) -> AgentState {
    AgentState::new(thread_id)
        // User message (no run metadata)
        .with_message(Message::user("hello"))
        // Run A, step 0: assistant + tool
        .with_message(
            Message::assistant("thinking...").with_metadata(MessageMetadata {
                run_id: Some("run-a".to_string()),
                step_index: Some(0),
            }),
        )
        .with_message(
            Message::tool("tc1", "result").with_metadata(MessageMetadata {
                run_id: Some("run-a".to_string()),
                step_index: Some(0),
            }),
        )
        // Run A, step 1: assistant final
        .with_message(Message::assistant("done").with_metadata(MessageMetadata {
            run_id: Some("run-a".to_string()),
            step_index: Some(1),
        }))
        // User follow-up (no run metadata)
        .with_message(Message::user("more"))
        // Run B, step 0
        .with_message(Message::assistant("ok").with_metadata(MessageMetadata {
            run_id: Some("run-b".to_string()),
            step_index: Some(0),
        }))
}

#[tokio::test]
async fn test_memory_storage_load_messages_by_run_id() {
    let storage = MemoryStore::new();
    let thread = make_multi_run_thread("test-run");
    storage.save(&thread).await.unwrap();

    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = AgentStateReader::load_messages(&storage, "test-run", &query)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 3);
}

// ========================================================================
// AgentState list pagination tests
// ========================================================================

#[tokio::test]
async fn test_list_paginated_default() {
    let storage = MemoryStore::new();
    for i in 0..5 {
        storage
            .save(&AgentState::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&AgentStateListQuery::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 5);
    assert_eq!(page.total, 5);
    assert!(!page.has_more);
}

#[tokio::test]
async fn test_list_paginated_with_limit() {
    let storage = MemoryStore::new();
    for i in 0..10 {
        storage
            .save(&AgentState::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&AgentStateListQuery {
            offset: 0,
            limit: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 3);
    assert_eq!(page.total, 10);
    assert!(page.has_more);
    // Items should be sorted.
    assert_eq!(page.items, vec!["s-00", "s-01", "s-02"]);
}

#[tokio::test]
async fn test_list_paginated_with_offset() {
    let storage = MemoryStore::new();
    for i in 0..5 {
        storage
            .save(&AgentState::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&AgentStateListQuery {
            offset: 3,
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 5);
    assert!(!page.has_more);
    assert_eq!(page.items, vec!["s-03", "s-04"]);
}

#[tokio::test]
async fn test_list_paginated_offset_beyond_total() {
    let storage = MemoryStore::new();
    for i in 0..3 {
        storage
            .save(&AgentState::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&AgentStateListQuery {
            offset: 100,
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.total, 3);
    assert!(!page.has_more);
}

#[tokio::test]
async fn test_list_paginated_empty() {
    let storage = MemoryStore::new();
    let page = storage
        .list_paginated(&AgentStateListQuery::default())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.total, 0);
    assert!(!page.has_more);
}

// ========================================================================
// AgentStateWriter / AgentStateReader / AgentStateSync tests
// ========================================================================

fn sample_delta(run_id: &str, reason: CheckpointReason) -> AgentChangeSet {
    AgentChangeSet {
        run_id: run_id.to_string(),
        parent_run_id: None,
        reason,
        messages: vec![Arc::new(Message::assistant("hello"))],
        patches: vec![],
        snapshot: None,
    }
}

#[tokio::test]
async fn test_thread_store_trait_object_roundtrip() {
    use std::sync::Arc;

    let agent_state_store: Arc<dyn AgentStateStore> = Arc::new(MemoryStore::new());
    let thread = AgentState::new("trait-object-t1").with_message(Message::user("hi"));

    agent_state_store.create(&thread).await.unwrap();
    let loaded = agent_state_store
        .load_agent_state("trait-object-t1")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.id, "trait-object-t1");
    assert_eq!(loaded.message_count(), 1);
}

#[tokio::test]
async fn test_thread_store_create_and_load() {
    let store = MemoryStore::new();
    let thread = AgentState::new("t1").with_message(Message::user("hi"));
    let committed = store.create(&thread).await.unwrap();
    assert_eq!(committed.version, 0);

    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 0);
    assert_eq!(head.agent_state.id, "t1");
    assert_eq!(head.agent_state.message_count(), 1);
}

#[tokio::test]
async fn test_thread_store_create_already_exists() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();
    let err = store.create(&AgentState::new("t1")).await.unwrap_err();
    assert!(matches!(err, AgentStateStoreError::AlreadyExists));
}

#[tokio::test]
async fn test_thread_store_append() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();

    let delta = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
    let committed = store
        .append("t1", &delta, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 1);

    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 1);
    assert_eq!(head.agent_state.message_count(), 1); // from delta
}

#[tokio::test]
async fn test_thread_store_append_not_found() {
    let store = MemoryStore::new();
    let delta = sample_delta("run-1", CheckpointReason::RunFinished);
    let err = store
        .append("missing", &delta, VersionPrecondition::Any)
        .await
        .unwrap_err();
    assert!(matches!(err, AgentStateStoreError::NotFound(_)));
}

#[tokio::test]
async fn test_thread_store_delete() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();
    AgentStateWriter::delete(&store, "t1").await.unwrap();
    assert!(AgentStateReader::load(&store, "t1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_thread_store_append_with_snapshot() {
    let store = MemoryStore::new();
    let thread = AgentState::with_initial_state("t1", json!({"counter": 0}));
    store.create(&thread).await.unwrap();

    let delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![],
        patches: vec![],
        snapshot: Some(json!({"counter": 42})),
    };
    store
        .append("t1", &delta, VersionPrecondition::Any)
        .await
        .unwrap();

    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.agent_state.state, json!({"counter": 42}));
    assert!(head.agent_state.patches.is_empty());
}

#[tokio::test]
async fn test_thread_sync_load_deltas() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();

    let d1 = sample_delta("run-1", CheckpointReason::UserMessage);
    let d2 = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
    let d3 = sample_delta("run-1", CheckpointReason::RunFinished);
    store
        .append("t1", &d1, VersionPrecondition::Any)
        .await
        .unwrap();
    store
        .append("t1", &d2, VersionPrecondition::Any)
        .await
        .unwrap();
    store
        .append("t1", &d3, VersionPrecondition::Any)
        .await
        .unwrap();

    // All deltas
    let deltas = store.load_deltas("t1", 0).await.unwrap();
    assert_eq!(deltas.len(), 3);

    // Deltas after version 1
    let deltas = store.load_deltas("t1", 1).await.unwrap();
    assert_eq!(deltas.len(), 2);

    // Deltas after version 3 (none)
    let deltas = store.load_deltas("t1", 3).await.unwrap();
    assert_eq!(deltas.len(), 0);
}

#[tokio::test]
async fn test_thread_query_list_threads() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();
    store.create(&AgentState::new("t2")).await.unwrap();

    let page = store
        .list_agent_states(&AgentStateListQuery::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 2);
}

#[tokio::test]
async fn test_thread_query_list_threads_by_parent() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("parent")).await.unwrap();
    store
        .create(&AgentState::new("child-1").with_parent_thread_id("parent"))
        .await
        .unwrap();
    store
        .create(&AgentState::new("child-2").with_parent_thread_id("parent"))
        .await
        .unwrap();
    store.create(&AgentState::new("unrelated")).await.unwrap();

    let query = AgentStateListQuery {
        parent_thread_id: Some("parent".to_string()),
        ..Default::default()
    };
    let page = store.list_agent_states(&query).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.items.contains(&"child-1".to_string()));
    assert!(page.items.contains(&"child-2".to_string()));
}

#[tokio::test]
async fn test_thread_query_load_messages() {
    let store = MemoryStore::new();
    let thread = AgentState::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("hi"));
    store.create(&thread).await.unwrap();

    let page = AgentStateReader::load_messages(
        &store,
        "t1",
        &MessageQuery {
            limit: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert!(page.has_more);
}

#[tokio::test]
async fn test_parent_thread_id_persisted() {
    let thread = AgentState::new("child-1").with_parent_thread_id("parent-1");
    let json_str = serde_json::to_string(&thread).unwrap();
    assert!(json_str.contains("parent_thread_id"));

    let restored: AgentState = serde_json::from_str(&json_str).unwrap();
    assert_eq!(restored.parent_thread_id.as_deref(), Some("parent-1"));
}

#[tokio::test]
async fn test_parent_thread_id_none_omitted() {
    let thread = AgentState::new("t1");
    let json_str = serde_json::to_string(&thread).unwrap();
    assert!(!json_str.contains("parent_thread_id"));
}

// ========================================================================
// End-to-end: PendingDelta → AgentChangeSet → append (full agent flow)
// ========================================================================

/// Simulates a complete agent run: create → user message → assistant turn →
/// tool results → run finished, all via append().
#[tokio::test]
async fn test_full_agent_run_via_append() {
    let store = MemoryStore::new();

    // 1. Create thread
    let thread = AgentState::new("t1");
    let committed = store.create(&thread).await.unwrap();
    assert_eq!(committed.version, 0);

    // 2. User message delta (simulates http handler)
    let mut thread = thread.with_message(Message::user("What is 2+2?"));
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);
    assert!(pending.patches.is_empty());

    let user_delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::UserMessage,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store
        .append("t1", &user_delta, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 1);

    // 3. Assistant turn committed (LLM inference)
    thread = thread.with_message(Message::assistant("2+2 = 4"));
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);

    let assistant_delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store
        .append("t1", &assistant_delta, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 2);

    // 4. Tool results committed (with patches)
    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("result"), json!(4))));
    thread = thread
        .with_message(Message::tool("call-1", "4"))
        .with_patch(patch);
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);
    assert_eq!(pending.patches.len(), 1);

    let tool_delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store
        .append("t1", &tool_delta, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 3);

    // 5. Run finished (final assistant message)
    thread = thread.with_message(Message::assistant("The answer is 4."));
    let pending = thread.take_pending();

    let finished_delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store
        .append("t1", &finished_delta, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 4);

    // 6. Verify final state
    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 4);
    assert_eq!(head.agent_state.message_count(), 4); // user + assistant + tool + assistant
    assert_eq!(head.agent_state.patch_count(), 1);

    let state = head.agent_state.rebuild_state().unwrap();
    assert_eq!(state["result"], 4);
}

/// Verify AgentStateSync::load_deltas can replay the full run history.
#[tokio::test]
async fn test_delta_replay_reconstructs_thread() {
    let store = MemoryStore::new();
    let thread = AgentState::with_initial_state("t1", json!({"count": 0}));
    store.create(&thread).await.unwrap();

    // Simulate 3 rounds of append
    let deltas: Vec<AgentChangeSet> = vec![
        AgentChangeSet {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::UserMessage,
            messages: vec![Arc::new(Message::user("inc"))],
            patches: vec![TrackedPatch::new(
                Patch::new().with_op(Op::increment(path!("count"), 1)),
            )],
            snapshot: None,
        },
        AgentChangeSet {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::AssistantTurnCommitted,
            messages: vec![Arc::new(Message::assistant("done"))],
            patches: vec![TrackedPatch::new(
                Patch::new().with_op(Op::increment(path!("count"), 1)),
            )],
            snapshot: None,
        },
        AgentChangeSet {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::RunFinished,
            messages: vec![],
            patches: vec![],
            snapshot: None,
        },
    ];

    for delta in &deltas {
        store
            .append("t1", delta, VersionPrecondition::Any)
            .await
            .unwrap();
    }

    // Replay from scratch
    let all_deltas = store.load_deltas("t1", 0).await.unwrap();
    assert_eq!(all_deltas.len(), 3);

    // Reconstruct thread from empty + deltas
    let mut reconstructed = AgentState::with_initial_state("t1", json!({"count": 0}));
    for d in &all_deltas {
        for m in &d.messages {
            reconstructed.messages.push(m.clone());
        }
        reconstructed.patches.extend(d.patches.iter().cloned());
    }

    let loaded = store.load_agent_state("t1").await.unwrap().unwrap();
    assert_eq!(reconstructed.message_count(), loaded.message_count());
    assert_eq!(reconstructed.patch_count(), loaded.patch_count());

    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["count"], 2);
}

/// Verify partial replay: load_deltas(after_version=1) skips early deltas.
#[tokio::test]
async fn test_partial_delta_replay() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();

    for i in 0..5u64 {
        let delta = AgentChangeSet {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::AssistantTurnCommitted,
            messages: vec![Arc::new(Message::assistant(format!("msg-{i}")))],
            patches: vec![],
            snapshot: None,
        };
        store
            .append("t1", &delta, VersionPrecondition::Any)
            .await
            .unwrap();
    }

    // Only deltas after version 3 (should be versions 4 and 5)
    let deltas = store.load_deltas("t1", 3).await.unwrap();
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0].messages[0].content, "msg-3");
    assert_eq!(deltas[1].messages[0].content, "msg-4");
}

/// PendingDelta → AgentChangeSet → append preserves patch content and source.
#[tokio::test]
async fn test_append_preserves_patch_provenance() {
    let store = MemoryStore::new();
    store.create(&AgentState::new("t1")).await.unwrap();

    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("key"), json!("value"))))
        .with_source("tool:weather")
        .with_description("Set weather data");

    let mut thread = AgentState::new("t1").with_patch(patch);
    let pending = thread.take_pending();

    let delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    store
        .append("t1", &delta, VersionPrecondition::Any)
        .await
        .unwrap();

    // Verify provenance survived
    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.agent_state.patches.len(), 1);
    assert_eq!(
        head.agent_state.patches[0].source.as_deref(),
        Some("tool:weather")
    );
    assert_eq!(
        head.agent_state.patches[0].description.as_deref(),
        Some("Set weather data")
    );

    // Also via AgentStateSync
    let deltas = store.load_deltas("t1", 0).await.unwrap();
    assert_eq!(deltas[0].patches[0].source.as_deref(), Some("tool:weather"));
}

/// Verify parent_run_id is preserved through delta storage.
#[tokio::test]
async fn test_append_preserves_parent_run_id() {
    let store = MemoryStore::new();
    store
        .create(&AgentState::new("child").with_parent_thread_id("parent"))
        .await
        .unwrap();

    let delta = AgentChangeSet {
        run_id: "child-run-1".to_string(),
        parent_run_id: Some("parent-run-1".to_string()),
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("sub-agent reply"))],
        patches: vec![],
        snapshot: None,
    };
    store
        .append("child", &delta, VersionPrecondition::Any)
        .await
        .unwrap();

    let deltas = store.load_deltas("child", 0).await.unwrap();
    assert_eq!(deltas[0].run_id, "child-run-1");
    assert_eq!(deltas[0].parent_run_id.as_deref(), Some("parent-run-1"));

    let head = AgentStateReader::load(&store, "child")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.agent_state.parent_thread_id.as_deref(), Some("parent"));
}

/// Empty delta produces no change but still increments version.
#[tokio::test]
async fn test_append_empty_delta() {
    let store = MemoryStore::new();
    store
        .create(&AgentState::new("t1").with_message(Message::user("hi")))
        .await
        .unwrap();

    let empty = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![],
        patches: vec![],
        snapshot: None,
    };
    let committed = store
        .append("t1", &empty, VersionPrecondition::Any)
        .await
        .unwrap();
    assert_eq!(committed.version, 1);

    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 1);
    assert_eq!(head.agent_state.message_count(), 1); // unchanged
}

/// Simulates what `run_stream` does when the frontend sends a state snapshot
/// for an existing thread: the snapshot replaces the current state and is
/// persisted atomically in the UserMessage delta.
#[tokio::test]
async fn frontend_state_replaces_existing_thread_state_in_user_message_delta() {
    let store = MemoryStore::new();

    // 1. Create thread with initial state + a patch
    let thread = AgentState::with_initial_state("t1", json!({"counter": 0}));
    store.create(&thread).await.unwrap();
    let patch_delta = AgentChangeSet {
        run_id: "run-0".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: vec![],
        patches: vec![TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        )],
        snapshot: None,
    };
    store
        .append("t1", &patch_delta, VersionPrecondition::Any)
        .await
        .unwrap();

    // Verify current state: base={"counter":0}, 1 patch → rebuilt={"counter":5}
    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(
        head.agent_state.rebuild_state().unwrap(),
        json!({"counter": 5})
    );
    assert_eq!(head.agent_state.patches.len(), 1);

    // 2. Frontend sends state={"counter":10, "name":"Alice"} along with a user message.
    //    This simulates what run_stream does: include snapshot in UserMessage delta.
    let frontend_state = json!({"counter": 10, "name": "Alice"});
    let user_delta = AgentChangeSet {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::UserMessage,
        messages: vec![Arc::new(Message::user("hello"))],
        patches: vec![],
        snapshot: Some(frontend_state.clone()),
    };
    store
        .append("t1", &user_delta, VersionPrecondition::Any)
        .await
        .unwrap();

    // 3. Verify: state is fully replaced, patches cleared
    let head = AgentStateReader::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.agent_state.state, frontend_state);
    assert!(head.agent_state.patches.is_empty());
    assert_eq!(head.agent_state.rebuild_state().unwrap(), frontend_state);
    // User message was also persisted
    assert!(head
        .agent_state
        .messages
        .iter()
        .any(|m| m.role == Role::User));
}
