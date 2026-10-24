mod install;
mod loader;
mod manifest;
mod package;
mod packaging;
mod trust_store;

pub use install::{discover_installed, find_installed, install_verified, InstalledPluginSummary};
pub use loader::{load_descriptor, load_plugin, LoadedPlugin, PluginCodec, PluginSql};
pub use manifest::{
    host_target, platform_library_extension, Capability, CodecDescriptorV1, PluginDescriptorV1,
    PluginManifest, SqlDescriptorV1, ABI_MAJOR, ABI_MINOR,
};
pub use package::{verify_package, PluginApiError};
pub use packaging::{generate_keypair, pack_plugin, sign_package, PackOptions};
pub use trust_store::{
    default_plugin_root, fingerprint as trust_store_fingerprint, official_release_key,
    official_release_signing_key, TrustEntry, TrustStore,
};
