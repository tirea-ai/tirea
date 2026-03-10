//! Background task management for agent tools.
//!
//! Provides a unified system for spawning, tracking, cancelling, and querying
//! background tasks. Tools that implement [`BackgroundExecutable`] can be
//! wrapped with [`BackgroundCapable`] for automatic `run_in_background` support.
//!
//! # Architecture
//!
//! - [`BackgroundTaskManager`] — thread-scoped handle table (spawn, cancel, query)
//! - [`BackgroundExecutable`] — trait for tools that support context-free background execution
//! - [`BackgroundCapable<T>`] — decorator that adds `run_in_background` parameter to a tool
//! - [`TaskStatusTool`] / [`TaskCancelTool`] / [`TaskOutputTool`] — built-in tools for LLM interaction
//! - [`TaskCompletionNotifier`] — pluggable callback for completion delivery (e.g. mailbox)
//!
//! Tasks are thread-scoped and outlive individual runs.

mod manager;
mod plugin;
mod tools;
mod types;
mod wrapper;

pub use manager::{BackgroundTaskManager, TaskCompletionNotifier};
pub use plugin::{BackgroundTasksPlugin, BACKGROUND_TASKS_PLUGIN_ID};
pub use tools::{
    TaskCancelTool, TaskOutputTool, TaskStatusTool, TASK_CANCEL_TOOL_ID, TASK_OUTPUT_TOOL_ID,
    TASK_STATUS_TOOL_ID,
};
pub use types::{
    BackgroundTask, BackgroundTaskAction, BackgroundTaskState, TaskId, TaskResult, TaskStatus,
    TaskSummary,
};
pub use wrapper::{BackgroundCapable, BackgroundExecutable};
