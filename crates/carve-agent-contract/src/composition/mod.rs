//! Minimal composition SPI shared outside orchestrator implementations.

mod registry;

pub use registry::{ToolRegistry, ToolRegistryError};
