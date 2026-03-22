//! Background task management for agent tools.
//!
//! Provides a system for spawning, tracking, cancelling, and querying
//! background tasks. Tasks are tracked in-memory and outlive individual runs.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;

use crate::plugins::{Plugin, PluginDescriptor, PluginRegistrar};
use crate::state::{MutationBatch, StateKey, StateKeyOptions};
use awaken_contract::StateError;
use awaken_contract::registry_spec::AgentSpec;

/// Unique identifier for a background task.
pub type TaskId = String;

pub const BACKGROUND_TASKS_PLUGIN_ID: &str = "background_tasks";

// ---------------------------------------------------------------------------
// TaskStatus
// ---------------------------------------------------------------------------

/// Status of a background task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    #[default]
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

// ---------------------------------------------------------------------------
// TaskResult
// ---------------------------------------------------------------------------

/// Result produced by a background task on completion.
#[derive(Debug, Clone)]
pub enum TaskResult {
    Success(serde_json::Value),
    Failed(String),
    Cancelled,
}

impl TaskResult {
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Success(_) => TaskStatus::Completed,
            Self::Failed(_) => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

// ---------------------------------------------------------------------------
// TaskSummary
// ---------------------------------------------------------------------------

/// Summary of a background task visible to tools and plugins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSummary {
    pub task_id: TaskId,
    pub task_type: String,
    pub description: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// BackgroundTaskState (StateKey)
// ---------------------------------------------------------------------------

/// Cached task view stored in the state store for prompt injection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackgroundTaskView {
    pub tasks: HashMap<String, TaskViewEntry>,
}

/// Lightweight view of a single background task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskViewEntry {
    pub task_type: String,
    pub description: String,
    pub status: TaskStatus,
}

/// Action for the background task view state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackgroundTaskViewAction {
    Replace {
        tasks: HashMap<String, TaskViewEntry>,
    },
    Clear,
}

impl BackgroundTaskView {
    fn reduce(&mut self, action: BackgroundTaskViewAction) {
        match action {
            BackgroundTaskViewAction::Replace { tasks } => {
                self.tasks = tasks;
            }
            BackgroundTaskViewAction::Clear => {
                self.tasks.clear();
            }
        }
    }
}

/// State key for the cached background task view.
pub struct BackgroundTaskViewKey;

impl StateKey for BackgroundTaskViewKey {
    const KEY: &'static str = "background_tasks";
    type Value = BackgroundTaskView;
    type Update = BackgroundTaskViewAction;

    fn apply(value: &mut Self::Value, update: Self::Update) {
        value.reduce(update);
    }
}

// ---------------------------------------------------------------------------
// CancellationHandle
// ---------------------------------------------------------------------------

/// Handle for cancelling a running task.
#[derive(Clone)]
pub struct CancellationHandle {
    sender: watch::Sender<bool>,
}

impl CancellationHandle {
    fn new() -> (Self, CancellationToken) {
        let (tx, rx) = watch::channel(false);
        (Self { sender: tx }, CancellationToken { receiver: rx })
    }

    pub fn cancel(&self) {
        let _ = self.sender.send(true);
    }
}

/// Token that a task checks for cancellation.
#[derive(Clone)]
pub struct CancellationToken {
    receiver: watch::Receiver<bool>,
}

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        *self.receiver.borrow()
    }

    pub async fn cancelled(&mut self) {
        while !*self.receiver.borrow() {
            if self.receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// BackgroundTaskManager
// ---------------------------------------------------------------------------

struct LiveTask {
    task_id: TaskId,
    owner_thread_id: String,
    task_type: String,
    description: String,
    status: TaskStatus,
    error: Option<String>,
    result: Option<serde_json::Value>,
    created_at_ms: u64,
    completed_at_ms: Option<u64>,
    cancel_handle: CancellationHandle,
    _join_handle: JoinHandle<()>,
}

/// Thread-scoped handle table for background tasks.
///
/// Spawns, tracks, cancels, and queries background tasks.
pub struct BackgroundTaskManager {
    tasks: Mutex<HashMap<TaskId, LiveTask>>,
    counter: std::sync::atomic::AtomicU64,
}

impl BackgroundTaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn next_task_id(&self) -> TaskId {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("bg_{n}")
    }

    /// Spawn a background task.
    ///
    /// The `task_fn` receives a `CancellationToken` and returns a `TaskResult`.
    pub async fn spawn<F, Fut>(
        self: &Arc<Self>,
        owner_thread_id: &str,
        task_type: &str,
        description: &str,
        task_fn: F,
    ) -> TaskId
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = TaskResult> + Send + 'static,
    {
        let task_id = self.next_task_id();
        let (cancel_handle, cancel_token) = CancellationHandle::new();
        let now = now_ms();

        let manager = Arc::clone(self);
        let tid = task_id.clone();

        let join_handle = tokio::spawn(async move {
            let result = task_fn(cancel_token).await;
            let completed_at = now_ms();
            let mut tasks = manager.tasks.lock().await;
            if let Some(task) = tasks.get_mut(&tid) {
                task.status = result.status();
                task.completed_at_ms = Some(completed_at);
                match &result {
                    TaskResult::Success(val) => {
                        task.result = Some(val.clone());
                    }
                    TaskResult::Failed(err) => {
                        task.error = Some(err.clone());
                    }
                    TaskResult::Cancelled => {}
                }
            }
        });

        let live = LiveTask {
            task_id: task_id.clone(),
            owner_thread_id: owner_thread_id.to_string(),
            task_type: task_type.to_string(),
            description: description.to_string(),
            status: TaskStatus::Running,
            error: None,
            result: None,
            created_at_ms: now,
            completed_at_ms: None,
            cancel_handle,
            _join_handle: join_handle,
        };

        self.tasks.lock().await.insert(task_id.clone(), live);
        task_id
    }

    /// Cancel a running task.
    pub async fn cancel(&self, task_id: &str) -> bool {
        let tasks = self.tasks.lock().await;
        if let Some(task) = tasks.get(task_id)
            && !task.status.is_terminal()
        {
            task.cancel_handle.cancel();
            return true;
        }
        false
    }

    /// List all tasks for a given owner thread.
    pub async fn list(&self, owner_thread_id: &str) -> Vec<TaskSummary> {
        let tasks = self.tasks.lock().await;
        tasks
            .values()
            .filter(|t| t.owner_thread_id == owner_thread_id)
            .map(|t| TaskSummary {
                task_id: t.task_id.clone(),
                task_type: t.task_type.clone(),
                description: t.description.clone(),
                status: t.status,
                error: t.error.clone(),
                result: t.result.clone(),
                created_at_ms: t.created_at_ms,
                completed_at_ms: t.completed_at_ms,
            })
            .collect()
    }

    /// Get the summary of a specific task.
    pub async fn get(&self, task_id: &str) -> Option<TaskSummary> {
        let tasks = self.tasks.lock().await;
        tasks.get(task_id).map(|t| TaskSummary {
            task_id: t.task_id.clone(),
            task_type: t.task_type.clone(),
            description: t.description.clone(),
            status: t.status,
            error: t.error.clone(),
            result: t.result.clone(),
            created_at_ms: t.created_at_ms,
            completed_at_ms: t.completed_at_ms,
        })
    }
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// BackgroundTaskPlugin
// ---------------------------------------------------------------------------

/// Plugin that registers the background task view state key.
pub struct BackgroundTaskPlugin;

impl Plugin for BackgroundTaskPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: BACKGROUND_TASKS_PLUGIN_ID,
        }
    }

    fn register(&self, registrar: &mut PluginRegistrar) -> Result<(), StateError> {
        registrar.register_key::<BackgroundTaskViewKey>(StateKeyOptions::default())?;
        Ok(())
    }

    fn on_activate(
        &self,
        _agent_spec: &AgentSpec,
        _patch: &mut MutationBatch,
    ) -> Result<(), StateError> {
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::StateStore;

    #[test]
    fn task_status_terminal_check() {
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
    }

    #[test]
    fn task_status_as_str() {
        assert_eq!(TaskStatus::Running.as_str(), "running");
        assert_eq!(TaskStatus::Completed.as_str(), "completed");
        assert_eq!(TaskStatus::Failed.as_str(), "failed");
        assert_eq!(TaskStatus::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn task_result_status() {
        assert_eq!(
            TaskResult::Success(serde_json::json!(null)).status(),
            TaskStatus::Completed
        );
        assert_eq!(
            TaskResult::Failed("err".into()).status(),
            TaskStatus::Failed
        );
        assert_eq!(TaskResult::Cancelled.status(), TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn manager_spawn_and_list() {
        let manager = Arc::new(BackgroundTaskManager::new());
        let _id = manager
            .spawn("thread-1", "test", "my task", |mut cancel| async move {
                cancel.cancelled().await;
                TaskResult::Cancelled
            })
            .await;

        let tasks = manager.list("thread-1").await;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_type, "test");
        assert_eq!(tasks[0].description, "my task");
        assert_eq!(tasks[0].status, TaskStatus::Running);

        // Other threads see nothing
        let tasks = manager.list("thread-2").await;
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn manager_task_completes() {
        let manager = Arc::new(BackgroundTaskManager::new());
        let id = manager
            .spawn("thread-1", "test", "fast task", |_| async {
                TaskResult::Success(serde_json::json!({"answer": 42}))
            })
            .await;

        // Wait briefly for task completion
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let summary = manager.get(&id).await.unwrap();
        assert_eq!(summary.status, TaskStatus::Completed);
        assert!(summary.completed_at_ms.is_some());
        assert_eq!(summary.result.unwrap()["answer"], 42);
    }

    #[tokio::test]
    async fn manager_task_fails() {
        let manager = Arc::new(BackgroundTaskManager::new());
        let id = manager
            .spawn("thread-1", "test", "failing task", |_| async {
                TaskResult::Failed("oops".into())
            })
            .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let summary = manager.get(&id).await.unwrap();
        assert_eq!(summary.status, TaskStatus::Failed);
        assert_eq!(summary.error.as_deref(), Some("oops"));
    }

    #[tokio::test]
    async fn manager_cancel() {
        let manager = Arc::new(BackgroundTaskManager::new());
        let id = manager
            .spawn("thread-1", "test", "cancellable", |mut cancel| async move {
                cancel.cancelled().await;
                TaskResult::Cancelled
            })
            .await;

        assert!(manager.cancel(&id).await);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let summary = manager.get(&id).await.unwrap();
        assert_eq!(summary.status, TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn manager_cancel_nonexistent() {
        let manager = Arc::new(BackgroundTaskManager::new());
        assert!(!manager.cancel("nonexistent").await);
    }

    #[test]
    fn plugin_registers_key() {
        let store = StateStore::new();
        store.install_plugin(BackgroundTaskPlugin).unwrap();
        let registry = store.registry.lock().unwrap();
        assert!(registry.keys_by_name.contains_key("background_tasks"));
    }

    #[test]
    fn background_task_view_reduce_replace() {
        let mut view = BackgroundTaskView::default();
        let mut tasks = HashMap::new();
        tasks.insert(
            "t1".into(),
            TaskViewEntry {
                task_type: "shell".into(),
                description: "build".into(),
                status: TaskStatus::Running,
            },
        );
        view.reduce(BackgroundTaskViewAction::Replace { tasks });
        assert_eq!(view.tasks.len(), 1);
        assert_eq!(view.tasks["t1"].task_type, "shell");
    }

    #[test]
    fn background_task_view_reduce_clear() {
        let mut view = BackgroundTaskView {
            tasks: {
                let mut m = HashMap::new();
                m.insert(
                    "t1".into(),
                    TaskViewEntry {
                        task_type: "shell".into(),
                        description: "build".into(),
                        status: TaskStatus::Running,
                    },
                );
                m
            },
        };
        view.reduce(BackgroundTaskViewAction::Clear);
        assert!(view.tasks.is_empty());
    }

    #[test]
    fn cancellation_token_check() {
        let (handle, token) = CancellationHandle::new();
        assert!(!token.is_cancelled());
        handle.cancel();
        assert!(token.is_cancelled());
    }
}
