//! In-memory transactional implementation of [`ThreadRunStore`] for testing.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;

use super::message::Message;
use super::storage::{RunRecord, StorageError, ThreadRunStore};

#[derive(Debug, Default)]
struct ThreadRunData {
    threads: HashMap<String, Vec<Message>>,
    runs: HashMap<String, RunRecord>,
}

/// In-memory transactional thread+run store.
#[derive(Debug, Default)]
pub struct InMemoryThreadRunStore {
    inner: RwLock<ThreadRunData>,
}

impl InMemoryThreadRunStore {
    /// Create a new empty transactional store.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ThreadRunStore for InMemoryThreadRunStore {
    async fn load_messages(&self, thread_id: &str) -> Result<Option<Vec<Message>>, StorageError> {
        let guard = self
            .inner
            .read()
            .map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(guard.threads.get(thread_id).cloned())
    }

    async fn checkpoint(
        &self,
        thread_id: &str,
        messages: &[Message],
        run: &RunRecord,
    ) -> Result<(), StorageError> {
        let mut guard = self
            .inner
            .write()
            .map_err(|e| StorageError::Io(e.to_string()))?;
        guard
            .threads
            .insert(thread_id.to_owned(), messages.to_vec());
        guard.runs.insert(run.run_id.clone(), run.clone());
        Ok(())
    }

    async fn load_run(&self, run_id: &str) -> Result<Option<RunRecord>, StorageError> {
        let guard = self
            .inner
            .read()
            .map_err(|e| StorageError::Io(e.to_string()))?;
        Ok(guard.runs.get(run_id).cloned())
    }

    async fn latest_run(&self, thread_id: &str) -> Result<Option<RunRecord>, StorageError> {
        let guard = self
            .inner
            .read()
            .map_err(|e| StorageError::Io(e.to_string()))?;
        let latest = guard
            .runs
            .values()
            .filter(|r| r.thread_id == thread_id)
            .max_by_key(|r| r.updated_at)
            .cloned();
        Ok(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::lifecycle::RunStatus;

    fn make_run(run_id: &str, thread_id: &str, updated_at: u64) -> RunRecord {
        RunRecord {
            run_id: run_id.to_owned(),
            thread_id: thread_id.to_owned(),
            agent_id: "agent-1".to_owned(),
            parent_run_id: None,
            status: RunStatus::Running,
            termination_code: None,
            created_at: 1000,
            updated_at,
            steps: 0,
            input_tokens: 0,
            output_tokens: 0,
            state: None,
        }
    }

    #[tokio::test]
    async fn checkpoint_persists_thread_and_run_atomically() {
        let store = InMemoryThreadRunStore::new();
        let run = make_run("run-x", "thread-x", 42);
        let messages = vec![Message::user("u1"), Message::assistant("a1")];

        store
            .checkpoint("thread-x", &messages, &run)
            .await
            .expect("checkpoint should succeed");

        let loaded_messages = store
            .load_messages("thread-x")
            .await
            .expect("load messages")
            .expect("thread exists");
        assert_eq!(loaded_messages.len(), 2);
        assert_eq!(loaded_messages[0].text(), "u1");

        let loaded_run = store
            .load_run("run-x")
            .await
            .expect("load run")
            .expect("run exists");
        assert_eq!(loaded_run.thread_id, "thread-x");
        assert_eq!(loaded_run.updated_at, 42);
    }

    #[tokio::test]
    async fn latest_run_by_thread() {
        let store = InMemoryThreadRunStore::new();
        let msgs = vec![Message::user("m")];
        store
            .checkpoint("thread-1", &msgs, &make_run("run-1", "thread-1", 100))
            .await
            .unwrap();
        store
            .checkpoint("thread-1", &msgs, &make_run("run-2", "thread-1", 200))
            .await
            .unwrap();
        store
            .checkpoint("thread-2", &msgs, &make_run("run-3", "thread-2", 300))
            .await
            .unwrap();

        let latest = store.latest_run("thread-1").await.unwrap().unwrap();
        assert_eq!(latest.run_id, "run-2");
        let latest2 = store.latest_run("thread-2").await.unwrap().unwrap();
        assert_eq!(latest2.run_id, "run-3");
    }

    #[tokio::test]
    async fn checkpoint_overwrites_previous_messages() {
        let store = InMemoryThreadRunStore::new();
        store
            .checkpoint(
                "t-1",
                &[Message::user("old")],
                &make_run("run-1", "t-1", 100),
            )
            .await
            .unwrap();

        store
            .checkpoint(
                "t-1",
                &[Message::user("new")],
                &make_run("run-2", "t-1", 200),
            )
            .await
            .unwrap();

        let msgs = store.load_messages("t-1").await.unwrap().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text(), "new");
    }

    #[tokio::test]
    async fn load_messages_nonexistent_returns_none() {
        let store = InMemoryThreadRunStore::new();
        let result = store.load_messages("missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn load_run_nonexistent_returns_none() {
        let store = InMemoryThreadRunStore::new();
        let result = store.load_run("missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn latest_run_nonexistent_thread_returns_none() {
        let store = InMemoryThreadRunStore::new();
        let result = store.latest_run("missing").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn checkpoint_preserves_tool_call_messages() {
        let store = InMemoryThreadRunStore::new();
        let tool_call = crate::contract::message::ToolCall::new(
            "call_1",
            "search",
            serde_json::json!({"q": "rust"}),
        );
        let messages = vec![
            Message::user("Find info"),
            Message::assistant_with_tool_calls("Searching...", vec![tool_call]),
            Message::tool("call_1", "Found it"),
            Message::assistant("Here are the results."),
        ];

        store
            .checkpoint("t-1", &messages, &make_run("run-1", "t-1", 100))
            .await
            .unwrap();

        let loaded = store.load_messages("t-1").await.unwrap().unwrap();
        assert_eq!(loaded.len(), 4);

        // Verify tool calls survived
        let tc = loaded[1].tool_calls.as_ref().expect("tool_calls lost");
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].name, "search");

        // Verify tool response
        assert_eq!(loaded[2].tool_call_id.as_deref(), Some("call_1"));
    }

    #[tokio::test]
    async fn concurrent_checkpoints_are_safe() {
        use std::sync::Arc;

        let store = Arc::new(InMemoryThreadRunStore::new());
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    let run = make_run(&format!("run-{i}"), "t-1", i as u64 * 100);
                    store
                        .checkpoint("t-1", &[Message::user(format!("msg-{i}"))], &run)
                        .await
                        .unwrap();
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        // Messages from the last checkpoint (non-deterministic order)
        let msgs = store.load_messages("t-1").await.unwrap().unwrap();
        assert_eq!(msgs.len(), 1);
    }

    #[tokio::test]
    async fn load_run_via_checkpoint() {
        let store = InMemoryThreadRunStore::new();
        let run = make_run("run-1", "t-1", 100);
        store
            .checkpoint("t-1", &[Message::user("m")], &run)
            .await
            .unwrap();

        let loaded = store.load_run("run-1").await.unwrap().unwrap();
        assert_eq!(loaded.run_id, "run-1");
        assert_eq!(loaded.thread_id, "t-1");
    }

    #[tokio::test]
    async fn checkpoint_run_with_tokens_and_steps() {
        let store = InMemoryThreadRunStore::new();
        let mut run = make_run("run-1", "t-1", 100);
        run.input_tokens = 500;
        run.output_tokens = 200;
        run.steps = 3;
        store
            .checkpoint("t-1", &[Message::user("m")], &run)
            .await
            .unwrap();

        let loaded = store.load_run("run-1").await.unwrap().unwrap();
        assert_eq!(loaded.input_tokens, 500);
        assert_eq!(loaded.output_tokens, 200);
        assert_eq!(loaded.steps, 3);
    }

    #[tokio::test]
    async fn checkpoint_run_with_parent_run_id() {
        let store = InMemoryThreadRunStore::new();
        let mut run = make_run("child-run", "t-1", 100);
        run.parent_run_id = Some("parent-run".to_string());
        store
            .checkpoint("t-1", &[Message::user("m")], &run)
            .await
            .unwrap();

        let loaded = store.load_run("child-run").await.unwrap().unwrap();
        assert_eq!(loaded.parent_run_id.as_deref(), Some("parent-run"));
    }

    #[tokio::test]
    async fn checkpoint_run_with_termination_code() {
        let store = InMemoryThreadRunStore::new();
        let mut run = make_run("run-1", "t-1", 100);
        run.status = RunStatus::Done;
        run.termination_code = Some("natural".to_string());
        store
            .checkpoint("t-1", &[Message::user("m")], &run)
            .await
            .unwrap();

        let loaded = store.load_run("run-1").await.unwrap().unwrap();
        assert_eq!(loaded.status, RunStatus::Done);
        assert_eq!(loaded.termination_code.as_deref(), Some("natural"));
    }
}
