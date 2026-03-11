use super::policy::is_scope_allowed;
use super::AgentOs;
use crate::composition::AgentRegistry;
use crate::contracts::runtime::tool_call::{ToolError, ToolResult};
use crate::contracts::thread::{Message, Role, ToolCall};
use crate::contracts::{AgentEvent, Suspension};
#[cfg(feature = "permission")]
use tirea_extension_permission::ToolPermissionBehavior;

#[cfg(not(feature = "permission"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ToolPermissionBehavior {
    Allow,
    Ask,
    Deny,
}
use crate::runtime::loop_runner::RunCancellationToken;
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;
use types::SubAgentStatus;

pub(crate) const AGENT_TOOLS_PLUGIN_ID: &str = "agent_tools";
pub(crate) const AGENT_RECOVERY_PLUGIN_ID: &str = "agent_recovery";
pub(crate) const AGENT_RUN_TOOL_ID: &str = "agent_run";
pub(crate) const AGENT_RECOVERY_INTERACTION_ACTION: &str = "recover_agent_run";
pub(crate) const AGENT_RECOVERY_INTERACTION_PREFIX: &str = "agent_recovery_";

mod manager;
mod plugins;
mod state;
mod tools;
mod types;

use manager::{execute_sub_agent, SubAgentExecutionRequest};
pub(super) use plugins::{AgentRecoveryPlugin, AgentToolsPlugin};
use state::{build_recovery_interaction, has_suspended_recovery_interaction, recovery_target_id};
pub(super) use tools::AgentRunTool;

#[cfg(test)]
mod tests;
