use std::ffi::CStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use libloading::{Library, Symbol};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{PluginDescriptorV1, PluginManifest, ABI_MAJOR, ABI_MINOR};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_LIBRARY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PluginApiError {
    #[error("invalid plugin package: {0}")]
    Invalid(String),
    #[error("plugin I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("plugin signature is invalid")]
    Signature,
    #[error("plugin loading failed: {0}")]
    Loading(#[from] libloading::Error),
}

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

pub fn verify_package(
    package_dir: &Path,
    trusted_keys: &[VerifyingKey],
    allow_unsigned: bool,
) -> Result<PluginManifest, PluginApiError> {
    let canonical_dir = package_dir.canonicalize()?;
    let manifest_path = canonical_dir.join("manifest.json");
    check_file_size(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest: PluginManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    validate_manifest(&manifest)?;
    let library_path = safe_child(&canonical_dir, &manifest.library)?;
    check_file_size(&library_path, MAX_LIBRARY_BYTES)?;
    let digest = hex::encode(Sha256::digest(fs::read(&library_path)?));
    if !digest.eq_ignore_ascii_case(&manifest.library_sha256) {
        return Err(PluginApiError::Invalid("library checksum mismatch".into()));
    }
    let signature_path = canonical_dir.join("manifest.sig");
    if signature_path.exists() {
        let signature_text = fs::read_to_string(signature_path)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(signature_text.trim())
            .map_err(|_| PluginApiError::Signature)?;
        let signature = Signature::from_slice(&bytes).map_err(|_| PluginApiError::Signature)?;
        let payload = manifest.canonical_bytes()?;
        if !trusted_keys
            .iter()
            .any(|key| key.verify_strict(&payload, &signature).is_ok())
        {
            return Err(PluginApiError::Signature);
        }
    } else if !allow_unsigned {
        return Err(PluginApiError::Signature);
    }
    Ok(manifest)
}

pub fn install_verified(
    package_dir: &Path,
    install_root: &Path,
    trusted_keys: &[VerifyingKey],
    allow_unsigned: bool,
) -> Result<PathBuf, PluginApiError> {
    let manifest = verify_package(package_dir, trusted_keys, allow_unsigned)?;
    fs::create_dir_all(install_root)?;
    let destination = install_root.join(format!(
        "{}-{}-{}",
        manifest.name, manifest.version, manifest.target
    ));
    if destination.exists() {
        return Err(PluginApiError::Invalid(
            "plugin version is already installed".into(),
        ));
    }
    let staging = install_root.join(format!(".installing-{}", manifest.name));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir(&staging)?;
    for filename in ["manifest.json", "manifest.sig", manifest.library.as_str()] {
        let source = safe_child(package_dir, filename)?;
        if source.exists() {
            fs::copy(source, staging.join(filename))?;
        }
    }
    fs::rename(&staging, &destination)?;
    Ok(destination)
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
    let library_path = safe_child(package_dir, &manifest.library)?;
    let library = unsafe { Library::new(library_path)? };
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
        .to_string_lossy()
        .into_owned();
    if name != manifest.name {
        return Err(PluginApiError::Invalid(
            "descriptor name does not match manifest".into(),
        ));
    }
    Ok(LoadedPlugin {
        descriptor,
        name,
        _library: library,
    })
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), PluginApiError> {
    if manifest.name.is_empty()
        || !manifest
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(PluginApiError::Invalid(
            "plugin name must be alphanumeric with dashes".into(),
        ));
    }
    if manifest.abi_major != ABI_MAJOR || manifest.abi_minor > ABI_MINOR {
        return Err(PluginApiError::Invalid("unsupported plugin ABI".into()));
    }
    if manifest.target != std::env::consts::ARCH.to_owned() + "-" + std::env::consts::OS {
        return Err(PluginApiError::Invalid(
            "plugin target does not match this host".into(),
        ));
    }
    Ok(())
}

fn safe_child(root: &Path, relative: &str) -> Result<PathBuf, PluginApiError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(PluginApiError::Invalid(
            "plugin path must be a plain relative filename".into(),
        ));
    }
    let child = root.join(path);
    if child.exists() {
        let canonical_root = root.canonicalize()?;
        let canonical_child = child.canonicalize()?;
        if !canonical_child.starts_with(canonical_root) {
            return Err(PluginApiError::Invalid(
                "plugin path escapes package directory".into(),
            ));
        }
        return Ok(canonical_child);
    }
    Ok(child)
}

fn check_file_size(path: &Path, maximum: u64) -> Result<(), PluginApiError> {
    let size = fs::metadata(path)?.len();
    if size > maximum {
        return Err(PluginApiError::Invalid(
            "plugin file exceeds size limit".into(),
        ));
    }
    Ok(())
}
