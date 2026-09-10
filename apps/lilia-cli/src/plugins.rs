use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use lilia_plugin_api::{
    default_plugin_root, install_verified, trust_store_fingerprint, verify_package, TrustStore,
};
use serde_json::{json, Value};

use crate::args::{Output, PluginCommand, TrustCommand};

pub(crate) fn execute(command: PluginCommand, output: Output) -> anyhow::Result<Value> {
    match command {
        PluginCommand::Verify {
            package,
            trusted_keys,
            allow_unsigned,
            development,
        } => {
            let root = default_plugin_root()?;
            Ok(serde_json::to_value(verify_package(
                &package,
                &verification_keys(Some(&root), &trusted_keys, development)?,
                unsigned_policy(allow_unsigned, development)?,
            )?)?)
        }
        PluginCommand::Trust { command } => trust(command, output),
        PluginCommand::Install {
            package,
            root,
            trusted_keys,
            allow_unsigned,
            development,
        } => {
            let root = selected_root(root, development)?;
            let path = install_verified(
                &package,
                &root,
                &verification_keys(Some(&root), &trusted_keys, development)?,
                unsigned_policy(allow_unsigned, development)?,
            )?;
            Ok(json!({"installed": path}))
        }
        PluginCommand::List { root, development } => {
            Ok(json!(installed_names(&selected_root(root, development)?)?))
        }
        PluginCommand::Remove {
            root,
            installed_name,
            development,
        } => {
            let root = selected_root(root, development)?;
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

fn trust(command: TrustCommand, output: Output) -> anyhow::Result<Value> {
    match command {
        TrustCommand::Add {
            key,
            yes,
            root,
            development,
        } => {
            let key = decode_key(&key)?;
            let fingerprint = trust_store_fingerprint(&key);
            confirm_add(&fingerprint, yes, output)?;
            let store = TrustStore::open(&selected_root(root, development)?)?;
            let added = store.add(&key)?;
            Ok(json!({"fingerprint": fingerprint, "added": added}))
        }
        TrustCommand::List { root, development } => {
            let store = TrustStore::open(&selected_root(root, development)?)?;
            Ok(serde_json::to_value(store.entries()?)?)
        }
        TrustCommand::Remove {
            fingerprint,
            root,
            development,
        } => {
            let store = TrustStore::open(&selected_root(root, development)?)?;
            let removed = store.remove(&fingerprint)?;
            Ok(json!({"fingerprint": fingerprint, "removed": removed}))
        }
    }
}

fn confirm_add(fingerprint: &str, yes: bool, output: Output) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
    if output != Output::Human || !io::stdin().is_terminal() {
        anyhow::bail!("trust add requires --yes in structured or non-interactive mode");
    }
    eprint!("Trust plugin signing key {fingerprint}? [y/N] ");
    io::stderr().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    if !matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        anyhow::bail!("trust key addition cancelled");
    }
    Ok(())
}

fn selected_root(root: Option<PathBuf>, development: bool) -> anyhow::Result<PathBuf> {
    match root {
        Some(_) if !development => anyhow::bail!("--root requires explicit --development mode"),
        Some(root) => Ok(root),
        None => Ok(default_plugin_root()?),
    }
}

fn verification_keys(
    root: Option<&Path>,
    encoded: &[String],
    development: bool,
) -> anyhow::Result<Vec<VerifyingKey>> {
    if !encoded.is_empty() && !development {
        anyhow::bail!("--trusted-key requires explicit --development mode");
    }
    let mut keys = root
        .map(TrustStore::open)
        .transpose()?
        .map(|store| store.keys())
        .transpose()?
        .unwrap_or_default();
    keys.extend(
        encoded
            .iter()
            .map(|value| decode_key(value))
            .collect::<anyhow::Result<Vec<_>>>()?,
    );
    Ok(keys)
}

fn decode_key(encoded: &str) -> anyhow::Result<VerifyingKey> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("trusted keys must be 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).map_err(Into::into)
}

fn installed_names(root: &Path) -> anyhow::Result<Vec<String>> {
    match fs::symlink_metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
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

fn require_real_directory(path: &Path, label: &str) -> anyhow::Result<()> {
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

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use super::require_real_directory;
    use super::{confirm_add, installed_names, selected_root, unsigned_policy, Output};

    #[test]
    fn production_rejects_root_and_unsigned_overrides() {
        assert!(selected_root(Some("custom".into()), false).is_err());
        assert!(unsigned_policy(true, false).is_err());
        assert!(unsigned_policy(true, true).expect("development mode"));
        assert!(!unsigned_policy(false, false).expect("signed policy"));
    }

    #[test]
    fn structured_trust_add_requires_confirmation_flag() {
        assert!(confirm_add("fingerprint", false, Output::Json).is_err());
        assert!(confirm_add("fingerprint", true, Output::Json).is_ok());
    }

    #[test]
    fn listing_ignores_trust_store_and_incomplete_installations() {
        let directory = tempfile::tempdir().expect("temporary root");
        fs::create_dir_all(directory.path().join("valid/package")).expect("valid package");
        fs::write(directory.path().join("valid/package/manifest.json"), b"{}").expect("manifest");
        fs::create_dir(directory.path().join("incomplete")).expect("incomplete install");
        fs::create_dir(directory.path().join(".trust")).expect("trust store");

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
