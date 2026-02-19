//! Tool contracts: execution traits, descriptors, context, and registry.

pub mod context;
pub mod contract;
pub mod registry;

pub use context::{ActivityContext, ToolCallContext};
pub use contract::{Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus};
pub use registry::{ToolRegistry, ToolRegistryError};
