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
        } => Ok(serde_json::to_value(verify_package(
            &package,
            &decode_keys(&trusted_keys)?,
            allow_unsigned,
        )?)?),
        PluginCommand::Trust { command } => trust(command),
        PluginCommand::Install {
            package,
            root,
            trusted_keys,
            allow_unsigned,
        } => {
            let path = install_verified(
                &package,
                &root,
                &decode_keys(&trusted_keys)?,
                allow_unsigned,
            )?;
            Ok(json!({"installed": path}))
        }
        PluginCommand::List { root } => {
            let entries = if root.exists() {
                fs::read_dir(root)?
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            Ok(json!(entries))
        }
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
            fs::remove_dir_all(&path)?;
            Ok(json!({"removed": path}))
        }
    }
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
