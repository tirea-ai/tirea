//! Sub-agent delegation tools.
//!
//! - `AgentTool`: executes a sub-agent run locally, returns result as tool output.
//! - `RemoteA2aTool`: HTTP call to a remote A2A agent endpoint.

mod agent_tool;
pub(crate) mod remote_a2a;

pub use agent_tool::AgentTool;
pub use remote_a2a::RemoteA2aTool;

#[cfg(test)]
mod tests;
