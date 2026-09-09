mod loader;
mod manifest;

pub use loader::{install_verified, load_descriptor, verify_package, LoadedPlugin, PluginApiError};
pub use manifest::{Capability, PluginDescriptorV1, PluginManifest, ABI_MAJOR, ABI_MINOR};
