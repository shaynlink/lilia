use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::json;

use crate::storage::{row_version, store_version, validate_name};
use lilia_core::{BatchOperation, ErrorCode, LiliaError, MutationResult, Result};

pub(crate) fn apply(tx: &Transaction<'_>, operation: &BatchOperation) -> Result<MutationResult> {
    match operation {
        BatchOperation::KvSet {
            namespace,
            key,
            value,
            if_version,
            expires_at_ms,
        } => {
            validate_name("namespace", namespace)?;
            let current = kv_version(tx, namespace, key)?;
            assert_version(current, *if_version)?;
            let version = current.unwrap_or(0) + 1;
            tx.execute(
                "INSERT INTO _lilia_kv(namespace,key,value,version,expires_at_ms) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,version=excluded.version,expires_at_ms=excluded.expires_at_ms",
                params![namespace, key, value, store_version(version)?, expires_at_ms],
            )?;
            Ok(MutationResult {
                version: Some(version),
                deleted: false,
            })
        }
        BatchOperation::KvDelete {
            namespace,
            key,
            if_version,
        } => {
            validate_name("namespace", namespace)?;
            let current = kv_version(tx, namespace, key)?;
            assert_version(current, *if_version)?;
            let deleted = tx.execute(
                "DELETE FROM _lilia_kv WHERE namespace = ?1 AND key = ?2",
                params![namespace, key],
            )? > 0;
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
        BatchOperation::JsonPut {
            space,
            id,
            value,
            if_version,
        } => {
            validate_name("space", space)?;
            validate_name("id", id)?;
            let current = json_version(tx, space, id)?;
            assert_version(current, *if_version)?;
            let version = current.unwrap_or(0) + 1;
            let encoded = serde_json::to_string(value).map_err(|error| {
                LiliaError::new(ErrorCode::InvalidInput, error.to_string(), false)
            })?;
            tx.execute(
                "INSERT INTO _lilia_json(space,id,value,version) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(space,id) DO UPDATE SET value=excluded.value,version=excluded.version",
                params![space, id, encoded, store_version(version)?],
            )?;
            Ok(MutationResult {
                version: Some(version),
                deleted: false,
            })
        }
        BatchOperation::JsonDelete {
            space,
            id,
            if_version,
        } => {
            validate_name("space", space)?;
            validate_name("id", id)?;
            let current = json_version(tx, space, id)?;
            assert_version(current, *if_version)?;
            let deleted = tx.execute(
                "DELETE FROM _lilia_json WHERE space = ?1 AND id = ?2",
                params![space, id],
            )? > 0;
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
    }
}

fn kv_version(tx: &Transaction<'_>, namespace: &str, key: &[u8]) -> Result<Option<u64>> {
    tx.query_row(
        "SELECT version FROM _lilia_kv WHERE namespace = ?1 AND key = ?2",
        params![namespace, key],
        |row| row_version(row, 0),
    )
    .optional()
    .map_err(Into::into)
}

fn json_version(tx: &Transaction<'_>, space: &str, id: &str) -> Result<Option<u64>> {
    tx.query_row(
        "SELECT version FROM _lilia_json WHERE space = ?1 AND id = ?2",
        params![space, id],
        |row| row_version(row, 0),
    )
    .optional()
    .map_err(Into::into)
}

fn assert_version(current: Option<u64>, expected: Option<u64>) -> Result<()> {
    if expected.is_some_and(|expected| current.unwrap_or(0) != expected) {
        return Err(
            LiliaError::new(ErrorCode::Conflict, "optimistic version conflict", true)
                .details(json!({ "expected": expected.map(|value| value.to_string()), "actual": current.map(|value| value.to_string()) })),
        );
    }
    Ok(())
}
