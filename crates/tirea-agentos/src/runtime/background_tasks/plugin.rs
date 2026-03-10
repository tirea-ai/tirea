//! `BackgroundTasksPlugin` — behavior that registers background task state
//! and provides running-task context reminders to the LLM.

use super::manager::BackgroundTaskManager;
use crate::contracts::runtime::behavior::{AgentBehavior, ReadOnlyContext};
use crate::contracts::runtime::phase::{ActionSet, AfterToolExecuteAction};
use async_trait::async_trait;
use std::sync::Arc;

pub const BACKGROUND_TASKS_PLUGIN_ID: &str = "background_tasks";

/// Behavior that injects running-task reminders into the context after tool
/// execution.
///
/// All background tasks (shell, agent_run, etc.) are rendered uniformly.
pub struct BackgroundTasksPlugin {
    manager: Arc<BackgroundTaskManager>,
}

impl BackgroundTasksPlugin {
    pub fn new(manager: Arc<BackgroundTaskManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl AgentBehavior for BackgroundTasksPlugin {
    fn id(&self) -> &str {
        BACKGROUND_TASKS_PLUGIN_ID
    }

    async fn after_tool_execute(
        &self,
        ctx: &ReadOnlyContext<'_>,
    ) -> ActionSet<AfterToolExecuteAction> {
        let thread_id = ctx
            .run_config()
            .value("__agent_tool_caller_thread_id")
            .and_then(|v| v.as_str())
            .unwrap_or(ctx.thread_id());

        let tasks = self.manager.list(thread_id, None).await;
        let active: Vec<_> = tasks.iter().filter(|t| !t.status.is_terminal()).collect();

        if active.is_empty() {
            return ActionSet::empty();
        }

        let mut s = String::new();
        s.push_str("<background_tasks>\n");
        for t in &active {
            s.push_str(&format!(
                "<task id=\"{}\" type=\"{}\" status=\"{}\" description=\"{}\"/>\n",
                t.task_id,
                t.task_type,
                t.status.as_str(),
                t.description,
            ));
        }
        s.push_str("</background_tasks>\n");
        s.push_str(
            "Use tool \"task_status\" to check progress, \"task_output\" to read results, or \"task_cancel\" to cancel active tasks.",
        );

        ActionSet::single(AfterToolExecuteAction::AddSystemReminder(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::RunConfig;
    use serde_json::{json, Value};
    use tirea_contract::runtime::phase::Phase;
    use tirea_state::DocCell;

    const THREAD_ID_KEY: &str = "__agent_tool_caller_thread_id";

    fn make_ctx_with_thread<'a>(
        thread_id: &'a str,
        run_config: &'a RunConfig,
        doc: &'a DocCell,
    ) -> ReadOnlyContext<'a> {
        ReadOnlyContext::new(Phase::AfterToolExecute, thread_id, &[], run_config, doc)
    }

    #[test]
    fn plugin_id_is_background_tasks() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let plugin = BackgroundTasksPlugin::new(mgr);
        assert_eq!(plugin.id(), BACKGROUND_TASKS_PLUGIN_ID);
    }

    #[test]
    fn plugin_registers_lattice_and_scope() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let plugin = BackgroundTasksPlugin::new(mgr);

        let mut lattice = tirea_state::LatticeRegistry::new();
        plugin.register_lattice_paths(&mut lattice);

        let mut scope_reg = tirea_contract::runtime::state::StateScopeRegistry::new();
        plugin.register_state_scopes(&mut scope_reg);
        // Verify no panic even though this plugin no longer registers thread state.

        let mut action_reg = tirea_contract::runtime::state::ActionDeserializerRegistry::new();
        plugin.register_action_deserializers(&mut action_reg);
        // Verify no panic.
    }

    #[tokio::test]
    async fn after_tool_execute_empty_when_no_tasks() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        let plugin = BackgroundTasksPlugin::new(mgr);

        let doc = DocCell::new(json!({}));
        let mut rc = RunConfig::new();
        rc.set(THREAD_ID_KEY, "thread-1".to_string()).unwrap();
        let ctx = make_ctx_with_thread("thread-1", &rc, &doc);

        let actions = plugin.after_tool_execute(&ctx).await;
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn after_tool_execute_shows_running_tasks() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        // Spawn a running task.
        mgr.spawn(
            "thread-1",
            "shell",
            "building project",
            |cancel| async move {
                cancel.cancelled().await;
                super::super::types::TaskResult::Cancelled
            },
        )
        .await;

        let plugin = BackgroundTasksPlugin::new(mgr);
        let doc = DocCell::new(json!({}));
        let mut rc = RunConfig::new();
        rc.set(THREAD_ID_KEY, "thread-1".to_string()).unwrap();
        let ctx = make_ctx_with_thread("thread-1", &rc, &doc);

        let actions = plugin.after_tool_execute(&ctx).await;
        assert_eq!(actions.len(), 1);

        let reminder = match actions.into_iter().next().unwrap() {
            AfterToolExecuteAction::AddSystemReminder(s) => s,
            _ => panic!("unexpected action variant"),
        };
        assert!(reminder.contains("<background_tasks>"));
        assert!(reminder.contains("building project"));
        assert!(reminder.contains("task_status"));
        assert!(reminder.contains("task_output"));
    }

    #[tokio::test]
    async fn after_tool_execute_ignores_completed_tasks() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        // Spawn a task that completes immediately.
        mgr.spawn("thread-1", "http", "fetch data", |_| async {
            super::super::types::TaskResult::Success(Value::Null)
        })
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let plugin = BackgroundTasksPlugin::new(mgr);
        let doc = DocCell::new(json!({}));
        let mut rc = RunConfig::new();
        rc.set(THREAD_ID_KEY, "thread-1".to_string()).unwrap();
        let ctx = make_ctx_with_thread("thread-1", &rc, &doc);

        let actions = plugin.after_tool_execute(&ctx).await;
        assert!(
            actions.is_empty(),
            "completed tasks should not trigger reminder"
        );
    }

    #[tokio::test]
    async fn after_tool_execute_fallback_to_thread_id_without_config_key() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        mgr.spawn("fallback-thread", "shell", "task A", |cancel| async move {
            cancel.cancelled().await;
            super::super::types::TaskResult::Cancelled
        })
        .await;

        let plugin = BackgroundTasksPlugin::new(mgr);
        let doc = DocCell::new(json!({}));
        let rc = RunConfig::new(); // no __agent_tool_caller_thread_id
                                   // thread_id parameter IS "fallback-thread" — used as fallback.
        let ctx = make_ctx_with_thread("fallback-thread", &rc, &doc);

        let actions = plugin.after_tool_execute(&ctx).await;
        assert_eq!(actions.len(), 1);
    }

    #[tokio::test]
    async fn after_tool_execute_thread_isolation() {
        let mgr = Arc::new(BackgroundTaskManager::new());
        mgr.spawn("thread-A", "shell", "private task", |cancel| async move {
            cancel.cancelled().await;
            super::super::types::TaskResult::Cancelled
        })
        .await;

        let plugin = BackgroundTasksPlugin::new(mgr);
        let doc = DocCell::new(json!({}));

        // Thread-B should see nothing.
        let mut rc_b = RunConfig::new();
        rc_b.set(THREAD_ID_KEY, "thread-B".to_string()).unwrap();
        let ctx_b = make_ctx_with_thread("thread-B", &rc_b, &doc);
        let actions = plugin.after_tool_execute(&ctx_b).await;
        assert!(actions.is_empty());

        // Thread-A sees the reminder.
        let mut rc_a = RunConfig::new();
        rc_a.set(THREAD_ID_KEY, "thread-A".to_string()).unwrap();
        let ctx_a = make_ctx_with_thread("thread-A", &rc_a, &doc);
        let actions = plugin.after_tool_execute(&ctx_a).await;
        assert_eq!(actions.len(), 1);
    }
}
