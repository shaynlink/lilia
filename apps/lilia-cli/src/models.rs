use std::fs;
use std::path::Path;

use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::{json, Value};

use crate::args::{Command, JsonCommand, KvCommand};

pub(crate) fn execute(command: Command, path: &Path) -> anyhow::Result<Value> {
    let database = Database::open(DatabaseOptions::durable(path))?;
    Ok(match command {
        Command::Init => json!({"ok": true, "path": database.path()}),
        Command::Doctor => json!({
            "ok": database.integrity_check()?,
            "path": database.path(),
            "journal": "wal",
            "synchronous": "full"
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

fn kv(database: &Database, command: KvCommand) -> anyhow::Result<Value> {
    let mutate =
        |operation| Ok::<_, anyhow::Error>(serde_json::to_value(database.batch(&[operation])?)?);
    match command {
        KvCommand::Get { namespace, key } => Ok(serde_json::to_value(
            database.kv_get(&namespace, key.as_bytes())?,
        )?),
        KvCommand::Set {
            namespace,
            key,
            value,
            if_version,
            expires_at_ms,
        } => mutate(BatchOperation::KvSet {
            namespace,
            key: key.into_bytes(),
            value: value.into_bytes(),
            if_version,
            expires_at_ms,
        }),
        KvCommand::Delete {
            namespace,
            key,
            if_version,
        } => mutate(BatchOperation::KvDelete {
            namespace,
            key: key.into_bytes(),
            if_version,
        }),
        KvCommand::Scan {
            namespace,
            after,
            limit,
        } => Ok(serde_json::to_value(database.kv_scan(
            &namespace,
            after.as_deref().map(str::as_bytes),
            limit,
        )?)?),
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
