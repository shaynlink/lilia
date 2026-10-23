#![allow(clippy::missing_errors_doc)]

use rusqlite::{params, OptionalExtension};

use crate::storage::{normalize_limit, now_ms, row_version, validate_name};
use crate::Database;
use lilia_core::{KvEntry, Result};

impl Database {
    pub fn kv_get(&self, namespace: &str, key: &[u8]) -> Result<Option<KvEntry>> {
        validate_name("namespace", namespace)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
            "SELECT value, version, expires_at_ms FROM _lilia_kv
             WHERE namespace = ?1 AND key = ?2 AND (expires_at_ms IS NULL OR expires_at_ms > ?3)",
        )?;
        let entry: Option<(Vec<u8>, u64, Option<i64>)> = statement
            .query_row(params![namespace, key, now_ms()], |row| {
                Ok((row.get(0)?, row_version(row, 1)?, row.get(2)?))
            })
            .optional()?;
        let Some((raw_val, version, expires_at_ms)) = entry else {
            return Ok(None);
        };
        let value = self.codec.decode(&raw_val)?;
        Ok(Some(KvEntry {
            namespace: namespace.to_owned(),
            key: key.to_vec(),
            value,
            version,
            expires_at_ms,
        }))
    }

    pub fn kv_scan(
        &self,
        namespace: &str,
        after: Option<&[u8]>,
        limit: u32,
    ) -> Result<Vec<KvEntry>> {
        validate_name("namespace", namespace)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
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
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row_version(row, 2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let (key, raw_val, version, expires_at_ms) = row?;
            let value = self.codec.decode(&raw_val)?;
            entries.push(KvEntry {
                namespace: namespace.to_owned(),
                key,
                value,
                version,
                expires_at_ms,
            });
        }
        Ok(entries)
    }
}
