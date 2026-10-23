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
         CREATE TABLE IF NOT EXISTS _lilia_graph_nodes(
           graph TEXT NOT NULL,
           id TEXT NOT NULL,
           kind TEXT NOT NULL,
           metadata TEXT NOT NULL CHECK(json_valid(metadata)),
           PRIMARY KEY(graph,id)
         ) STRICT, WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS _lilia_graph_edges(
           graph TEXT NOT NULL,
           source TEXT NOT NULL,
           target TEXT NOT NULL,
           kind TEXT NOT NULL,
           PRIMARY KEY(graph,source,target,kind)
         ) STRICT, WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS _lilia_graph_edges_target ON _lilia_graph_edges(graph,target);
         CREATE TABLE IF NOT EXISTS _lilia_json_indexes(
           space TEXT NOT NULL,
           index_name TEXT NOT NULL,
           json_path TEXT NOT NULL,
           PRIMARY KEY(space,index_name)
         ) STRICT, WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS _lilia_json_index_entries(
           space TEXT NOT NULL,
           index_name TEXT NOT NULL,
           idx_value TEXT NOT NULL,
           doc_id TEXT NOT NULL,
           PRIMARY KEY(space,index_name,idx_value,doc_id)
         ) STRICT, WITHOUT ROWID;
         CREATE VIRTUAL TABLE IF NOT EXISTS _lilia_fts USING fts5(
           collection UNINDEXED,
           doc_id UNINDEXED,
           content,
           tokenize='unicode61'
         );
         CREATE VIEW IF NOT EXISTS lilia_kv AS
           SELECT namespace, key, lilia_decode_kv(value) AS value, version, expires_at_ms
           FROM _lilia_kv;
         CREATE VIEW IF NOT EXISTS lilia_json AS
           SELECT space, id, lilia_decode_json(value) AS value, version
           FROM _lilia_json;
         CREATE VIEW IF NOT EXISTS lilia_graph_nodes AS
           SELECT graph, id, kind, metadata
           FROM _lilia_graph_nodes;
         CREATE VIEW IF NOT EXISTS lilia_graph_edges AS
           SELECT graph, source, target, kind
           FROM _lilia_graph_edges;
         COMMIT;",
    )?;
    Ok(())
}

pub(crate) fn is_uri_or_memory(path: &Path) -> bool {
    let s = path.to_str().unwrap_or("");
    s == ":memory:" || s.starts_with("file:")
}

pub(crate) fn prepare_parent(path: &Path) -> Result<()> {
    if is_uri_or_memory(path) {
        return Ok(());
    }
    let Some(parent) = path.parent() else {
        return Err(LiliaError::new(
            ErrorCode::InvalidInput,
            "database path has no parent",
            false,
        ));
    };
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    // Existing directories belong to the caller: never change their permissions.
    builder
        .create(parent)
        .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
    Ok(())
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
pub(crate) fn secure_database_file(path: &Path) -> Result<()> {
    if is_uri_or_memory(path) {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
    }
    #[cfg(not(unix))]
    let _ = path;
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
