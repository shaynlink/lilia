use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};

use crate::install::{create_private_directory, set_private_file, sync_directory};
use crate::package::{plain_child, read_regular_file, verify_package, MAX_LIBRARY_BYTES};
use crate::{host_target, Capability, PluginApiError, PluginManifest, ABI_MAJOR, ABI_MINOR};

/// Options for packaging a native plugin.
#[derive(Debug, Clone)]
pub struct PackOptions {
    pub name: String,
    pub version: String,
    pub capabilities: Vec<Capability>,
    pub engine_requirement: Option<String>,
}

/// Generate a cryptographically secure Ed25519 keypair for signing native plugins.
#[must_use]
pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let mut seed = [0u8; 32];
    rand::Rng::fill(&mut rand::rng(), &mut seed);
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

/// Assemble a native plugin package directory containing `manifest.json` and the library.
///
/// # Errors
/// Returns an error if the library cannot be read, the target extension is invalid,
/// or manifest generation fails.
pub fn pack_plugin(
    library_path: &Path,
    out_dir: &Path,
    options: &PackOptions,
) -> Result<PathBuf, PluginApiError> {
    if options.name.is_empty()
        || !options
            .name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(PluginApiError::Invalid(
            "plugin name must be alphanumeric with dashes".into(),
        ));
    }
    Version::parse(&options.version)
        .map_err(|_| PluginApiError::Invalid("plugin version is not SemVer".into()))?;

    let engine_req_str = options.engine_requirement.clone().unwrap_or_else(|| {
        let current_engine = env!("CARGO_PKG_VERSION");
        format!("^{current_engine}")
    });
    VersionReq::parse(&engine_req_str).map_err(|_| {
        PluginApiError::Invalid("engine requirement is not a valid SemVer range".into())
    })?;

    if options.capabilities.is_empty() {
        return Err(PluginApiError::Invalid(
            "plugin capabilities must be non-empty".into(),
        ));
    }

    let library_filename = library_path
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| PluginApiError::Invalid("invalid library filename".into()))?;

    let expected_ext = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    if library_path.extension().and_then(|v| v.to_str()) != Some(expected_ext) {
        return Err(PluginApiError::Invalid(format!(
            "library extension must be '.{expected_ext}' for this target"
        )));
    }

    let library_bytes = read_regular_file(library_path, MAX_LIBRARY_BYTES)?;
    let library_sha256 = hex::encode(Sha256::digest(&library_bytes));

    let manifest = PluginManifest {
        name: options.name.clone(),
        version: options.version.clone(),
        abi_major: ABI_MAJOR,
        abi_minor: ABI_MINOR,
        engine_requirement: engine_req_str,
        target: host_target().to_string(),
        library: library_filename.to_string(),
        library_sha256,
        capabilities: options.capabilities.clone(),
    };

    let canonical_manifest = manifest.canonical_bytes()?;

    if !out_dir.exists() {
        if let Some(parent) = out_dir.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        create_private_directory(out_dir)?;
    }

    // Write manifest.json
    let manifest_dest = plain_child(out_dir, "manifest.json")?;
    let mut manifest_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&manifest_dest)?;
    manifest_file.write_all(&canonical_manifest)?;
    manifest_file.sync_all()?;
    set_private_file(&manifest_dest)?;

    // Copy library
    let library_dest = plain_child(out_dir, library_filename)?;
    let mut lib_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&library_dest)?;
    lib_file.write_all(&library_bytes)?;
    lib_file.sync_all()?;
    set_private_file(&library_dest)?;

    sync_directory(out_dir)?;
    Ok(out_dir.to_path_buf())
}

/// Sign a packaged plugin directory by producing `manifest.sig` over canonical `manifest.json`.
///
/// # Errors
/// Returns an error if `manifest.json` is missing or not canonical, or if signature writing fails.
pub fn sign_package(
    package_dir: &Path,
    signing_key: &SigningKey,
) -> Result<String, PluginApiError> {
    let manifest_path = plain_child(package_dir, "manifest.json")?;
    let manifest_bytes = read_regular_file(&manifest_path, 64 * 1024)?;

    let manifest: PluginManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.canonical_bytes()? != manifest_bytes {
        return Err(PluginApiError::Invalid(
            "manifest.json is not canonical compact JSON".into(),
        ));
    }

    let signature = signing_key.sign(&manifest_bytes);
    let signature_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    let sig_path = plain_child(package_dir, "manifest.sig")?;
    let mut sig_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&sig_path)?;
    sig_file.write_all(signature_b64.as_bytes())?;
    sig_file.sync_all()?;
    set_private_file(&sig_path)?;
    sync_directory(package_dir)?;

    // Verify immediately
    verify_package(package_dir, &[signing_key.verifying_key()], false)?;

    Ok(signature_b64)
}
