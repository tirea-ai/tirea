use super::{AgentOs, AgentRegistry};
use crate::runtime::control::{
    AgentRunState, AgentRunStatus, AGENT_RECOVERY_INTERACTION_ACTION,
    AGENT_RECOVERY_INTERACTION_PREFIX, AGENT_STATE_PATH,
};
use crate::contracts::plugin::AgentPlugin;
use crate::contracts::tool::{Tool, ToolDescriptor, ToolResult, ToolStatus};
use crate::contracts::runtime::phase::{Phase, StepContext};
use crate::contracts::runtime::{AgentEvent, Interaction};
use crate::contracts::state::{Message, Role, ToolCall};
use crate::contracts::AgentState;
use crate::engine::tool_filter::{
    is_scope_allowed, SCOPE_ALLOWED_AGENTS_KEY, SCOPE_EXCLUDED_AGENTS_KEY,
};
use crate::extensions::permission::ToolPermissionBehavior;
use crate::extensions::permission::PermissionContextExt;
pub(super) use crate::runtime::loop_runner::TOOL_SCOPE_CALLER_AGENT_ID_KEY as SCOPE_CALLER_AGENT_ID_KEY;
use crate::runtime::loop_runner::{
    ChannelStateCommitter, RunCancellationToken, RunContext, TOOL_SCOPE_CALLER_MESSAGES_KEY,
    TOOL_SCOPE_CALLER_STATE_KEY, TOOL_SCOPE_CALLER_THREAD_ID_KEY,
};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

const SCOPE_CALLER_SESSION_ID_KEY: &str = TOOL_SCOPE_CALLER_THREAD_ID_KEY;
const SCOPE_CALLER_STATE_KEY: &str = TOOL_SCOPE_CALLER_STATE_KEY;
const SCOPE_CALLER_MESSAGES_KEY: &str = TOOL_SCOPE_CALLER_MESSAGES_KEY;
const SCOPE_RUN_ID_KEY: &str = "run_id";
const SCOPE_PARENT_RUN_ID_KEY: &str = "parent_run_id";
pub(crate) const AGENT_TOOLS_PLUGIN_ID: &str = "agent_tools";
pub(crate) const AGENT_RECOVERY_PLUGIN_ID: &str = "agent_recovery";
pub(crate) const AGENT_RUN_TOOL_ID: &str = "agent_run";
pub(crate) const AGENT_STOP_TOOL_ID: &str = "agent_stop";

fn collect_descendant_run_ids(
    children_by_parent: &HashMap<String, Vec<String>>,
    root_run_id: &str,
    include_root: bool,
) -> Vec<String> {
    let mut queue = VecDeque::from([root_run_id.to_string()]);
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if include_root || id != root_run_id {
            out.push(id.clone());
        }
        if let Some(children) = children_by_parent.get(&id) {
            for child_id in children {
                queue.push_back(child_id.clone());
            }
        }
    }
    out
}

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
