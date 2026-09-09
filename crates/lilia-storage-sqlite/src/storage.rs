use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use lilia_core::{ErrorCode, LiliaError, Result};

pub(crate) fn migrate(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS _lilia_metadata(
           key TEXT PRIMARY KEY NOT NULL,
           value TEXT NOT NULL
         ) STRICT;
         INSERT OR IGNORE INTO _lilia_metadata(key,value) VALUES('format_version','1');
         CREATE TABLE IF NOT EXISTS _lilia_kv(
           namespace TEXT NOT NULL,
           key BLOB NOT NULL,
           value BLOB NOT NULL,
           version INTEGER NOT NULL CHECK(version > 0),
           expires_at_ms INTEGER,
           PRIMARY KEY(namespace,key)
         ) STRICT, WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS _lilia_kv_expiry ON _lilia_kv(expires_at_ms)
           WHERE expires_at_ms IS NOT NULL;
         CREATE TABLE IF NOT EXISTS _lilia_json(
           space TEXT NOT NULL,
           id TEXT NOT NULL,
           value TEXT NOT NULL CHECK(json_valid(value)),
           version INTEGER NOT NULL CHECK(version > 0),
           PRIMARY KEY(space,id)
         ) STRICT, WITHOUT ROWID;
         COMMIT;",
    )?;
    Ok(())
}

pub(crate) fn prepare_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(LiliaError::new(
            ErrorCode::InvalidInput,
            "database path has no parent",
            false,
        ));
    };
    fs::create_dir_all(parent)
        .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
    }
    Ok(())
}

pub(crate) fn secure_database_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
    }
    Ok(())
}

pub(crate) fn validate_name(field: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 255 || value.chars().any(char::is_control) {
        return Err(LiliaError::new(
            ErrorCode::InvalidInput,
            format!("{field} must contain 1..=255 non-control characters"),
            false,
        ));
    }
    Ok(())
}

pub(crate) fn normalize_limit(limit: u32) -> i64 {
    i64::from(limit.clamp(1, 1_000))
}

pub(crate) fn row_version(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

pub(crate) fn store_version(version: u64) -> Result<i64> {
    i64::try_from(version)
        .map_err(|_| LiliaError::new(ErrorCode::Storage, "record version is exhausted", false))
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}
