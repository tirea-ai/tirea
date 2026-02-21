//! Tool contracts: execution traits, descriptors, context, and registry.

pub mod context;
pub mod contract;
pub mod registry;

pub use context::{ActivityContext, ToolCallContext};
pub use contract::{
    validate_against_schema, Tool, ToolDescriptor, ToolError, ToolResult, ToolStatus, TypedTool,
};
pub use registry::{ToolRegistry, ToolRegistryError};
