use std::ffi::CStr;
use std::path::Path;

use libloading::{Library, Symbol};

use crate::package::safe_regular_file;
use crate::{
    CodecDescriptorV1, PluginApiError, PluginDescriptorV1, PluginManifest, ABI_MAJOR, ABI_MINOR,
};

pub struct LoadedPlugin {
    pub descriptor: PluginDescriptorV1,
    pub name: String,
    _library: std::sync::Arc<Library>,
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
        _library: std::sync::Arc::new(library),
    })
}

/// Load a plugin package or raw dynamic library.
///
/// # Safety
/// Native plugins execute with host process privileges. Only load trusted code.
pub unsafe fn load_plugin(path: &Path) -> Result<LoadedPlugin, PluginApiError> {
    if path.is_file() {
        let library = unsafe { Library::new(path)? };
        let entry: Symbol<'_, unsafe extern "C" fn() -> PluginDescriptorV1> =
            unsafe { library.get(b"lilia_plugin_v1\0")? };
        let descriptor = unsafe { entry() };
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
        if descriptor.capabilities & crate::Capability::Codec.bit() != 0
            && descriptor.codec.is_null()
        {
            return Err(PluginApiError::Invalid(
                "plugin claims codec capability but codec descriptor is null".into(),
            ));
        }
        return Ok(LoadedPlugin {
            descriptor,
            name,
            _library: std::sync::Arc::new(library),
        });
    }

    let manifest_path = path.join("manifest.json");
    if manifest_path.exists() {
        let manifest_bytes = std::fs::read(&manifest_path)?;
        let manifest: PluginManifest = serde_json::from_slice(&manifest_bytes)?;
        return unsafe { load_descriptor(path, &manifest) };
    }

    Err(PluginApiError::Invalid(format!(
        "path is neither a dynamic library nor a package directory: {}",
        path.display()
    )))
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
    if manifest.capabilities.contains(&crate::Capability::Codec) {
        if descriptor.codec.is_null() {
            return Err(PluginApiError::Invalid(
                "plugin declares codec capability but descriptor.codec is null".into(),
            ));
        }
        let codec = unsafe { &*descriptor.codec };
        if codec.encode.is_none() || codec.decode.is_none() {
            return Err(PluginApiError::Invalid(
                "codec descriptor is missing encode or decode function".into(),
            ));
        }
    }
    Ok(name)
}

impl LoadedPlugin {
    #[must_use]
    pub fn codec(&self) -> Option<PluginCodec> {
        if self.descriptor.codec.is_null() {
            return None;
        }
        let desc = unsafe { *self.descriptor.codec };
        let codec_name = if desc.name.is_null() {
            self.name.clone()
        } else {
            unsafe { CStr::from_ptr(desc.name) }
                .to_str()
                .unwrap_or(&self.name)
                .to_owned()
        };
        Some(PluginCodec {
            name: codec_name,
            descriptor: desc,
            _library: std::sync::Arc::clone(&self._library),
        })
    }
}

#[derive(Clone)]
pub struct PluginCodec {
    name: String,
    descriptor: CodecDescriptorV1,
    _library: std::sync::Arc<Library>,
}

impl PluginCodec {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Debug for PluginCodec {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginCodec")
            .field("name", &self.name)
            .finish()
    }
}

unsafe impl Send for PluginCodec {}
unsafe impl Sync for PluginCodec {}

impl lilia_core::ValueCodec for PluginCodec {
    fn name(&self) -> &str {
        &self.name
    }

    fn encode(&self, plaintext: &[u8]) -> lilia_core::Result<Vec<u8>> {
        let encode_fn = self.descriptor.encode.ok_or_else(|| {
            lilia_core::LiliaError::new(
                lilia_core::ErrorCode::Storage,
                "plugin codec missing encode function",
                false,
            )
        })?;
        let mut out = vec![0u8; plaintext.len() + 64];
        let mut out_len = 0usize;
        let res = unsafe {
            encode_fn(
                plaintext.as_ptr(),
                plaintext.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut out_len,
            )
        };
        if res == 1 {
            out.resize(out_len, 0);
            let res2 = unsafe {
                encode_fn(
                    plaintext.as_ptr(),
                    plaintext.len(),
                    out.as_mut_ptr(),
                    out.len(),
                    &mut out_len,
                )
            };
            if res2 != 0 {
                return Err(lilia_core::LiliaError::new(
                    lilia_core::ErrorCode::Storage,
                    "plugin codec encode failed",
                    false,
                ));
            }
        } else if res != 0 {
            return Err(lilia_core::LiliaError::new(
                lilia_core::ErrorCode::Storage,
                "plugin codec encode failed",
                false,
            ));
        }
        out.truncate(out_len);
        Ok(out)
    }

    fn decode(&self, encoded: &[u8]) -> lilia_core::Result<Vec<u8>> {
        let decode_fn = self.descriptor.decode.ok_or_else(|| {
            lilia_core::LiliaError::new(
                lilia_core::ErrorCode::Storage,
                "plugin codec missing decode function",
                false,
            )
        })?;
        let mut out = vec![0u8; encoded.len()];
        let mut out_len = 0usize;
        let res = unsafe {
            decode_fn(
                encoded.as_ptr(),
                encoded.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut out_len,
            )
        };
        if res == 1 {
            out.resize(out_len, 0);
            let res2 = unsafe {
                decode_fn(
                    encoded.as_ptr(),
                    encoded.len(),
                    out.as_mut_ptr(),
                    out.len(),
                    &mut out_len,
                )
            };
            if res2 != 0 {
                return Err(lilia_core::LiliaError::new(
                    lilia_core::ErrorCode::Corrupt,
                    "plugin codec decode failed or authentication failure",
                    false,
                ));
            }
        } else if res != 0 {
            return Err(lilia_core::LiliaError::new(
                lilia_core::ErrorCode::Corrupt,
                "plugin codec decode failed or authentication failure",
                false,
            ));
        }
        out.truncate(out_len);
        Ok(out)
    }
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

    #[test]
    fn rejects_codec_capability_when_codec_descriptor_is_null() {
        let mut man = manifest();
        man.capabilities = vec![Capability::Codec];
        let descriptor =
            PluginDescriptorV1::new(NAME.as_ptr().cast::<c_char>(), Capability::Codec.bit());
        let error =
            validate_descriptor(&descriptor, &man).expect_err("must reject null codec descriptor");
        assert!(error.to_string().contains("codec"));
    }

    #[test]
    fn accepts_codec_descriptor_when_capability_is_set() {
        unsafe extern "C" fn dummy_codec(
            _input: *const u8,
            _input_len: usize,
            _out: *mut u8,
            _out_cap: usize,
            _out_len: *mut usize,
        ) -> i32 {
            0
        }

        let mut man = manifest();
        man.capabilities = vec![Capability::Codec];
        static CODEC_DESC: CodecDescriptorV1 = CodecDescriptorV1 {
            name: NAME.as_ptr().cast::<c_char>(),
            init: None,
            encode: Some(dummy_codec),
            decode: Some(dummy_codec),
        };
        let descriptor = PluginDescriptorV1::with_codec(
            NAME.as_ptr().cast::<c_char>(),
            Capability::Codec.bit(),
            &CODEC_DESC,
        );
        assert_eq!(
            validate_descriptor(&descriptor, &man).expect("valid descriptor"),
            "trusted-plugin"
        );
    }
}
