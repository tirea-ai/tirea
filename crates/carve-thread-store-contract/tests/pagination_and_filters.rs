use carve_agent_contract::{Message, MessageMetadata, AgentState, Visibility};
use carve_agent_contract::storage::{
    paginate_in_memory, MessagePage, MessageQuery, MessageWithCursor, SortOrder, ThreadStoreError,
};
use std::sync::Arc;

fn make_messages(n: usize) -> Vec<Arc<Message>> {
    (0..n)
        .map(|i| Arc::new(Message::user(format!("msg-{i}"))))
        .collect()
}

fn make_mixed_visibility_thread(thread_id: &str) -> AgentState {
    AgentState::new(thread_id)
        .with_message(Message::user("user-0"))
        .with_message(Message::assistant("assistant-1"))
        .with_message(Message::internal_system("reminder-2"))
        .with_message(Message::user("user-3"))
        .with_message(Message::internal_system("reminder-4"))
        .with_message(Message::assistant("assistant-5"))
}

fn make_multi_run_thread(thread_id: &str) -> AgentState {
    AgentState::new(thread_id)
        .with_message(Message::user("hello"))
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
        .with_message(Message::assistant("done").with_metadata(MessageMetadata {
            run_id: Some("run-a".to_string()),
            step_index: Some(1),
        }))
        .with_message(Message::user("more"))
        .with_message(Message::assistant("ok").with_metadata(MessageMetadata {
            run_id: Some("run-b".to_string()),
            step_index: Some(0),
        }))
}

#[test]
fn storage_error_display_includes_context() {
    let err = ThreadStoreError::NotFound("thread-1".to_string());
    assert!(err.to_string().contains("not found"));
    assert!(err.to_string().contains("thread-1"));

    let err = ThreadStoreError::Serialization("invalid json".to_string());
    assert!(err.to_string().contains("Serialization"));
}

#[test]
fn message_page_serialization_roundtrip_shape() {
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

#[test]
fn paginate_in_memory_basic() {
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
fn paginate_in_memory_cursor_forward() {
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
fn paginate_in_memory_cursor_backward() {
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
    assert_eq!(page.messages[0].cursor, 7);
    assert_eq!(page.messages[1].cursor, 6);
    assert_eq!(page.messages[2].cursor, 5);
}

#[test]
fn paginate_in_memory_empty() {
    let msgs: Vec<Arc<Message>> = Vec::new();
    let query = MessageQuery::default();
    let page = paginate_in_memory(&msgs, &query);

    assert!(page.messages.is_empty());
    assert!(!page.has_more);
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.prev_cursor, None);
}

#[test]
fn paginate_in_memory_beyond_end() {
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
fn paginate_in_memory_exact_fit() {
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
fn paginate_in_memory_last_page() {
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
fn paginate_in_memory_after_and_before() {
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

#[test]
fn paginate_visibility_all_default() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        limit: 50,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    assert_eq!(page.messages.len(), 4);
    assert_eq!(page.messages[0].message.content, "user-0");
    assert_eq!(page.messages[1].message.content, "assistant-1");
    assert_eq!(page.messages[2].message.content, "user-3");
    assert_eq!(page.messages[3].message.content, "assistant-5");
}

#[test]
fn paginate_visibility_internal_only() {
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
fn paginate_visibility_none_returns_all() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        limit: 50,
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    assert_eq!(page.messages.len(), 6);
}

#[test]
fn paginate_visibility_cursors_stable() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        limit: 50,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    assert_eq!(page.messages[0].cursor, 0);
    assert_eq!(page.messages[1].cursor, 1);
    assert_eq!(page.messages[2].cursor, 3);
    assert_eq!(page.messages[3].cursor, 5);
}

#[test]
fn paginate_visibility_with_cursor_pagination() {
    let thread = make_mixed_visibility_thread("t");
    let query = MessageQuery {
        after: Some(1),
        limit: 2,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    assert_eq!(page.messages.len(), 2);
    assert_eq!(page.messages[0].cursor, 3);
    assert_eq!(page.messages[0].message.content, "user-3");
    assert_eq!(page.messages[1].cursor, 5);
    assert_eq!(page.messages[1].message.content, "assistant-5");
    assert!(!page.has_more);
}

#[test]
fn internal_system_message_serialization() {
    let msg = Message::internal_system("a reminder");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"visibility\":\"internal\""));

    let parsed: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.visibility, Visibility::Internal);
    assert_eq!(parsed.content, "a reminder");
}

#[test]
fn user_message_omits_visibility_in_json() {
    let msg = Message::user("hello");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(!json.contains("visibility"));

    let parsed: Message = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.visibility, Visibility::All);
}

#[test]
fn paginate_filter_by_run_id() {
    let thread = make_multi_run_thread("t");

    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 3);
    assert_eq!(page.messages[0].message.content, "thinking...");
    assert_eq!(page.messages[2].message.content, "done");

    let query = MessageQuery {
        run_id: Some("run-b".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].message.content, "ok");

    let query = MessageQuery {
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 6);
}

#[test]
fn paginate_run_id_with_cursor() {
    let thread = make_multi_run_thread("t");
    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        after: Some(1),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert_eq!(page.messages.len(), 2);
}

#[test]
fn paginate_nonexistent_run_id() {
    let thread = make_multi_run_thread("t");
    let query = MessageQuery {
        run_id: Some("run-z".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);
    assert!(page.messages.is_empty());
}

#[test]
fn message_metadata_preserved_in_pagination() {
    let thread = make_multi_run_thread("t");
    let query = MessageQuery {
        run_id: Some("run-a".to_string()),
        visibility: None,
        ..Default::default()
    };
    let page = paginate_in_memory(&thread.messages, &query);

    let meta = page.messages[0].message.metadata.as_ref().unwrap();
    assert_eq!(meta.run_id.as_deref(), Some("run-a"));
    assert_eq!(meta.step_index, Some(0));
}
