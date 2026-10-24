use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use lilia_plugin_api::{
    default_plugin_root, discover_installed, generate_keypair, install_verified,
    official_release_key, pack_plugin, sign_package, trust_store_fingerprint, verify_package,
    PackOptions, TrustStore,
};
use serde_json::{json, Value};

use crate::args::{Output, PluginCommand, TrustCommand};

#[allow(clippy::too_many_lines)]
pub(crate) fn execute(command: PluginCommand, output: Output) -> anyhow::Result<Value> {
    match command {
        PluginCommand::Keygen {
            out_dir,
            name,
            force,
        } => {
            let (signing_key, verifying_key) = generate_keypair();
            if !out_dir.exists() {
                fs::create_dir_all(&out_dir)?;
            }
            let key_path = out_dir.join(format!("{name}.key"));
            let pub_path = out_dir.join(format!("{name}.pub"));

            if !force && (key_path.exists() || pub_path.exists()) {
                anyhow::bail!("key files already exist, use --force to overwrite");
            }

            let sk_b64 = base64::engine::general_purpose::STANDARD.encode(signing_key.to_bytes());
            let vk_b64 = base64::engine::general_purpose::STANDARD.encode(verifying_key.as_bytes());
            let fingerprint = trust_store_fingerprint(&verifying_key);

            let mut key_file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&key_path)?;
            key_file.write_all(sk_b64.as_bytes())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
            }

            let mut pub_file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&pub_path)?;
            pub_file.write_all(vk_b64.as_bytes())?;

            Ok(json!({
                "fingerprint": fingerprint,
                "public_key": vk_b64,
                "private_key_path": key_path,
                "public_key_path": pub_path
            }))
        }
        PluginCommand::Pack {
            library,
            name,
            version,
            capabilities,
            engine_requirement,
            out_dir,
        } => {
            let default_dir = PathBuf::from(format!("./dist/{name}-{version}-package"));
            let target_out = out_dir.unwrap_or(default_dir);
            let pack_opts = PackOptions {
                name: name.clone(),
                version: version.clone(),
                capabilities: capabilities.into_iter().map(Into::into).collect(),
                engine_requirement,
            };
            let packed = pack_plugin(&library, &target_out, &pack_opts)?;
            Ok(json!({
                "packed": packed,
                "name": name,
                "version": version
            }))
        }
        PluginCommand::Sign { package, key } => {
            let key_bytes = if Path::new(&key).is_file() {
                let content = fs::read_to_string(&key)?;
                base64::engine::general_purpose::STANDARD.decode(content.trim())?
            } else {
                base64::engine::general_purpose::STANDARD.decode(&key)?
            };
            let key_slice: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("private signing key must be 32 bytes"))?;
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_slice);
            let signature = sign_package(&package, &signing_key)?;
            let verifying_key = signing_key.verifying_key();
            let fingerprint = trust_store_fingerprint(&verifying_key);
            Ok(json!({
                "signed": true,
                "signature": signature,
                "fingerprint": fingerprint,
                "package": package
            }))
        }
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
            force,
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
                force,
            )?;
            Ok(json!({ "installed": path }))
        }
        PluginCommand::List { root, development } => {
            let root = selected_root(root, development)?;
            let plugins = discover_installed(&root)?;
            Ok(json!(plugins))
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
            Ok(json!({ "removed": path }))
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
    let mut keys = vec![official_release_key()];
    if let Some(store) = root.map(TrustStore::open).transpose()? {
        for key in store.keys()? {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
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
    use super::{confirm_add, selected_root, unsigned_policy, Output};

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
        let manifest = serde_json::json!({
            "name": "valid",
            "version": "1.0.0",
            "abi_major": 1,
            "abi_minor": 0,
            "engine_requirement": "^0.1.0",
            "target": "any",
            "library": "lib.so",
            "library_sha256": "abc",
            "capabilities": ["codec"]
        });
        fs::write(
            directory.path().join("valid/package/manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .expect("manifest");
        fs::create_dir(directory.path().join("incomplete")).expect("incomplete install");
        fs::create_dir(directory.path().join(".trust")).expect("trust store");

        let discovered = super::discover_installed(directory.path()).expect("discover installed");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id, "valid");
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

    #[test]
    fn plugin_cli_pack_sign_install_lifecycle() {
        use crate::args::{CapabilityArg, PluginCommand};

        let temp = tempfile::tempdir().expect("tempdir");
        let keys_dir = temp.path().join("keys");
        let plugin_root = temp.path().join("installed");

        // 1. Keygen
        let keygen_val = super::execute(
            PluginCommand::Keygen {
                out_dir: keys_dir.clone(),
                name: "test-author".into(),
                force: false,
            },
            Output::Json,
        )
        .expect("keygen succeeds");
        let priv_key_path = keys_dir.join("test-author.key");
        let pub_key = keygen_val["public_key"].as_str().expect("pubkey");
        assert!(priv_key_path.is_file());

        // 2. Dummy library file to pack
        let lib_ext = lilia_plugin_api::platform_library_extension();
        let lib_path = temp.path().join(format!("dummy.{lib_ext}"));
        fs::write(&lib_path, b"mock binary content").expect("write mock lib");

        let pack_out = temp.path().join("packed-pkg");
        let pack_val = super::execute(
            PluginCommand::Pack {
                library: lib_path,
                name: "lilia-plugin-custom".into(),
                version: "1.0.0".into(),
                capabilities: vec![CapabilityArg::Codec],
                engine_requirement: None,
                out_dir: Some(pack_out.clone()),
            },
            Output::Json,
        )
        .expect("pack succeeds");
        assert_eq!(pack_val["name"], "lilia-plugin-custom");
        assert!(pack_out.join("manifest.json").is_file());

        // 3. Sign
        let sign_val = super::execute(
            PluginCommand::Sign {
                package: pack_out.clone(),
                key: priv_key_path.to_str().unwrap().into(),
            },
            Output::Json,
        )
        .expect("sign succeeds");
        assert_eq!(sign_val["signed"], true);
        assert!(pack_out.join("manifest.sig").is_file());

        // 4. Install with custom key in dev mode
        let install_val = super::execute(
            PluginCommand::Install {
                package: pack_out.clone(),
                root: Some(plugin_root.clone()),
                force: false,
                trusted_keys: vec![pub_key.into()],
                allow_unsigned: false,
                development: true,
            },
            Output::Json,
        )
        .expect("install succeeds");
        assert!(install_val.get("installed").is_some());

        // 5. Re-install without force should fail
        super::execute(
            PluginCommand::Install {
                package: pack_out.clone(),
                root: Some(plugin_root.clone()),
                force: false,
                trusted_keys: vec![pub_key.into()],
                allow_unsigned: false,
                development: true,
            },
            Output::Json,
        )
        .expect_err("install without force should fail on existing package");

        // 6. Re-install with force=true should succeed
        super::execute(
            PluginCommand::Install {
                package: pack_out.clone(),
                root: Some(plugin_root.clone()),
                force: true,
                trusted_keys: vec![pub_key.into()],
                allow_unsigned: false,
                development: true,
            },
            Output::Json,
        )
        .expect("install with force should succeed");

        // 7. List
        let list_val = super::execute(
            PluginCommand::List {
                root: Some(plugin_root.clone()),
                development: true,
            },
            Output::Json,
        )
        .expect("list succeeds");
        let list_arr = list_val.as_array().expect("array of plugins");
        assert_eq!(list_arr.len(), 1);
        assert_eq!(list_arr[0]["name"], "lilia-plugin-custom");
        assert_eq!(list_arr[0]["version"], "1.0.0");
    }
}
