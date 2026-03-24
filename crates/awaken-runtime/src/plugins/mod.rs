mod descriptor;
mod lifecycle;
mod registry;

pub use descriptor::PluginDescriptor;
pub use lifecycle::{ConfigSchema, Plugin};
pub use registry::{InstalledPlugin, PluginRegistrar, PluginRegistry, ProfileKeyRegistration};
pub(crate) use registry::{KeyRegistration, RequestTransformArc};
