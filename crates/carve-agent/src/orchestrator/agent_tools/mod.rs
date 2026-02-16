use super::{AgentOs, AgentRegistry};
use crate::contracts::agent_plugin::AgentPlugin;
use crate::contracts::events::AgentEvent;
use crate::contracts::phase::{Phase, StepContext};
use crate::contracts::state_types::{
    AgentRunState, AgentRunStatus, AgentState, Interaction, ToolPermissionBehavior,
    AGENT_RECOVERY_INTERACTION_ACTION, AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_STATE_PATH,
};
use crate::contracts::traits::tool::{Tool, ToolDescriptor, ToolResult, ToolStatus};
use crate::engine::stop_conditions::StopReason;
use crate::engine::tool_filter::{
    is_runtime_allowed, RUNTIME_ALLOWED_AGENTS_KEY, RUNTIME_EXCLUDED_AGENTS_KEY,
};
use crate::extensions::permission::PermissionContextExt;
pub(super) use crate::runtime::loop_runner::TOOL_RUNTIME_CALLER_AGENT_ID_KEY as RUNTIME_CALLER_AGENT_ID_KEY;
use crate::runtime::loop_runner::{
    ChannelStateCommitter, RunCancellationToken, RunContext, TOOL_RUNTIME_CALLER_MESSAGES_KEY,
    TOOL_RUNTIME_CALLER_STATE_KEY, TOOL_RUNTIME_CALLER_THREAD_ID_KEY,
};
use crate::types::{Message, Role, ToolCall};
use async_trait::async_trait;
use carve_state::Context;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

const RUNTIME_CALLER_SESSION_ID_KEY: &str = TOOL_RUNTIME_CALLER_THREAD_ID_KEY;
const RUNTIME_CALLER_STATE_KEY: &str = TOOL_RUNTIME_CALLER_STATE_KEY;
const RUNTIME_CALLER_MESSAGES_KEY: &str = TOOL_RUNTIME_CALLER_MESSAGES_KEY;
const RUNTIME_RUN_ID_KEY: &str = "run_id";
const RUNTIME_PARENT_RUN_ID_KEY: &str = "parent_run_id";

mod manager;
mod plugins;
mod state;
mod tools;

use manager::execute_target_agent;
#[cfg(test)]
use manager::AgentRunCompletion;
pub(super) use manager::{AgentRunManager, AgentRunSummary};
pub(super) use plugins::{AgentRecoveryPlugin, AgentToolsPlugin};
use state::*;
pub(super) use tools::{AgentRunTool, AgentStopTool};

#[cfg(test)]
mod tests;
