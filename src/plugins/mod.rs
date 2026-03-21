mod descriptor;
mod lifecycle;
mod registry;

pub use descriptor::PluginDescriptor;
pub use lifecycle::Plugin;
pub(crate) use registry::KeyRegistration;
pub use registry::{InstalledPlugin, PluginRegistrar, PluginRegistry};
