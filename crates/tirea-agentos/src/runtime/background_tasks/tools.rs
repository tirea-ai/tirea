//! Built-in tools for querying and managing background tasks.
//!
//! These tools provide a unified interface for the LLM to check status,
//! read output, and cancel any background task regardless of type.

use super::manager::BackgroundTaskManager;
use super::types::BackgroundTaskState;
use crate::contracts::runtime::tool_call::{
    Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult,
};
use crate::contracts::storage::ThreadStore;
use crate::contracts::thread::Role;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub const TASK_STATUS_TOOL_ID: &str = "task_status";
pub const TASK_CANCEL_TOOL_ID: &str = "task_cancel";
pub const TASK_OUTPUT_TOOL_ID: &str = "task_output";

fn required_string_for<'a>(
    args: &'a Value,
    key: &str,
    tool_name: &str,
) -> Result<&'a str, ToolResult> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolResult::error(tool_name, format!("Missing required parameter: {key}")))
}

fn owner_thread_id(ctx: &ToolCallContext<'_>) -> Option<String> {
    ctx.run_config()
        .value("__agent_tool_caller_thread_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// task_status
// ---------------------------------------------------------------------------

/// Query background task status and result.
///
/// Supports querying a single task by `task_id` or listing all tasks.
#[derive(Debug, Clone)]
pub struct TaskStatusTool {
    manager: Arc<BackgroundTaskManager>,
}

impl TaskStatusTool {
    pub fn new(manager: Arc<BackgroundTaskManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for TaskStatusTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            TASK_STATUS_TOOL_ID,
            "Task Status",
            "Check the status and result of background tasks. \
             Provide task_id to query a specific task, or omit to list all tasks.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID to query. Omit to list all tasks."
                }
            }
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let Some(thread_id) = owner_thread_id(ctx) else {
            return Ok(ToolResult::error(
                TASK_STATUS_TOOL_ID,
                "Missing caller thread context",
            ));
        };

        let task_id = args.get("task_id").and_then(Value::as_str);

        if let Some(task_id) = task_id {
            // Single task query.
            match self.manager.get(&thread_id, task_id).await {
                Some(summary) => Ok(ToolResult::success(
                    TASK_STATUS_TOOL_ID,
                    serde_json::to_value(&summary).unwrap_or(Value::Null),
                )),
                None => Ok(ToolResult::error(
                    TASK_STATUS_TOOL_ID,
                    format!("Unknown task_id: {task_id}"),
                )),
            }
        } else {
            // List all tasks.
            let tasks = self.manager.list(&thread_id, None).await;
            Ok(ToolResult::success(
                TASK_STATUS_TOOL_ID,
                json!({
                    "tasks": serde_json::to_value(&tasks).unwrap_or(Value::Null),
                    "total": tasks.len(),
                }),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// task_cancel
// ---------------------------------------------------------------------------

/// Cancel a running background task and any descendant tasks.
#[derive(Debug, Clone)]
pub struct TaskCancelTool {
    manager: Arc<BackgroundTaskManager>,
}

impl TaskCancelTool {
    pub fn new(manager: Arc<BackgroundTaskManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for TaskCancelTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            TASK_CANCEL_TOOL_ID,
            "Task Cancel",
            "Cancel a running background task by task_id. \
             Descendant tasks are cancelled automatically.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to cancel"
                }
            },
            "required": ["task_id"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let task_id = match required_string_for(&args, "task_id", TASK_CANCEL_TOOL_ID) {
            Ok(v) => v,
            Err(err) => return Ok(err),
        };

        let Some(thread_id) = owner_thread_id(ctx) else {
            return Ok(ToolResult::error(
                TASK_CANCEL_TOOL_ID,
                "Missing caller thread context",
            ));
        };

        match self.manager.cancel_tree(&thread_id, task_id).await {
            Ok(cancelled) => {
                let ids: Vec<&str> = cancelled.iter().map(|s| s.task_id.as_str()).collect();
                Ok(ToolResult::success(
                    TASK_CANCEL_TOOL_ID,
                    json!({
                        "task_id": task_id,
                        "cancelled": true,
                        "cancelled_ids": ids,
                        "cancelled_count": cancelled.len(),
                    }),
                ))
            }
            Err(e) => Ok(ToolResult::error(TASK_CANCEL_TOOL_ID, e)),
        }
    }
}

// ---------------------------------------------------------------------------
// task_output
// ---------------------------------------------------------------------------

/// Read the output of a background task.
///
/// For `agent_run` tasks, returns the last assistant message from the sub-agent
/// thread. For other task types, returns the task result from the manager.
/// Falls back to persisted `BackgroundTaskState` when the task is not in the
/// in-memory manager (e.g. after process restart).
#[derive(Clone)]
pub struct TaskOutputTool {
    manager: Arc<BackgroundTaskManager>,
    thread_store: Option<Arc<dyn ThreadStore>>,
}

impl std::fmt::Debug for TaskOutputTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskOutputTool")
            .field("has_thread_store", &self.thread_store.is_some())
            .finish()
    }
}

impl TaskOutputTool {
    pub fn new(
        manager: Arc<BackgroundTaskManager>,
        thread_store: Option<Arc<dyn ThreadStore>>,
    ) -> Self {
        Self {
            manager,
            thread_store,
        }
    }

    /// Load the last assistant message from a thread stored in ThreadStore.
    async fn load_agent_output(&self, thread_id: &str) -> Result<Option<String>, String> {
        let Some(store) = &self.thread_store else {
            return Err("no thread store configured".to_string());
        };
        match store.load(thread_id).await {
            Ok(Some(head)) => Ok(head
                .thread
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| m.content.clone())),
            Ok(None) => Ok(None),
            Err(e) => Err(format!("failed to load sub-agent thread: {e}")),
        }
    }
}

#[async_trait]
impl Tool for TaskOutputTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            TASK_OUTPUT_TOOL_ID,
            "Task Output",
            "Read the output of a background task. \
             For agent runs, returns the last assistant message. \
             For other tasks, returns the task result.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID to read output from"
                }
            },
            "required": ["task_id"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let task_id = match required_string_for(&args, "task_id", TASK_OUTPUT_TOOL_ID) {
            Ok(v) => v,
            Err(err) => return Ok(err),
        };

        let Some(thread_id) = owner_thread_id(ctx) else {
            // Fall back to persisted state when no caller thread context.
            return Ok(self.output_from_persisted(task_id, ctx).await);
        };

        // Try live manager first.
        if let Some(summary) = self.manager.get(&thread_id, task_id).await {
            if summary.task_type == "agent_run" {
                let agent_thread_id = summary
                    .metadata
                    .get("thread_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let agent_id = summary
                    .metadata
                    .get("agent_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                match self.load_agent_output(agent_thread_id).await {
                    Ok(output) => {
                        return Ok(ToolResult::success(
                            TASK_OUTPUT_TOOL_ID,
                            json!({
                                "task_id": task_id,
                                "task_type": "agent_run",
                                "agent_id": agent_id,
                                "status": summary.status.as_str(),
                                "output": output,
                            }),
                        ));
                    }
                    Err(e) => {
                        return Ok(ToolResult::error(TASK_OUTPUT_TOOL_ID, e));
                    }
                }
            }
            // Non-agent task: return the result from manager.
            return Ok(ToolResult::success(
                TASK_OUTPUT_TOOL_ID,
                json!({
                    "task_id": task_id,
                    "task_type": summary.task_type,
                    "status": summary.status.as_str(),
                    "output": summary.result,
                }),
            ));
        }

        // Fall back to persisted state.
        Ok(self.output_from_persisted(task_id, ctx).await)
    }
}

impl TaskOutputTool {
    async fn output_from_persisted(&self, task_id: &str, ctx: &ToolCallContext<'_>) -> ToolResult {
        let persisted = ctx
            .state_of::<BackgroundTaskState>()
            .tasks()
            .ok()
            .unwrap_or_default();

        let Some(task) = persisted.get(task_id) else {
            return ToolResult::error(TASK_OUTPUT_TOOL_ID, format!("Unknown task_id: {task_id}"));
        };

        if task.task_type == "agent_run" {
            let agent_id = task
                .metadata
                .get("agent_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let agent_thread_id = task
                .metadata
                .get("thread_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match self.load_agent_output(agent_thread_id).await {
                Ok(output) => ToolResult::success(
                    TASK_OUTPUT_TOOL_ID,
                    json!({
                        "task_id": task_id,
                        "task_type": "agent_run",
                        "agent_id": agent_id,
                        "status": task.status.as_str(),
                        "output": output,
                    }),
                ),
                Err(e) => ToolResult::error(TASK_OUTPUT_TOOL_ID, e),
            }
        } else {
            ToolResult::success(
                TASK_OUTPUT_TOOL_ID,
                json!({
                    "task_id": task_id,
                    "task_type": task.task_type,
                    "status": task.status.as_str(),
                    "output": null,
                }),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::RunConfig;
    use tirea_contract::testing::TestFixture;

    const THREAD_ID_KEY: &str = "__agent_tool_caller_thread_id";

    fn fixture_with_thread(thread_id: &str) -> TestFixture {
        let mut fix = TestFixture::new();
        fix.run_config = {
            let mut rc = RunConfig::new();
            rc.set(THREAD_ID_KEY, thread_id.to_string()).unwrap();
            rc
        };
        fix
    }

    #[test]
    fn task_status_descriptor_has_optional_task_id() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskStatusTool::new(mgr);
        let desc = tool.descriptor();
        assert_eq!(desc.id, TASK_STATUS_TOOL_ID);
        // task_id is not in "required".
        let required = desc.parameters.get("required");
        assert!(required.is_none() || required.unwrap().as_array().unwrap().is_empty());
    }

    #[test]
    fn task_cancel_descriptor_requires_task_id() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskCancelTool::new(mgr);
        let desc = tool.descriptor();
        assert_eq!(desc.id, TASK_CANCEL_TOOL_ID);
        let required = desc.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("task_id")));
        assert!(desc.parameters["properties"].get("tree").is_none());
    }

    // -----------------------------------------------------------------------
    // TaskStatusTool execute() tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn status_tool_missing_thread_context_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskStatusTool::new(mgr);
        let fix = TestFixture::new(); // no __agent_tool_caller_thread_id
        let result = tool.execute(json!({}), &fix.ctx()).await.unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Missing caller thread context"));
    }

    #[tokio::test]
    async fn status_tool_list_all_when_no_tasks() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskStatusTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool.execute(json!({}), &fix.ctx()).await.unwrap();
        assert!(result.is_success());
        let content: Value = result.data.clone();
        assert_eq!(content["total"], 0);
        assert!(content["tasks"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn status_tool_query_single_task() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-1", "shell", "echo hi", |_cancel| async {
                super::super::types::TaskResult::Success(json!({"exit": 0}))
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskStatusTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": tid}), &fix.ctx())
            .await
            .unwrap();
        assert!(result.is_success());
        let content: Value = result.data.clone();
        assert_eq!(content["status"], "completed");
        assert_eq!(content["result"]["exit"], 0);
    }

    #[tokio::test]
    async fn status_tool_query_unknown_task_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskStatusTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": "bogus"}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Unknown task_id"));
    }

    #[tokio::test]
    async fn status_tool_list_shows_running_and_completed() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        // Running task.
        let _running = mgr
            .spawn("thread-1", "shell", "long", |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            })
            .await;
        // Completed task.
        mgr.spawn("thread-1", "http", "fetch", |_| async {
            super::super::types::TaskResult::Success(Value::Null)
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskStatusTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool.execute(json!({}), &fix.ctx()).await.unwrap();
        assert!(result.is_success());
        let content: Value = result.data.clone();
        assert_eq!(content["total"], 2);
    }

    #[tokio::test]
    async fn status_tool_thread_isolation() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-A", "shell", "private", |_| async {
                super::super::types::TaskResult::Success(Value::Null)
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskStatusTool::new(mgr);

        // Thread-B cannot see thread-A's task.
        let fix_b = fixture_with_thread("thread-B");
        let result = tool
            .execute(json!({"task_id": tid}), &fix_b.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());

        // Thread-A can see it.
        let fix_a = fixture_with_thread("thread-A");
        let result = tool
            .execute(json!({"task_id": tid}), &fix_a.ctx())
            .await
            .unwrap();
        assert!(result.is_success());
    }

    // -----------------------------------------------------------------------
    // TaskCancelTool execute() tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cancel_tool_missing_thread_context_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskCancelTool::new(mgr);
        let fix = TestFixture::new();
        let result = tool
            .execute(json!({"task_id": "some"}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Missing caller thread context"));
    }

    #[tokio::test]
    async fn cancel_tool_missing_task_id_param() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskCancelTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool.execute(json!({}), &fix.ctx()).await.unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn cancel_tool_cancels_running_task() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-1", "shell", "long", |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            })
            .await;

        let tool = TaskCancelTool::new(mgr.clone());
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": tid}), &fix.ctx())
            .await
            .unwrap();
        assert!(result.is_success());
        let content: Value = result.data.clone();
        assert!(content["cancelled"].as_bool().unwrap());

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let summary = mgr.get("thread-1", &tid).await.unwrap();
        assert_eq!(summary.status, super::super::types::TaskStatus::Cancelled);
    }

    #[tokio::test]
    async fn cancel_tool_cancels_descendants_by_default() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let root_token = crate::loop_runtime::loop_runner::RunCancellationToken::new();
        let child_token = crate::loop_runtime::loop_runner::RunCancellationToken::new();

        mgr.spawn_with_id(
            "root".to_string(),
            "thread-1",
            "agent_run",
            "agent:root",
            root_token,
            None,
            json!({}),
            |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            },
        )
        .await;

        mgr.spawn_with_id(
            "child".to_string(),
            "thread-1",
            "agent_run",
            "agent:child",
            child_token,
            Some("root"),
            json!({}),
            |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            },
        )
        .await;

        let tool = TaskCancelTool::new(mgr.clone());
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": "root"}), &fix.ctx())
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(result.data["cancelled_count"], 2);
        assert!(result.data["cancelled_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "root"));
        assert!(result.data["cancelled_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "child"));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            mgr.get("thread-1", "root").await.unwrap().status,
            super::super::types::TaskStatus::Cancelled
        );
        assert_eq!(
            mgr.get("thread-1", "child").await.unwrap().status,
            super::super::types::TaskStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn cancel_tool_unknown_task_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskCancelTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": "nope"}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Unknown task_id"));
    }

    #[tokio::test]
    async fn cancel_tool_already_completed_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-1", "shell", "done", |_| async {
                super::super::types::TaskResult::Success(Value::Null)
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskCancelTool::new(mgr);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": tid}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("not running"));
    }

    #[tokio::test]
    async fn cancel_tool_wrong_owner_rejected() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-1", "shell", "private", |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            })
            .await;

        let tool = TaskCancelTool::new(mgr);
        let fix = fixture_with_thread("thread-2");
        let result = tool
            .execute(json!({"task_id": tid}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
    }

    // -----------------------------------------------------------------------
    // TaskOutputTool execute() tests
    // -----------------------------------------------------------------------

    #[test]
    fn output_tool_descriptor_requires_task_id() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskOutputTool::new(mgr, None);
        let desc = tool.descriptor();
        assert_eq!(desc.id, TASK_OUTPUT_TOOL_ID);
        let required = desc.parameters["required"].as_array().unwrap();
        assert!(required.contains(&json!("task_id")));
    }

    #[tokio::test]
    async fn output_tool_missing_task_id_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskOutputTool::new(mgr, None);
        let fix = fixture_with_thread("thread-1");
        let result = tool.execute(json!({}), &fix.ctx()).await.unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn output_tool_unknown_task_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskOutputTool::new(mgr, None);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": "nonexistent"}), &fix.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("Unknown task_id"));
    }

    #[tokio::test]
    async fn output_tool_returns_result_for_non_agent_task() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-1", "shell", "echo hi", |_| async {
                super::super::types::TaskResult::Success(json!({"exit_code": 0, "stdout": "hi"}))
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskOutputTool::new(mgr, None);
        let fix = fixture_with_thread("thread-1");
        let result = tool
            .execute(json!({"task_id": tid}), &fix.ctx())
            .await
            .unwrap();
        assert!(result.is_success());
        assert_eq!(result.data["task_type"], "shell");
        assert_eq!(result.data["status"], "completed");
        assert_eq!(result.data["output"]["exit_code"], 0);
        assert_eq!(result.data["output"]["stdout"], "hi");
    }

    #[tokio::test]
    async fn output_tool_falls_back_to_persisted_state() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskOutputTool::new(mgr, None);

        let doc = json!({
            "background_tasks": {
                "tasks": {
                    "run-1": {
                        "task_type": "shell",
                        "description": "echo test",
                        "status": "completed",
                        "created_at_ms": 0,
                        "metadata": {}
                    }
                }
            }
        });
        let fix = TestFixture::new_with_state(doc);
        let result = tool
            .execute(
                json!({"task_id": "run-1"}),
                &fix.ctx_with("call-1", "tool:task_output"),
            )
            .await
            .unwrap();
        assert!(result.is_success());
        assert_eq!(result.data["task_type"], "shell");
        assert_eq!(result.data["status"], "completed");
    }

    #[tokio::test]
    async fn output_tool_persisted_agent_run_without_store_returns_error() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tool = TaskOutputTool::new(mgr, None);

        let doc = json!({
            "background_tasks": {
                "tasks": {
                    "run-1": {
                        "task_type": "agent_run",
                        "description": "agent:worker",
                        "status": "completed",
                        "created_at_ms": 0,
                        "metadata": {
                            "thread_id": "sub-agent-run-1",
                            "agent_id": "worker"
                        }
                    }
                }
            }
        });
        let fix = TestFixture::new_with_state(doc);
        let result = tool
            .execute(
                json!({"task_id": "run-1"}),
                &fix.ctx_with("call-1", "tool:task_output"),
            )
            .await
            .unwrap();
        // No ThreadStore configured → error.
        assert!(!result.is_success());
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("no thread store configured"));
    }

    #[tokio::test]
    async fn output_tool_thread_isolation() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let tid = mgr
            .spawn("thread-A", "shell", "private", |_| async {
                super::super::types::TaskResult::Success(json!("secret"))
            })
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let tool = TaskOutputTool::new(mgr, None);

        // Thread-B cannot see thread-A's task.
        let fix_b = fixture_with_thread("thread-B");
        let result = tool
            .execute(json!({"task_id": tid}), &fix_b.ctx())
            .await
            .unwrap();
        assert!(!result.is_success());

        // Thread-A can see it.
        let fix_a = fixture_with_thread("thread-A");
        let result = tool
            .execute(json!({"task_id": tid}), &fix_a.ctx())
            .await
            .unwrap();
        assert!(result.is_success());
    }
}
