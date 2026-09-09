#![allow(clippy::missing_errors_doc)]

use rusqlite::{params, OptionalExtension};

use crate::storage::{normalize_limit, now_ms, row_version, validate_name};
use crate::Database;
use lilia_core::{KvEntry, Result};

impl Database {
    pub fn kv_get(&self, namespace: &str, key: &[u8]) -> Result<Option<KvEntry>> {
        validate_name("namespace", namespace)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value, version, expires_at_ms FROM _lilia_kv
                 WHERE namespace = ?1 AND key = ?2 AND (expires_at_ms IS NULL OR expires_at_ms > ?3)",
                params![namespace, key, now_ms()],
                |row| {
                    Ok(KvEntry {
                        namespace: namespace.to_owned(),
                        key: key.to_vec(),
                        value: row.get(0)?,
                        version: row_version(row, 1)?,
                        expires_at_ms: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn kv_scan(
        &self,
        namespace: &str,
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<Vec<KvEntry>> {
        validate_name("namespace", namespace)?;
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT key, value, version, expires_at_ms FROM _lilia_kv
             WHERE namespace = ?1 AND key > ?2 AND (expires_at_ms IS NULL OR expires_at_ms > ?3)
             ORDER BY key LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                namespace,
                after.unwrap_or_default(),
                now_ms(),
                normalize_limit(limit)
            ],
            |row| {
                Ok(KvEntry {
                    namespace: namespace.to_owned(),
                    key: row.get(0)?,
                    value: row.get(1)?,
                    version: row_version(row, 2)?,
                    expires_at_ms: row.get(3)?,
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}
