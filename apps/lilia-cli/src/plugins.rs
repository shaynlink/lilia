use std::fs;

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use lilia_plugin_api::{install_verified, verify_package};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::args::{PluginCommand, TrustCommand};

pub(crate) fn execute(command: PluginCommand) -> anyhow::Result<Value> {
    match command {
        PluginCommand::Verify {
            package,
            trusted_keys,
            allow_unsigned,
            development,
        } => Ok(serde_json::to_value(verify_package(
            &package,
            &decode_keys(&trusted_keys)?,
            unsigned_policy(allow_unsigned, development)?,
        )?)?),
        PluginCommand::Trust { command } => trust(command),
        PluginCommand::Install {
            package,
            root,
            trusted_keys,
            allow_unsigned,
            development,
        } => {
            let path = install_verified(
                &package,
                &root,
                &decode_keys(&trusted_keys)?,
                unsigned_policy(allow_unsigned, development)?,
            )?;
            Ok(json!({"installed": path}))
        }
        PluginCommand::List { root } => Ok(json!(installed_names(&root)?)),
        PluginCommand::Remove {
            root,
            installed_name,
        } => {
            if installed_name.contains('/')
                || installed_name.contains('\\')
                || installed_name.starts_with('.')
            {
                anyhow::bail!("invalid installed plugin name");
            }
            let path = root.join(installed_name);
            require_real_directory(&root, "plugin root")?;
            require_real_directory(&path, "installed plugin")?;
            require_real_directory(&path.join("package"), "installed plugin package")?;
            fs::remove_dir_all(&path)?;
            Ok(json!({"removed": path}))
        }
    }
}

fn installed_names(root: &std::path::Path) -> anyhow::Result<Vec<String>> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
        Ok(_) => require_real_directory(root, "plugin root")?,
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let package = entry.path().join("package");
        let manifest = package.join("manifest.json");
        if fs::symlink_metadata(package).is_ok_and(|metadata| metadata.file_type().is_dir())
            && fs::symlink_metadata(manifest).is_ok_and(|metadata| metadata.file_type().is_file())
        {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names)
}

fn require_real_directory(path: &std::path::Path, label: &str) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("{label} must be a real directory");
    }
    Ok(())
}

fn unsigned_policy(allow_unsigned: bool, development: bool) -> anyhow::Result<bool> {
    if allow_unsigned && !development {
        anyhow::bail!("--allow-unsigned requires explicit --development mode");
    }
    Ok(allow_unsigned)
}

fn trust(command: TrustCommand) -> anyhow::Result<Value> {
    match command {
        TrustCommand::Add { root, key } => {
            let decoded = decode_keys(std::slice::from_ref(&key))?;
            let fingerprint = hex::encode(Sha256::digest(decoded[0].as_bytes()));
            fs::create_dir_all(&root)?;
            let path = root.join("trust-keys.json");
            let mut keys: Vec<String> = if path.exists() {
                serde_json::from_slice(&fs::read(&path)?)?
            } else {
                Vec::new()
            };
            if !keys.contains(&key) {
                keys.push(key);
            }
            fs::write(&path, serde_json::to_vec_pretty(&keys)?)?;
            Ok(json!({"added": fingerprint}))
        }
        TrustCommand::List { root } => {
            let path = root.join("trust-keys.json");
            let keys: Vec<String> = if path.exists() {
                serde_json::from_slice(&fs::read(path)?)?
            } else {
                Vec::new()
            };
            Ok(json!(keys
                .into_iter()
                .map(|key| {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&key)
                        .unwrap_or_default();
                    json!({"fingerprint": hex::encode(Sha256::digest(bytes)), "key": key})
                })
                .collect::<Vec<_>>()))
        }
        TrustCommand::Remove { root, fingerprint } => {
            let path = root.join("trust-keys.json");
            let mut keys: Vec<String> = serde_json::from_slice(&fs::read(&path)?)?;
            keys.retain(|key| {
                hex::encode(Sha256::digest(
                    base64::engine::general_purpose::STANDARD
                        .decode(key)
                        .unwrap_or_default(),
                )) != fingerprint
            });
            fs::write(&path, serde_json::to_vec_pretty(&keys)?)?;
            Ok(json!({"removed": fingerprint}))
        }
    }
}

fn decode_keys(encoded: &[String]) -> anyhow::Result<Vec<VerifyingKey>> {
    encoded
        .iter()
        .map(|value| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("trusted keys must be 32 bytes"))?;
            VerifyingKey::from_bytes(&bytes).map_err(Into::into)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{installed_names, require_real_directory, unsigned_policy};

    #[test]
    fn unsigned_policy_requires_explicit_development_mode() {
        assert!(unsigned_policy(true, false).is_err());
        assert!(unsigned_policy(true, true).expect("development mode"));
        assert!(!unsigned_policy(false, false).expect("signed policy"));
    }

    #[test]
    fn listing_ignores_incomplete_and_hidden_installations() {
        let directory = tempfile::tempdir().expect("temporary root");
        fs::create_dir_all(directory.path().join("valid/package")).expect("valid package");
        fs::write(directory.path().join("valid/package/manifest.json"), b"{}").expect("manifest");
        fs::create_dir(directory.path().join("incomplete")).expect("incomplete install");
        fs::create_dir(directory.path().join(".installing-stale")).expect("staging");
        fs::write(directory.path().join("plain-file"), b"value").expect("plain file");

        assert_eq!(
            installed_names(directory.path()).expect("installed names"),
            ["valid"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_directory_check_rejects_symlinks() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary root");
        let target = directory.path().join("target");
        let link = directory.path().join("link");
        fs::create_dir(&target).expect("target");
        symlink(target, &link).expect("link");
        require_real_directory(&link, "plugin").expect_err("symlink must fail");
    }
}
