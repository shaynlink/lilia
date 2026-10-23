use std::fs;
use std::path::Path;

use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::{json, Value};

use base64::Engine;

use crate::args::{Command, Encoding, JsonCommand, KvCommand};

#[allow(unsafe_code)]
pub(crate) fn execute(
    command: Command,
    path: &Path,
    plugin: Option<&Path>,
) -> anyhow::Result<Value> {
    let mut options = DatabaseOptions::durable(path);
    let _loaded = if let Some(plugin_path) = plugin {
        let loaded = unsafe { lilia_plugin_api::load_plugin(plugin_path)? };
        if let Some(codec) = loaded.codec() {
            options = options.with_codec(std::sync::Arc::new(codec));
        }
        Some(loaded)
    } else {
        None
    };
    let database = Database::open(options)?;
    Ok(match command {
        Command::Init => json!({"ok": true, "path": database.path()}),
        Command::Doctor { deep } => json!({
            "ok": database.integrity_check()?,
            "path": database.path(),
            "journal": "wal",
            "synchronous": "full",
            "deep": deep,
            "checkpoint": if deep { database.checkpoint().is_ok() } else { true }
        }),
        Command::Backup { destination } => {
            database.backup(&destination)?;
            json!({"ok": true, "destination": destination})
        }
        Command::Kv { command } => kv(&database, command)?,
        Command::Json { command } => json_model(&database, command)?,
        Command::Batch { file } => {
            let operations: Vec<BatchOperation> = serde_json::from_slice(&fs::read(file)?)?;
            serde_json::to_value(database.batch(&operations)?)?
        }
        Command::Plugin { .. } | Command::Daemon { .. } => unreachable!(),
    })
}

fn decode_binary(raw: &str, encoding: Encoding, field: &str) -> lilia_core::Result<Vec<u8>> {
    match encoding {
        Encoding::Text => Ok(raw.as_bytes().to_vec()),
        Encoding::Hex => hex::decode(raw).map_err(|error| {
            lilia_core::LiliaError::new(
                lilia_core::ErrorCode::InvalidInput,
                format!("invalid {field} hex encoding: {error}"),
                false,
            )
        }),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|error| {
                lilia_core::LiliaError::new(
                    lilia_core::ErrorCode::InvalidInput,
                    format!("invalid {field} base64 encoding: {error}"),
                    false,
                )
            }),
    }
}

fn kv(database: &Database, command: KvCommand) -> anyhow::Result<Value> {
    let mutate =
        |operation| Ok::<_, anyhow::Error>(serde_json::to_value(database.batch(&[operation])?)?);
    match command {
        KvCommand::Get {
            namespace,
            key,
            encoding,
            key_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            Ok(serde_json::to_value(database.kv_get(&namespace, &key)?)?)
        }
        KvCommand::Set {
            namespace,
            key,
            value,
            if_version,
            expires_at_ms,
            encoding,
            key_encoding,
            value_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            let value = decode_binary(&value, value_encoding.unwrap_or(encoding), "value")?;
            mutate(BatchOperation::KvSet {
                namespace,
                key,
                value,
                if_version,
                expires_at_ms,
            })
        }
        KvCommand::Delete {
            namespace,
            key,
            if_version,
            encoding,
            key_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            mutate(BatchOperation::KvDelete {
                namespace,
                key,
                if_version,
            })
        }
        KvCommand::Scan {
            namespace,
            after,
            limit,
            encoding,
            key_encoding,
        } => {
            let key_enc = key_encoding.unwrap_or(encoding);
            let after = after
                .as_deref()
                .map(|cursor| decode_binary(cursor, key_enc, "after"))
                .transpose()?;
            Ok(serde_json::to_value(database.kv_scan(
                &namespace,
                after.as_deref(),
                limit,
            )?)?)
        }
    }
}

fn json_model(database: &Database, command: JsonCommand) -> anyhow::Result<Value> {
    let mutate =
        |operation| Ok::<_, anyhow::Error>(serde_json::to_value(database.batch(&[operation])?)?);
    match command {
        JsonCommand::Get { space, id } => {
            Ok(serde_json::to_value(database.json_get(&space, &id)?)?)
        }
        JsonCommand::Put {
            space,
            id,
            value,
            if_version,
        } => mutate(BatchOperation::JsonPut {
            space,
            id,
            value: serde_json::from_str(&value)?,
            if_version,
        }),
        JsonCommand::Delete {
            space,
            id,
            if_version,
        } => mutate(BatchOperation::JsonDelete {
            space,
            id,
            if_version,
        }),
        JsonCommand::Scan {
            space,
            after,
            limit,
        } => Ok(serde_json::to_value(database.json_scan(
            &space,
            after.as_deref(),
            limit,
        )?)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn decode_binary_formats() {
        assert_eq!(
            decode_binary("hello", Encoding::Text, "key").unwrap(),
            b"hello"
        );
        assert_eq!(
            decode_binary("0080ff", Encoding::Hex, "key").unwrap(),
            vec![0, 128, 255]
        );
        assert_eq!(
            decode_binary("AID/", Encoding::Base64, "key").unwrap(),
            vec![0, 128, 255]
        );

        let err_hex = decode_binary("zz", Encoding::Hex, "key").unwrap_err();
        assert_eq!(err_hex.code, lilia_core::ErrorCode::InvalidInput);

        let err_b64 = decode_binary("!!!", Encoding::Base64, "key").unwrap_err();
        assert_eq!(err_b64.code, lilia_core::ErrorCode::InvalidInput);
    }

    #[test]
    fn kv_hex_and_base64_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.lilia");
        let database = Database::open(DatabaseOptions::durable(&path)).unwrap();

        let res = kv(
            &database,
            KvCommand::Set {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                value: "gICAgA==".into(),
                if_version: None,
                expires_at_ms: None,
                encoding: Encoding::Text,
                key_encoding: Some(Encoding::Hex),
                value_encoding: Some(Encoding::Base64),
            },
        )
        .unwrap();
        assert_eq!(res[0]["deleted"], false);

        let entry = kv(
            &database,
            KvCommand::Get {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        assert_eq!(entry["key"], serde_json::json!([0, 128, 255]));
        assert_eq!(entry["value"], serde_json::json!([128, 128, 128, 128]));

        let scan = kv(
            &database,
            KvCommand::Scan {
                namespace: "test_ns".into(),
                after: Some("00".into()),
                limit: 10,
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        let items: Vec<Value> = serde_json::from_value(scan).unwrap();
        assert_eq!(items.len(), 1);

        let del = kv(
            &database,
            KvCommand::Delete {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                if_version: None,
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        assert_eq!(del[0]["deleted"], true);
    }
}
