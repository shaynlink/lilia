use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::install::{
    create_private_directory, prepare_install_root, set_private_file, sync_directory,
};
use crate::package::read_regular_file;
use crate::PluginApiError;

const MAX_KEY_BYTES: u64 = 128;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrustEntry {
    pub fingerprint: String,
}

#[derive(Debug)]
pub struct TrustStore {
    root: PathBuf,
}

impl TrustStore {
    pub fn open(plugin_root: &Path) -> Result<Self, PluginApiError> {
        prepare_install_root(plugin_root)?;
        let root = plugin_root.join(".trust");
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_private_directory(&root)?;
                sync_directory(plugin_root)?;
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) => validate_store_directory(&metadata)?,
        }
        Ok(Self { root })
    }

    pub fn keys(&self) -> Result<Vec<VerifyingKey>, PluginApiError> {
        let mut keys = Vec::new();
        for path in self.key_paths()? {
            let encoded = read_regular_file(&path, MAX_KEY_BYTES)?;
            let encoded = std::str::from_utf8(&encoded)
                .map_err(|_| PluginApiError::Invalid("trusted key is not UTF-8".into()))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| PluginApiError::Invalid("trusted key is not valid base64".into()))?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| PluginApiError::Invalid("trusted key must be 32 bytes".into()))?;
            let key = VerifyingKey::from_bytes(&bytes)
                .map_err(|_| PluginApiError::Invalid("trusted key is invalid".into()))?;
            if path.file_stem().and_then(|value| value.to_str()) != Some(&fingerprint(&key)) {
                return Err(PluginApiError::Invalid(
                    "trusted key fingerprint does not match its filename".into(),
                ));
            }
            keys.push(key);
        }
        Ok(keys)
    }

    pub fn entries(&self) -> Result<Vec<TrustEntry>, PluginApiError> {
        let mut entries = self
            .keys()?
            .iter()
            .map(|key| TrustEntry {
                fingerprint: fingerprint(key),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
        Ok(entries)
    }

    pub fn add(&self, key: &VerifyingKey) -> Result<bool, PluginApiError> {
        let fingerprint = fingerprint(key);
        let destination = self.root.join(format!("{fingerprint}.pub"));
        if destination.exists() {
            if key_entry_matches(&destination, key)? {
                return Ok(false);
            }
            return Err(PluginApiError::Invalid(
                "trusted key entry conflicts with its fingerprint".into(),
            ));
        }
        let staging = tempfile::Builder::new()
            .prefix(".adding-key-")
            .tempfile_in(&self.root)?;
        set_private_file(staging.path())?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(key.as_bytes());
        let mut file = staging.as_file();
        file.write_all(encoded.as_bytes())?;
        file.sync_all()?;
        match fs::hard_link(staging.path(), &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return key_entry_matches(&destination, key).and_then(|matches| {
                    if matches {
                        Ok(false)
                    } else {
                        Err(PluginApiError::Invalid(
                            "trusted key entry conflicts with its fingerprint".into(),
                        ))
                    }
                });
            }
            Err(error) => return Err(error.into()),
        }
        sync_directory(&self.root)?;
        Ok(true)
    }

    pub fn remove(&self, fingerprint: &str) -> Result<bool, PluginApiError> {
        validate_fingerprint(fingerprint)?;
        let path = self.root.join(format!("{fingerprint}.pub"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(path)?;
                sync_directory(&self.root)?;
                Ok(true)
            }
            Ok(_) => Err(PluginApiError::Invalid(
                "trusted key entry must be a regular file".into(),
            )),
        }
    }

    fn key_paths(&self) -> Result<Vec<PathBuf>, PluginApiError> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || !name.ends_with(".pub") {
                continue;
            }
            validate_fingerprint(name.trim_end_matches(".pub"))?;
            paths.push(entry.path());
        }
        paths.sort();
        Ok(paths)
    }
}

fn validate_store_directory(metadata: &fs::Metadata) -> Result<(), PluginApiError> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(PluginApiError::Invalid(
            "plugin trust store must be a real directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(PluginApiError::Invalid(
                "plugin trust store must not be accessible by group or others".into(),
            ));
        }
    }
    Ok(())
}

pub fn default_plugin_root() -> Result<PathBuf, PluginApiError> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("LiliaDB").join("plugins"));
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library/Application Support/LiliaDB/plugins"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|path| path.join(".local/share"))
        })
        .map(|path| path.join("liliadb/plugins"));
    let root =
        root.ok_or_else(|| PluginApiError::Invalid("user data directory is unavailable".into()))?;
    if !root.is_absolute() {
        return Err(PluginApiError::Invalid(
            "user data directory must be absolute".into(),
        ));
    }
    Ok(root)
}

pub fn fingerprint(key: &VerifyingKey) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

fn validate_fingerprint(value: &str) -> Result<(), PluginApiError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PluginApiError::Invalid(
            "trusted key fingerprint must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn key_entry_matches(path: &Path, key: &VerifyingKey) -> Result<bool, PluginApiError> {
    let existing = read_regular_file(path, MAX_KEY_BYTES)?;
    let expected = base64::engine::general_purpose::STANDARD.encode(key.as_bytes());
    Ok(existing == expected.as_bytes())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;

    #[test]
    fn stores_keys_atomically_without_exposing_material_in_entries() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path().join("plugins");
        let store = TrustStore::open(&root).expect("trust store");
        let key = SigningKey::from_bytes(&[5_u8; 32]).verifying_key();
        let expected = fingerprint(&key);

        assert!(store.add(&key).expect("first add"));
        assert!(!store.add(&key).expect("idempotent add"));
        assert_eq!(store.keys().expect("keys"), [key]);
        assert_eq!(
            store.entries().expect("entries"),
            [TrustEntry {
                fingerprint: expected.clone()
            }]
        );
        assert!(store.remove(&expected).expect("remove"));
        assert!(!store.remove(&expected).expect("idempotent remove"));
    }

    #[test]
    fn rejects_entries_with_mismatched_fingerprints() {
        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path().join("plugins");
        let store = TrustStore::open(&root).expect("trust store");
        let key = SigningKey::from_bytes(&[7_u8; 32]).verifying_key();
        fs::write(
            root.join(".trust").join(format!("{}.pub", "0".repeat(64))),
            base64::engine::general_purpose::STANDARD.encode(key.as_bytes()),
        )
        .expect("malformed entry");

        assert!(store.keys().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn key_entries_are_user_only() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("temporary root");
        let root = directory.path().join("plugins");
        let store = TrustStore::open(&root).expect("trust store");
        let key = SigningKey::from_bytes(&[9_u8; 32]).verifying_key();
        store.add(&key).expect("add key");
        let path = root
            .join(".trust")
            .join(format!("{}.pub", fingerprint(&key)));
        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_public_or_symlinked_store_roots() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let directory = tempfile::tempdir().expect("temporary root");
        let public = directory.path().join("public");
        fs::create_dir(&public).expect("public root");
        fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).expect("permissions");
        assert!(TrustStore::open(&public).is_err());

        let private = directory.path().join("private");
        fs::create_dir(&private).expect("private root");
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).expect("permissions");
        let target = directory.path().join("target");
        fs::create_dir(&target).expect("target");
        symlink(target, private.join(".trust")).expect("symlink");
        assert!(TrustStore::open(&private).is_err());
    }
}
