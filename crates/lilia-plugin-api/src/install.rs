use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::package::{plain_child, read_regular_file, MAX_LIBRARY_BYTES};
use crate::{verify_package, Capability, PluginApiError, PluginManifest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledPluginSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub target: String,
    pub capabilities: Vec<Capability>,
    pub library: String,
    pub package_path: PathBuf,
}

pub fn install_verified(
    package_dir: &Path,
    install_root: &Path,
    trusted_keys: &[VerifyingKey],
    allow_unsigned: bool,
    force: bool,
) -> Result<PathBuf, PluginApiError> {
    let source_manifest = verify_package(package_dir, trusted_keys, allow_unsigned)?;
    prepare_install_root(install_root)?;
    let staging = tempfile::Builder::new()
        .prefix(".installing-")
        .tempdir_in(install_root)?;
    copy_entry(package_dir, staging.path(), "manifest.json", 64 * 1024)?;
    let signature = package_dir.join("manifest.sig");
    match fs::symlink_metadata(&signature) {
        Ok(_) => copy_entry(package_dir, staging.path(), "manifest.sig", 256)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    copy_entry(
        package_dir,
        staging.path(),
        &source_manifest.library,
        MAX_LIBRARY_BYTES,
    )?;
    let staged_manifest = verify_package(staging.path(), trusted_keys, allow_unsigned)?;
    if staged_manifest.canonical_bytes()? != source_manifest.canonical_bytes()? {
        return Err(PluginApiError::Invalid(
            "plugin package changed during installation".into(),
        ));
    }

    let installed_name = format!(
        "{}-{}-{}",
        staged_manifest.name, staged_manifest.version, staged_manifest.target
    );
    let destination = install_root.join(&installed_name);
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_private_directory(&destination)?;
        }
        Ok(_) if force => {
            let published = destination.join("package");
            if published.exists() {
                fs::remove_dir_all(&published)?;
            }
        }
        Ok(_) => {
            return Err(PluginApiError::Invalid(
                "plugin version is already installed".into(),
            ));
        }
        Err(error) => return Err(error.into()),
    }
    let published = destination.join("package");
    if let Err(error) = fs::rename(staging.path(), &published) {
        let _ = fs::remove_dir(&destination);
        return Err(error.into());
    }
    sync_directory(install_root)?;
    Ok(published)
}

pub fn discover_installed(
    install_root: &Path,
) -> Result<Vec<InstalledPluginSummary>, PluginApiError> {
    match fs::symlink_metadata(install_root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(PluginApiError::Invalid(
                    "plugin root must be a real directory".into(),
                ));
            }
        }
    }
    let mut plugins = Vec::new();
    for entry in fs::read_dir(install_root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let package_path = entry.path().join("package");
        let manifest_path = package_path.join("manifest.json");
        if fs::symlink_metadata(&package_path).is_ok_and(|m| m.file_type().is_dir())
            && fs::symlink_metadata(&manifest_path).is_ok_and(|m| m.file_type().is_file())
        {
            if let Ok(manifest_bytes) = fs::read(&manifest_path) {
                if let Ok(manifest) = serde_json::from_slice::<PluginManifest>(&manifest_bytes) {
                    plugins.push(InstalledPluginSummary {
                        id: name,
                        name: manifest.name,
                        version: manifest.version,
                        target: manifest.target,
                        capabilities: manifest.capabilities,
                        library: manifest.library,
                        package_path,
                    });
                }
            }
        }
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plugins)
}

pub fn find_installed(
    install_root: &Path,
    query: &str,
) -> Result<Option<InstalledPluginSummary>, PluginApiError> {
    let plugins = discover_installed(install_root)?;
    // 1. Exact match on full ID (e.g. lilia-plugin-codec-0.1.0-alpha.1-aarch64-apple-darwin)
    if let Some(p) = plugins.iter().find(|p| p.id == query) {
        return Ok(Some(p.clone()));
    }
    // 2. Exact match on plugin name (e.g. lilia-plugin-codec)
    if let Some(p) = plugins.iter().find(|p| p.name == query) {
        return Ok(Some(p.clone()));
    }
    // 3. Short name without "lilia-plugin-" prefix (e.g. "codec" matches "lilia-plugin-codec")
    let short_query = query.strip_prefix("lilia-plugin-").unwrap_or(query);
    if let Some(p) = plugins
        .iter()
        .find(|p| p.name.strip_prefix("lilia-plugin-").unwrap_or(&p.name) == short_query)
    {
        return Ok(Some(p.clone()));
    }
    // 4. Capability match (e.g. capability "codec", "sql", "json", "kv")
    let query_lower = query.to_lowercase();
    if let Some(p) = plugins.iter().find(|p| {
        p.capabilities.iter().any(|c| {
            let cap_str = format!("{c:?}").to_lowercase();
            cap_str == query_lower
        })
    }) {
        return Ok(Some(p.clone()));
    }
    Ok(None)
}

fn copy_entry(
    source_root: &Path,
    destination_root: &Path,
    filename: &str,
    maximum: u64,
) -> Result<(), PluginApiError> {
    let bytes = read_regular_file(&plain_child(source_root, filename)?, maximum)?;
    let destination = plain_child(destination_root, filename)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    set_private_file(&destination)?;
    Ok(())
}

pub(crate) fn prepare_install_root(path: &Path) -> Result<(), PluginApiError> {
    if !path.exists() {
        create_private_tree(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(PluginApiError::Invalid(
            "plugin root must be a real directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(PluginApiError::Invalid(
                "plugin root must not be accessible by group or others".into(),
            ));
        }
    }
    Ok(())
}

fn create_private_tree(path: &Path) -> Result<(), PluginApiError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or_else(|| {
            PluginApiError::Invalid("plugin root has no existing ancestor".into())
        })?;
    }
    let ancestor = fs::symlink_metadata(cursor)?;
    if !ancestor.file_type().is_dir() || ancestor.file_type().is_symlink() {
        return Err(PluginApiError::Invalid(
            "plugin root ancestor must be a real directory".into(),
        ));
    }
    for directory in missing.into_iter().rev() {
        create_private_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), PluginApiError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
pub(crate) fn set_private_file(path: &Path) -> Result<(), PluginApiError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), PluginApiError> {
    lilia_core::sync_directory(path)?;
    Ok(())
}
