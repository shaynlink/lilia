use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{host_target, PluginManifest, ABI_MAJOR, ABI_MINOR};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 256;
pub(crate) const MAX_LIBRARY_BYTES: u64 = 256 * 1024 * 1024;

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

pub fn verify_package(
    package_dir: &Path,
    trusted_keys: &[VerifyingKey],
    allow_unsigned: bool,
) -> Result<PluginManifest, PluginApiError> {
    let package_dir = regular_directory(package_dir)?;
    let manifest_bytes = read_regular_file(
        &plain_child(&package_dir, "manifest.json")?,
        MAX_MANIFEST_BYTES,
    )?;
    let manifest: PluginManifest = serde_json::from_slice(&manifest_bytes)?;
    validate_manifest(&manifest)?;
    if manifest.canonical_bytes()? != manifest_bytes {
        return Err(PluginApiError::Invalid(
            "manifest.json is not canonical compact JSON".into(),
        ));
    }
    validate_package_entries(&package_dir, &manifest.library)?;
    let library = read_regular_file(
        &plain_child(&package_dir, &manifest.library)?,
        MAX_LIBRARY_BYTES,
    )?;
    let digest = hex::encode(Sha256::digest(&library));
    if digest != manifest.library_sha256 {
        return Err(PluginApiError::Invalid("library checksum mismatch".into()));
    }
    let signature_path = plain_child(&package_dir, "manifest.sig")?;
    match fs::symlink_metadata(&signature_path) {
        Ok(_) => {
            let encoded = read_regular_file(&signature_path, MAX_SIGNATURE_BYTES)?;
            let encoded = std::str::from_utf8(&encoded).map_err(|_| PluginApiError::Signature)?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| PluginApiError::Signature)?;
            let signature = Signature::from_slice(&bytes).map_err(|_| PluginApiError::Signature)?;
            if !trusted_keys
                .iter()
                .any(|key| key.verify_strict(&manifest_bytes, &signature).is_ok())
            {
                return Err(PluginApiError::Signature);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_unsigned => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(PluginApiError::Signature);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(manifest)
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
    Version::parse(&manifest.version)
        .map_err(|_| PluginApiError::Invalid("plugin version is not SemVer".into()))?;
    let requirement = VersionReq::parse(&manifest.engine_requirement)
        .map_err(|_| PluginApiError::Invalid("engine requirement is not a SemVer range".into()))?;
    let engine = Version::parse(env!("CARGO_PKG_VERSION")).expect("workspace version is SemVer");
    if !requirement.matches(&engine) {
        return Err(PluginApiError::Invalid(
            "plugin does not support this engine version".into(),
        ));
    }
    if manifest.abi_major != ABI_MAJOR || manifest.abi_minor > ABI_MINOR {
        return Err(PluginApiError::Invalid("unsupported plugin ABI".into()));
    }
    if host_target() == "unsupported-target" {
        return Err(PluginApiError::Invalid(
            "this distribution target does not support native plugins".into(),
        ));
    }
    if manifest.target != host_target() {
        return Err(PluginApiError::Invalid(
            "plugin target does not match this host".into(),
        ));
    }
    let expected_extension = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    if Path::new(&manifest.library)
        .extension()
        .and_then(|value| value.to_str())
        != Some(expected_extension)
    {
        return Err(PluginApiError::Invalid(
            "plugin library extension does not match the target".into(),
        ));
    }
    plain_relative_filename(&manifest.library)?;
    if manifest.library_sha256.len() != 64
        || !manifest
            .library_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PluginApiError::Invalid(
            "library SHA-256 must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    if manifest.capabilities.is_empty()
        || manifest.capabilities.iter().collect::<HashSet<_>>().len() != manifest.capabilities.len()
    {
        return Err(PluginApiError::Invalid(
            "plugin capabilities must be nonempty and unique".into(),
        ));
    }
    Ok(())
}

fn validate_package_entries(root: &Path, library: &str) -> Result<(), PluginApiError> {
    let expected = ["manifest.json", "manifest.sig", library];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(PluginApiError::Invalid(
                "plugin package contains a non-UTF-8 entry".into(),
            ));
        };
        if !expected.contains(&name) {
            return Err(PluginApiError::Invalid(format!(
                "plugin package contains unexpected entry {name}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn regular_directory(path: &Path) -> Result<PathBuf, PluginApiError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(PluginApiError::Invalid(
            "plugin package must be a real directory".into(),
        ));
    }
    Ok(path.canonicalize()?)
}

pub(crate) fn safe_regular_file(
    root: &Path,
    relative: &str,
    maximum: u64,
) -> Result<PathBuf, PluginApiError> {
    let root = regular_directory(root)?;
    let path = plain_child(&root, relative)?;
    let _ = read_regular_file(&path, maximum)?;
    Ok(path)
}

pub(crate) fn plain_child(root: &Path, relative: &str) -> Result<PathBuf, PluginApiError> {
    plain_relative_filename(relative)?;
    Ok(root.join(relative))
}

fn plain_relative_filename(relative: &str) -> Result<(), PluginApiError> {
    let mut components = Path::new(relative).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(PluginApiError::Invalid(
            "plugin path must be one plain relative filename".into(),
        ));
    }
    Ok(())
}

pub(crate) fn read_regular_file(path: &Path, maximum: u64) -> Result<Vec<u8>, PluginApiError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.len() > maximum {
        return Err(PluginApiError::Invalid(
            "plugin entry must be a bounded regular file".into(),
        ));
    }
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    if !opened.is_file()
        || !current.file_type().is_file()
        || opened.len() > maximum
        || opened.len() != current.len()
        || !same_file(&opened, &current)
    {
        return Err(PluginApiError::Invalid(
            "plugin entry changed or is not a regular file".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(opened.len()).unwrap_or(0));
    file.by_ref().take(maximum + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        return Err(PluginApiError::Invalid(
            "plugin file exceeds size limit".into(),
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn same_file(opened: &fs::Metadata, current: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    opened.dev() == current.dev() && opened.ino() == current.ino()
}

#[cfg(not(unix))]
fn same_file(_opened: &fs::Metadata, _current: &fs::Metadata) -> bool {
    true
}
