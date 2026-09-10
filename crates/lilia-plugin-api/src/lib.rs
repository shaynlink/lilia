mod install;
mod loader;
mod manifest;
mod package;

pub use install::install_verified;
pub use loader::{load_descriptor, LoadedPlugin};
pub use manifest::{
    host_target, Capability, PluginDescriptorV1, PluginManifest, ABI_MAJOR, ABI_MINOR,
};
pub use package::{verify_package, PluginApiError};
