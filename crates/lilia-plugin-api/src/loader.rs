use std::ffi::CStr;
use std::path::Path;

use libloading::{Library, Symbol};

use crate::package::safe_regular_file;
use crate::{PluginApiError, PluginDescriptorV1, PluginManifest, ABI_MAJOR, ABI_MINOR};

pub struct LoadedPlugin {
    pub descriptor: PluginDescriptorV1,
    pub name: String,
    _library: Library,
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LoadedPlugin")
            .field("name", &self.name)
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

/// Load a package only after `verify_package` has accepted it.
///
/// # Safety
/// A valid native plugin still executes with the host process privileges. Callers must only pass
/// a verified, trusted package and must keep the returned `LoadedPlugin` alive while using it.
pub unsafe fn load_descriptor(
    package_dir: &Path,
    manifest: &PluginManifest,
) -> Result<LoadedPlugin, PluginApiError> {
    let library_path = safe_regular_file(package_dir, &manifest.library, 256 * 1024 * 1024)?;
    let library = unsafe { Library::new(library_path)? };
    let entry: Symbol<'_, unsafe extern "C" fn() -> PluginDescriptorV1> =
        unsafe { library.get(b"lilia_plugin_v1\0")? };
    let descriptor = unsafe { entry() };
    let name = validate_descriptor(&descriptor, manifest)?;
    Ok(LoadedPlugin {
        descriptor,
        name,
        _library: library,
    })
}

fn validate_descriptor(
    descriptor: &PluginDescriptorV1,
    manifest: &PluginManifest,
) -> Result<String, PluginApiError> {
    if descriptor.abi_major != ABI_MAJOR
        || descriptor.abi_minor > ABI_MINOR
        || descriptor.name.is_null()
    {
        return Err(PluginApiError::Invalid(
            "plugin descriptor is ABI-incompatible".into(),
        ));
    }
    let name = unsafe { CStr::from_ptr(descriptor.name) }
        .to_str()
        .map_err(|_| PluginApiError::Invalid("plugin descriptor name is not UTF-8".into()))?
        .to_owned();
    if name != manifest.name {
        return Err(PluginApiError::Invalid(
            "descriptor name does not match manifest".into(),
        ));
    }
    let expected = manifest
        .capabilities
        .iter()
        .fold(0, |bits, capability| bits | capability.bit());
    if descriptor.capabilities != expected {
        return Err(PluginApiError::Invalid(
            "descriptor capabilities do not match manifest".into(),
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::ffi::c_char;

    use super::*;
    use crate::{host_target, Capability};

    static NAME: &[u8] = b"trusted-plugin\0";

    fn manifest() -> PluginManifest {
        PluginManifest {
            name: "trusted-plugin".into(),
            version: "1.0.0".into(),
            abi_major: ABI_MAJOR,
            abi_minor: ABI_MINOR,
            engine_requirement: ">=0.1.0-alpha.1,<0.2.0".into(),
            target: host_target().into(),
            library: "plugin.so".into(),
            library_sha256: "0".repeat(64),
            capabilities: vec![Capability::Kv],
        }
    }

    #[test]
    fn rejects_descriptor_capabilities_not_declared_by_manifest() {
        let descriptor = PluginDescriptorV1::new(NAME.as_ptr().cast::<c_char>(), 1 << 1);
        let error =
            validate_descriptor(&descriptor, &manifest()).expect_err("must reject mismatch");
        assert!(error.to_string().contains("capabilities"));
    }

    #[test]
    fn accepts_descriptor_that_exactly_matches_manifest() {
        let descriptor = PluginDescriptorV1::new(NAME.as_ptr().cast::<c_char>(), 1);
        assert_eq!(
            validate_descriptor(&descriptor, &manifest()).expect("valid descriptor"),
            "trusted-plugin"
        );
    }
}
