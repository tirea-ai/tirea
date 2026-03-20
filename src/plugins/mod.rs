mod lifecycle;
mod registry;

pub use lifecycle::{PluginMeta, StatePlugin};
pub use registry::{InstalledPlugin, PluginRegistrar, PluginRegistry};
