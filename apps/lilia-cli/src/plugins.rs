use std::fs;

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use lilia_plugin_api::{install_verified, verify_package};
use serde_json::{json, Value};

use crate::args::PluginCommand;

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
