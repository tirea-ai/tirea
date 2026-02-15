use super::*;
use crate::types::Message;
use carve_state::{path, Op, Patch, TrackedPatch};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn test_memory_storage_save_load() {
    let storage = MemoryStore::new();
    let thread = Thread::new("test-1").with_message(Message::user("Hello"));

    storage.save(&thread).await.unwrap();
    let loaded = storage.load_thread("test-1").await.unwrap();

    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "test-1");
    assert_eq!(loaded.message_count(), 1);
}

#[tokio::test]
async fn test_memory_storage_load_not_found() {
    let storage = MemoryStore::new();
    let loaded = storage.load_thread("nonexistent").await.unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn test_memory_storage_delete() {
    let storage = MemoryStore::new();
    let thread = Thread::new("test-1");

    storage.save(&thread).await.unwrap();
    assert!(storage.load_thread("test-1").await.unwrap().is_some());

    storage.delete("test-1").await.unwrap();
    assert!(storage.load_thread("test-1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_memory_storage_list() {
    let storage = MemoryStore::new();

    storage.save(&Thread::new("thread-1")).await.unwrap();
    storage.save(&Thread::new("thread-2")).await.unwrap();

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
    let thread = Thread::new("test-1").with_message(Message::user("Hello"));
    storage.save(&thread).await.unwrap();

    // Update session
    let thread = thread.with_message(Message::assistant("Hi!"));
    storage.save(&thread).await.unwrap();

    // Load and verify
    let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 2);
}

#[tokio::test]
async fn test_memory_storage_with_state_and_patches() {
    let storage = MemoryStore::new();

    let thread = Thread::with_initial_state("test-1", json!({"counter": 0}))
        .with_message(Message::user("Increment"))
        .with_patch(TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        ));

    storage.save(&thread).await.unwrap();

    let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
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
                let thread = Thread::new(format!("thread-{}", i));
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

// File storage tests

#[tokio::test]
async fn test_file_storage_save_load() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    let thread = Thread::new("test-1").with_message(Message::user("Hello"));
    storage.save(&thread).await.unwrap();

    let loaded = storage.load_thread("test-1").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.id, "test-1");
    assert_eq!(loaded.message_count(), 1);
}

#[tokio::test]
async fn test_file_storage_load_not_found() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    let loaded = storage.load_thread("nonexistent").await.unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn test_file_storage_delete() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    let thread = Thread::new("test-1");
    storage.save(&thread).await.unwrap();
    assert!(storage.load_thread("test-1").await.unwrap().is_some());

    storage.delete("test-1").await.unwrap();
    assert!(storage.load_thread("test-1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_file_storage_list() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    storage.save(&Thread::new("thread-a")).await.unwrap();
    storage.save(&Thread::new("thread-b")).await.unwrap();
    storage.save(&Thread::new("thread-c")).await.unwrap();

    let mut ids = storage.list().await.unwrap();
    ids.sort();

    assert_eq!(ids.len(), 3);
    assert_eq!(ids, vec!["thread-a", "thread-b", "thread-c"]);
}

#[tokio::test]
async fn test_file_storage_list_empty_dir() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    let ids = storage.list().await.unwrap();
    assert!(ids.is_empty());
}

#[tokio::test]
async fn test_file_storage_list_nonexistent_dir() {
    let storage = FileStore::new("/tmp/nonexistent_carve_test_dir");
    let ids = storage.list().await.unwrap();
    assert!(ids.is_empty());
}

#[tokio::test]
async fn test_file_storage_creates_directory() {
    let temp_dir = TempDir::new().unwrap();
    let nested_path = temp_dir.path().join("nested").join("threads");
    let storage = FileStore::new(&nested_path);

    let thread = Thread::new("test-1");
    storage.save(&thread).await.unwrap();

    assert!(nested_path.exists());
    assert!(storage.load_thread("test-1").await.unwrap().is_some());
}

#[tokio::test]
async fn test_file_storage_with_complex_session() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    let thread = Thread::with_initial_state(
        "complex-thread",
        json!({
            "user": {"name": "Alice", "age": 30},
            "items": [1, 2, 3],
            "active": true
        }),
    )
    .with_message(Message::user("Hello"))
    .with_message(Message::assistant("Hi there!"))
    .with_patch(TrackedPatch::new(
        Patch::new()
            .with_op(Op::set(path!("user").key("age"), json!(31)))
            .with_op(Op::append(path!("items"), json!(4))),
    ));

    storage.save(&thread).await.unwrap();

    let loaded = storage
        .load_thread("complex-thread")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.message_count(), 2);
    assert_eq!(loaded.patch_count(), 1);

    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["user"]["age"], 31);
}

#[tokio::test]
async fn test_file_storage_update_session() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());

    // Save initial
    let thread = Thread::new("test-1").with_message(Message::user("First"));
    storage.save(&thread).await.unwrap();

    // Update
    let thread = thread
        .with_message(Message::assistant("Second"))
        .with_message(Message::user("Third"));
    storage.save(&thread).await.unwrap();

    // Verify
    let loaded = storage.load_thread("test-1").await.unwrap().unwrap();
    assert_eq!(loaded.message_count(), 3);
}

#[tokio::test]
async fn test_file_storage_ignores_non_json_files() {
    let temp_dir = TempDir::new().unwrap();

    // Create a non-JSON file
    tokio::fs::write(temp_dir.path().join("not-json.txt"), "hello")
        .await
        .unwrap();

    let storage = FileStore::new(temp_dir.path());
    storage.save(&Thread::new("valid")).await.unwrap();

    let ids = storage.list().await.unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], "valid");
}

#[test]
fn test_storage_error_display() {
    let err = StorageError::NotFound("thread-1".to_string());
    assert!(err.to_string().contains("not found"));
    assert!(err.to_string().contains("thread-1"));

    let err = StorageError::Serialization("invalid json".to_string());
    assert!(err.to_string().contains("Serialization"));
}

#[test]
fn test_file_storage_thread_path() {
    let storage = FileStore::new("/base/path");
    let path = storage.thread_path("my-thread").unwrap();
    assert_eq!(path.to_string_lossy(), "/base/path/my-thread.json");
}

#[test]
fn test_file_storage_rejects_path_traversal() {
    let storage = FileStore::new("/base/path");
    assert!(storage.thread_path("../../etc/passwd").is_err());
    assert!(storage.thread_path("foo/bar").is_err());
    assert!(storage.thread_path("foo\\bar").is_err());
    assert!(storage.thread_path("").is_err());
    assert!(storage.thread_path("foo\0bar").is_err());
}

// ========================================================================
// Pagination tests
// ========================================================================

fn make_messages(n: usize) -> Vec<std::sync::Arc<Message>> {
    (0..n)
        .map(|i| std::sync::Arc::new(Message::user(format!("msg-{}", i))))
        .collect()
}

fn make_thread_with_messages(thread_id: &str, n: usize) -> Thread {
    let mut thread = Thread::new(thread_id);
    for msg in make_messages(n) {
        // Deref Arc to get Message for with_message
        thread = thread.with_message((*msg).clone());
    }
    thread
}

#[test]
fn test_paginate_in_memory_basic() {
    let msgs = make_messages(10);
    let query = MessageQuery {
        limit: 3,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.messages[0].cursor, 0);
    assert_eq!(page.messages[1].cursor, 1);
    assert_eq!(page.messages[2].cursor, 2);
    assert_eq!(page.next_cursor, Some(2));
    assert_eq!(page.prev_cursor, Some(0));
}

#[test]
fn test_paginate_in_memory_cursor_forward() {
    let msgs = make_messages(10);
    let query = MessageQuery {
        after: Some(2),
        limit: 3,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    assert_eq!(page.messages[0].cursor, 3);
    assert_eq!(page.messages[1].cursor, 4);
    assert_eq!(page.messages[2].cursor, 5);
}

#[test]
fn test_paginate_in_memory_cursor_backward() {
    let msgs = make_messages(10);
    let query = MessageQuery {
        before: Some(8),
        limit: 3,
        order: SortOrder::Desc,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 3);
    assert!(page.has_more);
    // Desc order: highest cursors first
    assert_eq!(page.messages[0].cursor, 7);
    assert_eq!(page.messages[1].cursor, 6);
    assert_eq!(page.messages[2].cursor, 5);
}

#[test]
fn test_paginate_in_memory_empty() {
    let msgs: Vec<std::sync::Arc<Message>> = Vec::new();
    let query = MessageQuery::default();
    let page = paginate_in_memory(&msgs, &query);

    assert!(page.messages.is_empty());
    assert!(!page.has_more);
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.prev_cursor, None);
}

#[test]
fn test_paginate_in_memory_beyond_end() {
    let msgs = make_messages(5);
    let query = MessageQuery {
        after: Some(10),
        limit: 3,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert!(page.messages.is_empty());
    assert!(!page.has_more);
}

#[test]
fn test_paginate_in_memory_exact_fit() {
    let msgs = make_messages(3);
    let query = MessageQuery {
        limit: 3,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 3);
    assert!(!page.has_more);
}

#[test]
fn test_paginate_in_memory_last_page() {
    let msgs = make_messages(10);
    let query = MessageQuery {
        after: Some(7),
        limit: 5,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 2);
    assert!(!page.has_more);
    assert_eq!(page.messages[0].cursor, 8);
    assert_eq!(page.messages[1].cursor, 9);
}

#[test]
fn test_paginate_in_memory_after_and_before() {
    let msgs = make_messages(10);
    let query = MessageQuery {
        after: Some(2),
        before: Some(7),
        limit: 10,
        ..Default::default()
    };
    let page = paginate_in_memory(&msgs, &query);

    assert_eq!(page.messages.len(), 4);
    assert!(!page.has_more);
    assert_eq!(page.messages[0].cursor, 3);
    assert_eq!(page.messages[3].cursor, 6);
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
    let page = ThreadReadStore::load_messages(&storage, "test-1", &query)
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
    let result = ThreadReadStore::load_messages(&storage, "nonexistent", &query).await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
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
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

#[tokio::test]
async fn test_file_storage_load_messages() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());
    let thread = make_thread_with_messages("test-1", 10);
    storage.save(&thread).await.unwrap();

    let query = MessageQuery {
        after: Some(4),
        limit: 3,
        ..Default::default()
    };
    let page = ThreadReadStore::load_messages(&storage, "test-1", &query)
        .await
        .unwrap();

    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].cursor, 5);
    assert_eq!(page.messages[0].message.content, "msg-5");
}

#[tokio::test]
async fn test_file_storage_message_count() {
    let temp_dir = TempDir::new().unwrap();
    let storage = FileStore::new(temp_dir.path());
    let thread = make_thread_with_messages("test-1", 5);
    storage.save(&thread).await.unwrap();

    let count = storage.message_count("test-1").await.unwrap();
    assert_eq!(count, 5);
}

#[test]
fn test_message_page_serialization() {
    let page = MessagePage {
        messages: vec![MessageWithCursor {
            cursor: 0,
            message: Message::user("hello"),
        }],
        has_more: true,
        next_cursor: Some(0),
        prev_cursor: Some(0),
    };
    let json = serde_json::to_string(&page).unwrap();
    assert!(json.contains("\"cursor\":0"));
    assert!(json.contains("\"has_more\":true"));
    assert!(json.contains("\"next_cursor\":0"));
}

// ========================================================================
// Visibility tests
// ========================================================================

fn make_mixed_visibility_thread(thread_id: &str) -> Thread {
    Thread::new(thread_id)
        .with_message(Message::user("user-0"))
        .with_message(Message::assistant("assistant-1"))
        .with_message(Message::internal_system("reminder-2"))
        .with_message(Message::user("user-3"))
        .with_message(Message::internal_system("reminder-4"))
        .with_message(Message::assistant("assistant-5"))
}

#[test]
fn test_paginate_visibility_all_default() {
    let thread = make_mixed_visibility_thread("t");
    // Default query filters to Visibility::All (user-visible only).
    let query = MessageQuery {
        limit: 50,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    // Should exclude 2 internal messages (at indices 2 and 4).
    assert_eq!(page.messages.len(), 4);
    assert_eq!(page.messages[0].message.content, "user-0");
    assert_eq!(page.messages[1].message.content, "assistant-1");
    assert_eq!(page.messages[2].message.content, "user-3");
    assert_eq!(page.messages[3].message.content, "assistant-5");
}

#[test]
fn test_paginate_visibility_internal_only() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        limit: 50,
        visibility: Some(Visibility::Internal),
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].message.content, "reminder-2");
    assert_eq!(page.messages[1].message.content, "reminder-4");
}

#[test]
fn test_paginate_visibility_none_returns_all() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        limit: 50,
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    // Should return all 6 messages.
    assert_eq!(page.messages.len(), 6);
}

#[test]
fn test_paginate_visibility_cursors_stable() {
    let thread = make_mixed_visibility_thread("t");
    // With visibility=All, cursors should correspond to original indices.
    let query = MessageQuery {
        limit: 50,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    // Cursors should be 0, 1, 3, 5 (original indices, skipping internal at 2, 4).
    assert_eq!(page.messages[0].cursor, 0);
    assert_eq!(page.messages[1].cursor, 1);
    assert_eq!(page.messages[2].cursor, 3);
    assert_eq!(page.messages[3].cursor, 5);
}

#[test]
fn test_paginate_visibility_with_cursor_pagination() {
    let thread = make_mixed_visibility_thread("t");
    // Start after cursor 1 (assistant-1), visibility=All.
    let query = MessageQuery {
        after: Some(1),
        limit: 2,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    // Should return user-3 (cursor 3) and assistant-5 (cursor 5).
    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].cursor, 3);
    assert_eq!(page.messages[0].message.content, "user-3");
    assert_eq!(page.messages[1].cursor, 5);
    assert_eq!(page.messages[1].message.content, "assistant-5");
    assert!(!page.has_more);
}

#[test]
fn test_internal_system_message_serialization() {
    let msg = Message::internal_system("a reminder");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"visibility\":\"internal\""));

    // Round-trip
    let parsed: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.visibility, Visibility::Internal);
    assert_eq!(parsed.content, "a reminder");
}

#[test]
fn test_user_message_omits_visibility_in_json() {
    let msg = Message::user("hello");
    let json = serde_json::to_string(&msg).unwrap();
    // Default visibility should be skipped in serialization.
    assert!(!json.contains("visibility"));

    // Deserializing without visibility field should default to All.
    let parsed: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.visibility, Visibility::All);
}

#[tokio::test]
async fn test_memory_storage_load_messages_filters_visibility() {
    let storage = MemoryStore::new();
    let thread = make_mixed_visibility_thread("test-vis");
    storage.save(&thread).await.unwrap();

    // Default query (visibility = All)
    let page = ThreadReadStore::load_messages(&storage, "test-vis", &MessageQuery::default())
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 4);

    // visibility = None (all messages)
    let query = MessageQuery {
        visibility: None,
        ..Default::default()
    };
    let page = ThreadReadStore::load_messages(&storage, "test-vis", &query)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 6);
}

// ========================================================================
// Run ID filtering tests
// ========================================================================

use crate::types::MessageMetadata;

fn make_multi_run_thread(thread_id: &str) -> Thread {
    Thread::new(thread_id)
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

#[test]
fn test_paginate_filter_by_run_id() {
    let thread = make_multi_run_thread("t");

    // Filter to run-a only
    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].message.content, "thinking...");
    assert_eq!(page.messages[2].message.content, "done");

    // Filter to run-b only
    let query = MessageQuery {
        run_id: Some("run-b".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].message.content, "ok");

    // No run filter: returns all 6
    let query = MessageQuery {
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 6);
}

#[test]
fn test_paginate_run_id_with_cursor() {
    let thread = make_multi_run_thread("t");

    // Filter run-a, after cursor 1 (skip first run-a msg at cursor 1)
    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        after: Some(1),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 2); // tool (cursor 2) + final (cursor 3)
}

#[test]
fn test_paginate_nonexistent_run_id() {
    let thread = make_multi_run_thread("t");
    let query = MessageQuery {
        run_id: Some("run-z".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert!(page.messages.is_empty());
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
    let page = ThreadReadStore::load_messages(&storage, "test-run", &query)
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 3);
}

#[test]
fn test_message_metadata_preserved_in_pagination() {
    let thread = make_multi_run_thread("t");
    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    // Metadata should be preserved on the returned messages.
    let meta = page.messages[0].message.metadata.as_ref().unwrap();
    assert_eq!(meta.run_id.as_deref(), Some("run-a"));
    assert_eq!(meta.step_index, Some(0));
}

// ========================================================================
// Thread list pagination tests
// ========================================================================

#[tokio::test]
async fn test_list_paginated_default() {
    let storage = MemoryStore::new();
    for i in 0..5 {
        storage
            .save(&Thread::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&ThreadListQuery::default())
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
            .save(&Thread::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&ThreadListQuery {
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
            .save(&Thread::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&ThreadListQuery {
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
            .save(&Thread::new(format!("s-{i:02}")))
            .await
            .unwrap();
    }
    let page = storage
        .list_paginated(&ThreadListQuery {
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
        .list_paginated(&ThreadListQuery::default())
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.total, 0);
    assert!(!page.has_more);
}

// ========================================================================
// ThreadWriteStore / ThreadReadStore / ThreadSync tests
// ========================================================================

fn sample_delta(run_id: &str, reason: CheckpointReason) -> ThreadDelta {
    ThreadDelta {
        run_id: run_id.to_string(),
        parent_run_id: None,
        reason,
        messages: vec![Arc::new(Message::assistant("hello"))],
        patches: vec![],
        snapshot: None,
    }
}

#[tokio::test]
async fn test_thread_store_create_and_load() {
    let store = MemoryStore::new();
    let thread = Thread::new("t1").with_message(Message::user("hi"));
    let committed = store.create(&thread).await.unwrap();
    assert_eq!(committed.version, 0);

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 0);
    assert_eq!(head.thread.id, "t1");
    assert_eq!(head.thread.message_count(), 1);
}

#[tokio::test]
async fn test_thread_store_create_already_exists() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();
    let err = store.create(&Thread::new("t1")).await.unwrap_err();
    assert!(matches!(err, StorageError::AlreadyExists));
}

#[tokio::test]
async fn test_thread_store_append() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();

    let delta = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
    let committed = store.append("t1", &delta).await.unwrap();
    assert_eq!(committed.version, 1);

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 1);
    assert_eq!(head.thread.message_count(), 1); // from delta
}

#[tokio::test]
async fn test_thread_store_append_not_found() {
    let store = MemoryStore::new();
    let delta = sample_delta("run-1", CheckpointReason::RunFinished);
    let err = store.append("missing", &delta).await.unwrap_err();
    assert!(matches!(err, StorageError::NotFound(_)));
}

#[tokio::test]
async fn test_thread_store_delete() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();
    ThreadWriteStore::delete(&store, "t1").await.unwrap();
    assert!(ThreadReadStore::load(&store, "t1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_thread_store_append_with_snapshot() {
    let store = MemoryStore::new();
    let thread = Thread::with_initial_state("t1", json!({"counter": 0}));
    store.create(&thread).await.unwrap();

    let delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![],
        patches: vec![],
        snapshot: Some(json!({"counter": 42})),
    };
    store.append("t1", &delta).await.unwrap();

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.thread.state, json!({"counter": 42}));
    assert!(head.thread.patches.is_empty());
}

#[tokio::test]
async fn test_thread_sync_load_deltas() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();

    let d1 = sample_delta("run-1", CheckpointReason::UserMessage);
    let d2 = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
    let d3 = sample_delta("run-1", CheckpointReason::RunFinished);
    store.append("t1", &d1).await.unwrap();
    store.append("t1", &d2).await.unwrap();
    store.append("t1", &d3).await.unwrap();

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
    store.create(&Thread::new("t1")).await.unwrap();
    store.create(&Thread::new("t2")).await.unwrap();

    let page = store
        .list_threads(&ThreadListQuery::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 2);
}

#[tokio::test]
async fn test_thread_query_list_threads_by_parent() {
    let store = MemoryStore::new();
    store.create(&Thread::new("parent")).await.unwrap();
    store
        .create(&Thread::new("child-1").with_parent_thread_id("parent"))
        .await
        .unwrap();
    store
        .create(&Thread::new("child-2").with_parent_thread_id("parent"))
        .await
        .unwrap();
    store.create(&Thread::new("unrelated")).await.unwrap();

    let query = ThreadListQuery {
        parent_thread_id: Some("parent".to_string()),
        ..Default::default()
    };
    let page = store.list_threads(&query).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert!(page.items.contains(&"child-1".to_string()));
    assert!(page.items.contains(&"child-2".to_string()));
}

#[tokio::test]
async fn test_thread_query_load_messages() {
    let store = MemoryStore::new();
    let thread = Thread::new("t1")
        .with_message(Message::user("hello"))
        .with_message(Message::assistant("hi"));
    store.create(&thread).await.unwrap();

    let page = ThreadReadStore::load_messages(
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
    let thread = Thread::new("child-1").with_parent_thread_id("parent-1");
    let json_str = serde_json::to_string(&thread).unwrap();
    assert!(json_str.contains("parent_thread_id"));

    let restored: Thread = serde_json::from_str(&json_str).unwrap();
    assert_eq!(restored.parent_thread_id.as_deref(), Some("parent-1"));
}

#[tokio::test]
async fn test_parent_thread_id_none_omitted() {
    let thread = Thread::new("t1");
    let json_str = serde_json::to_string(&thread).unwrap();
    assert!(!json_str.contains("parent_thread_id"));
}

// ========================================================================
// FileStore ThreadWriteStore tests
// ========================================================================

#[tokio::test]
async fn test_file_thread_store_create_and_load() {
    let temp_dir = TempDir::new().unwrap();
    let store = FileStore::new(temp_dir.path());
    let thread = Thread::new("t1").with_message(Message::user("hi"));
    let committed = store.create(&thread).await.unwrap();
    assert_eq!(committed.version, 0);

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 0);
    assert_eq!(head.thread.message_count(), 1);
}

#[tokio::test]
async fn test_file_thread_store_append_and_version() {
    let temp_dir = TempDir::new().unwrap();
    let store = FileStore::new(temp_dir.path());
    store.create(&Thread::new("t1")).await.unwrap();

    let delta = sample_delta("run-1", CheckpointReason::AssistantTurnCommitted);
    let committed = store.append("t1", &delta).await.unwrap();
    assert_eq!(committed.version, 1);

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 1);
    assert_eq!(head.thread.message_count(), 1);
}

#[tokio::test]
async fn test_file_thread_store_already_exists() {
    let temp_dir = TempDir::new().unwrap();
    let store = FileStore::new(temp_dir.path());
    store.create(&Thread::new("t1")).await.unwrap();
    let err = store.create(&Thread::new("t1")).await.unwrap_err();
    assert!(matches!(err, StorageError::AlreadyExists));
}

// ========================================================================
// End-to-end: PendingDelta → ThreadDelta → append (full agent flow)
// ========================================================================

/// Simulates a complete agent run: create → user message → assistant turn →
/// tool results → run finished, all via append().
#[tokio::test]
async fn test_full_agent_run_via_append() {
    let store = MemoryStore::new();

    // 1. Create thread
    let thread = Thread::new("t1");
    let committed = store.create(&thread).await.unwrap();
    assert_eq!(committed.version, 0);

    // 2. User message delta (simulates http handler)
    let mut thread = thread.with_message(Message::user("What is 2+2?"));
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);
    assert!(pending.patches.is_empty());

    let user_delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::UserMessage,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store.append("t1", &user_delta).await.unwrap();
    assert_eq!(committed.version, 1);

    // 3. Assistant turn committed (LLM inference)
    thread = thread.with_message(Message::assistant("2+2 = 4"));
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);

    let assistant_delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store.append("t1", &assistant_delta).await.unwrap();
    assert_eq!(committed.version, 2);

    // 4. Tool results committed (with patches)
    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("result"), json!(4))));
    thread = thread
        .with_message(Message::tool("call-1", "4"))
        .with_patch(patch);
    let pending = thread.take_pending();
    assert_eq!(pending.messages.len(), 1);
    assert_eq!(pending.patches.len(), 1);

    let tool_delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store.append("t1", &tool_delta).await.unwrap();
    assert_eq!(committed.version, 3);

    // 5. Run finished (final assistant message)
    thread = thread.with_message(Message::assistant("The answer is 4."));
    let pending = thread.take_pending();

    let finished_delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    let committed = store.append("t1", &finished_delta).await.unwrap();
    assert_eq!(committed.version, 4);

    // 6. Verify final state
    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 4);
    assert_eq!(head.thread.message_count(), 4); // user + assistant + tool + assistant
    assert_eq!(head.thread.patch_count(), 1);

    let state = head.thread.rebuild_state().unwrap();
    assert_eq!(state["result"], 4);
}

/// Verify ThreadSync::load_deltas can replay the full run history.
#[tokio::test]
async fn test_delta_replay_reconstructs_thread() {
    let store = MemoryStore::new();
    let thread = Thread::with_initial_state("t1", json!({"count": 0}));
    store.create(&thread).await.unwrap();

    // Simulate 3 rounds of append
    let deltas: Vec<ThreadDelta> = vec![
        ThreadDelta {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::UserMessage,
            messages: vec![Arc::new(Message::user("inc"))],
            patches: vec![TrackedPatch::new(
                Patch::new().with_op(Op::increment(path!("count"), 1)),
            )],
            snapshot: None,
        },
        ThreadDelta {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::AssistantTurnCommitted,
            messages: vec![Arc::new(Message::assistant("done"))],
            patches: vec![TrackedPatch::new(
                Patch::new().with_op(Op::increment(path!("count"), 1)),
            )],
            snapshot: None,
        },
        ThreadDelta {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::RunFinished,
            messages: vec![],
            patches: vec![],
            snapshot: None,
        },
    ];

    for delta in &deltas {
        store.append("t1", delta).await.unwrap();
    }

    // Replay from scratch
    let all_deltas = store.load_deltas("t1", 0).await.unwrap();
    assert_eq!(all_deltas.len(), 3);

    // Reconstruct thread from empty + deltas
    let mut reconstructed = Thread::with_initial_state("t1", json!({"count": 0}));
    for d in &all_deltas {
        for m in &d.messages {
            reconstructed.messages.push(m.clone());
        }
        reconstructed.patches.extend(d.patches.iter().cloned());
    }

    let loaded = store.load_thread("t1").await.unwrap().unwrap();
    assert_eq!(reconstructed.message_count(), loaded.message_count());
    assert_eq!(reconstructed.patch_count(), loaded.patch_count());

    let state = loaded.rebuild_state().unwrap();
    assert_eq!(state["count"], 2);
}

/// Verify partial replay: load_deltas(after_version=1) skips early deltas.
#[tokio::test]
async fn test_partial_delta_replay() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();

    for i in 0..5u64 {
        let delta = ThreadDelta {
            run_id: "run-1".to_string(),
            parent_run_id: None,
            reason: CheckpointReason::AssistantTurnCommitted,
            messages: vec![Arc::new(Message::assistant(format!("msg-{i}")))],
            patches: vec![],
            snapshot: None,
        };
        store.append("t1", &delta).await.unwrap();
    }

    // Only deltas after version 3 (should be versions 4 and 5)
    let deltas = store.load_deltas("t1", 3).await.unwrap();
    assert_eq!(deltas.len(), 2);
    assert_eq!(deltas[0].messages[0].content, "msg-3");
    assert_eq!(deltas[1].messages[0].content, "msg-4");
}

/// PendingDelta → ThreadDelta → append preserves patch content and source.
#[tokio::test]
async fn test_append_preserves_patch_provenance() {
    let store = MemoryStore::new();
    store.create(&Thread::new("t1")).await.unwrap();

    let patch = TrackedPatch::new(Patch::new().with_op(Op::set(path!("key"), json!("value"))))
        .with_source("tool:weather")
        .with_description("Set weather data");

    let mut thread = Thread::new("t1").with_patch(patch);
    let pending = thread.take_pending();

    let delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: pending.messages,
        patches: pending.patches,
        snapshot: None,
    };
    store.append("t1", &delta).await.unwrap();

    // Verify provenance survived
    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.thread.patches.len(), 1);
    assert_eq!(
        head.thread.patches[0].source.as_deref(),
        Some("tool:weather")
    );
    assert_eq!(
        head.thread.patches[0].description.as_deref(),
        Some("Set weather data")
    );

    // Also via ThreadSync
    let deltas = store.load_deltas("t1", 0).await.unwrap();
    assert_eq!(deltas[0].patches[0].source.as_deref(), Some("tool:weather"));
}

/// Verify parent_run_id is preserved through delta storage.
#[tokio::test]
async fn test_append_preserves_parent_run_id() {
    let store = MemoryStore::new();
    store
        .create(&Thread::new("child").with_parent_thread_id("parent"))
        .await
        .unwrap();

    let delta = ThreadDelta {
        run_id: "child-run-1".to_string(),
        parent_run_id: Some("parent-run-1".to_string()),
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("sub-agent reply"))],
        patches: vec![],
        snapshot: None,
    };
    store.append("child", &delta).await.unwrap();

    let deltas = store.load_deltas("child", 0).await.unwrap();
    assert_eq!(deltas[0].run_id, "child-run-1");
    assert_eq!(deltas[0].parent_run_id.as_deref(), Some("parent-run-1"));

    let head = ThreadReadStore::load(&store, "child")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.thread.parent_thread_id.as_deref(), Some("parent"));
}

/// FileStore: multi-round append with version tracking persisted to disk.
#[tokio::test]
async fn test_file_multi_round_append() {
    let temp_dir = TempDir::new().unwrap();
    let store = FileStore::new(temp_dir.path());
    store.create(&Thread::new("t1")).await.unwrap();

    // Round 1: user message
    let d1 = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::UserMessage,
        messages: vec![Arc::new(Message::user("hello"))],
        patches: vec![],
        snapshot: None,
    };
    let c1 = store.append("t1", &d1).await.unwrap();
    assert_eq!(c1.version, 1);

    // Round 2: assistant response + patch
    let d2 = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::AssistantTurnCommitted,
        messages: vec![Arc::new(Message::assistant("hi"))],
        patches: vec![TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("greeted"), json!(true))),
        )],
        snapshot: None,
    };
    let c2 = store.append("t1", &d2).await.unwrap();
    assert_eq!(c2.version, 2);

    // Round 3: run finished with snapshot
    let d3 = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![],
        patches: vec![],
        snapshot: Some(json!({"greeted": true})),
    };
    let c3 = store.append("t1", &d3).await.unwrap();
    assert_eq!(c3.version, 3);

    // Re-create store from same path (simulates restart)
    let store2 = FileStore::new(temp_dir.path());
    let head = ThreadReadStore::load(&store2, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 3);
    assert_eq!(head.thread.message_count(), 2);
    assert!(head.thread.patches.is_empty()); // snapshot cleared patches
    assert_eq!(head.thread.state, json!({"greeted": true}));
}

/// Empty delta produces no change but still increments version.
#[tokio::test]
async fn test_append_empty_delta() {
    let store = MemoryStore::new();
    store
        .create(&Thread::new("t1").with_message(Message::user("hi")))
        .await
        .unwrap();

    let empty = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::RunFinished,
        messages: vec![],
        patches: vec![],
        snapshot: None,
    };
    let committed = store.append("t1", &empty).await.unwrap();
    assert_eq!(committed.version, 1);

    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.version, 1);
    assert_eq!(head.thread.message_count(), 1); // unchanged
}

/// Simulates what `run_stream` does when the frontend sends a state snapshot
/// for an existing thread: the snapshot replaces the current state and is
/// persisted atomically in the UserMessage delta.
#[tokio::test]
async fn frontend_state_replaces_existing_thread_state_in_user_message_delta() {
    let store = MemoryStore::new();

    // 1. Create thread with initial state + a patch
    let thread = Thread::with_initial_state("t1", json!({"counter": 0}));
    store.create(&thread).await.unwrap();
    let patch_delta = ThreadDelta {
        run_id: "run-0".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::ToolResultsCommitted,
        messages: vec![],
        patches: vec![TrackedPatch::new(
            Patch::new().with_op(Op::set(path!("counter"), json!(5))),
        )],
        snapshot: None,
    };
    store.append("t1", &patch_delta).await.unwrap();

    // Verify current state: base={"counter":0}, 1 patch → rebuilt={"counter":5}
    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.thread.rebuild_state().unwrap(), json!({"counter": 5}));
    assert_eq!(head.thread.patches.len(), 1);

    // 2. Frontend sends state={"counter":10, "name":"Alice"} along with a user message.
    //    This simulates what run_stream does: include snapshot in UserMessage delta.
    let frontend_state = json!({"counter": 10, "name": "Alice"});
    let user_delta = ThreadDelta {
        run_id: "run-1".to_string(),
        parent_run_id: None,
        reason: CheckpointReason::UserMessage,
        messages: vec![Arc::new(Message::user("hello"))],
        patches: vec![],
        snapshot: Some(frontend_state.clone()),
    };
    store.append("t1", &user_delta).await.unwrap();

    // 3. Verify: state is fully replaced, patches cleared
    let head = ThreadReadStore::load(&store, "t1").await.unwrap().unwrap();
    assert_eq!(head.thread.state, frontend_state);
    assert!(head.thread.patches.is_empty());
    assert_eq!(head.thread.rebuild_state().unwrap(), frontend_state);
    // User message was also persisted
    assert!(head
        .thread
        .messages
        .iter()
        .any(|m| m.role == crate::types::Role::User));
}
