#![allow(clippy::missing_errors_doc)]

use rusqlite::{params, OptionalExtension};

use crate::storage::{normalize_limit, row_version, validate_name};
use crate::Database;
use lilia_core::{ErrorCode, JsonEntry, LiliaError, Result};

fn decode_json_value(
    encoded: &str,
    codec: &dyn lilia_core::ValueCodec,
) -> Result<serde_json::Value> {
    if codec.name() == "identity" {
        return serde_json::from_str(encoded)
            .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false));
    }
    if let Ok(envelope) = serde_json::from_str::<serde_json::Value>(encoded) {
        if let (Some(_c_name), Some(b64_data)) = (
            envelope.get("$codec").and_then(serde_json::Value::as_str),
            envelope.get("$data").and_then(serde_json::Value::as_str),
        ) {
            use base64::Engine;
            let cipher_bytes = base64::engine::general_purpose::STANDARD
                .decode(b64_data)
                .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false))?;
            let plaintext_bytes = codec.decode(&cipher_bytes)?;
            return serde_json::from_slice(&plaintext_bytes)
                .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false));
        }
    }
    serde_json::from_str(encoded)
        .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false))
}

impl Database {
    pub fn json_get(&self, space: &str, id: &str) -> Result<Option<JsonEntry>> {
        validate_name("space", space)?;
        validate_name("id", id)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
            "SELECT value, version FROM _lilia_json WHERE space = ?1 AND id = ?2",
        )?;
        statement
            .query_row(params![space, id], |row| {
                Ok((row.get::<_, String>(0)?, row_version(row, 1)?))
            })
            .optional()?
            .map(|(encoded, version)| {
                decode_json_value(&encoded, self.codec.as_ref()).map(|value| JsonEntry {
                    space: space.to_owned(),
                    id: id.to_owned(),
                    value,
                    version,
                })
            })
            .transpose()
    }

    pub fn json_scan(
        &self,
        space: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JsonEntry>> {
        validate_name("space", space)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
            "SELECT id, value, version FROM _lilia_json WHERE space = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![space, after.unwrap_or(""), normalize_limit(limit)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row_version(row, 2)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, encoded, version) = row?;
            let value = decode_json_value(&encoded, self.codec.as_ref())?;
            entries.push(JsonEntry {
                space: space.to_owned(),
                id,
                value,
                version,
            });
        }
        Ok(entries)
    }

    pub fn create_json_index(&self, space: &str, index_name: &str, json_path: &str) -> Result<()> {
        validate_name("space", space)?;
        validate_name("index_name", index_name)?;
        if json_path.is_empty() {
            return Err(LiliaError::new(
                ErrorCode::InvalidInput,
                "json_path cannot be empty",
                false,
            ));
        }
        let mut writer = self.lock_writer()?;
        let tx = writer.transaction()?;
        tx.execute(
            "INSERT INTO _lilia_json_indexes(space, index_name, json_path) VALUES(?1, ?2, ?3)
             ON CONFLICT(space, index_name) DO UPDATE SET json_path = excluded.json_path",
            params![space, index_name, json_path],
        )?;
        tx.execute(
            "DELETE FROM _lilia_json_index_entries WHERE space = ?1 AND index_name = ?2",
            params![space, index_name],
        )?;
        let mut stmt = tx.prepare("SELECT id, value FROM _lilia_json WHERE space = ?1")?;
        let rows = stmt.query_map([space], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut entries = Vec::new();
        for r in rows {
            entries.push(r?);
        }
        drop(stmt);
        let path = if let Some(stripped) = json_path.strip_prefix("$.") {
            format!("/{}", stripped.replace('.', "/"))
        } else if !json_path.starts_with('/') {
            format!("/{json_path}")
        } else {
            json_path.to_owned()
        };
        let mut ins = tx.prepare(
            "INSERT INTO _lilia_json_index_entries(space, index_name, idx_value, doc_id) VALUES(?1, ?2, ?3, ?4)",
        )?;
        for (doc_id, encoded) in entries {
            let decoded = decode_json_value(&encoded, self.codec.as_ref())?;
            if let Some(target_val) = decoded.pointer(&path) {
                let val_str = match target_val {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Null => "null".to_string(),
                    _ => target_val.to_string(),
                };
                ins.execute(params![space, index_name, val_str, doc_id])?;
            }
        }
        drop(ins);
        tx.commit()?;
        Ok(())
    }

    pub fn json_scan_by_index(
        &self,
        space: &str,
        index_name: &str,
        value: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JsonEntry>> {
        validate_name("space", space)?;
        validate_name("index_name", index_name)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
            "SELECT j.id, j.value, j.version
             FROM _lilia_json_index_entries e
             JOIN _lilia_json j ON e.space = j.space AND e.doc_id = j.id
             WHERE e.space = ?1 AND e.index_name = ?2 AND e.idx_value = ?3 AND e.doc_id > ?4
             ORDER BY e.doc_id LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                space,
                index_name,
                value,
                after_id.unwrap_or(""),
                normalize_limit(limit)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row_version(row, 2)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, encoded, version) = row?;
            let value = decode_json_value(&encoded, self.codec.as_ref())?;
            entries.push(JsonEntry {
                space: space.to_owned(),
                id,
                value,
                version,
            });
        }
        Ok(entries)
    }

    pub fn json_scan_prefix(
        &self,
        space: &str,
        prefix: &str,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JsonEntry>> {
        validate_name("space", space)?;
        let connection = self.read_lock()?;
        let min_id = after_id.unwrap_or("");
        let mut statement = connection.prepare_cached(
            "SELECT id, value, version FROM _lilia_json
             WHERE space = ?1 AND id >= ?2 AND id > ?3 AND id LIKE ?4
             ORDER BY id LIMIT ?5",
        )?;
        let like_pattern = format!("{prefix}%");
        let rows = statement.query_map(
            params![space, prefix, min_id, like_pattern, normalize_limit(limit)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row_version(row, 2)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let (id, encoded, version) = row?;
            let value = decode_json_value(&encoded, self.codec.as_ref())?;
            entries.push(JsonEntry {
                space: space.to_owned(),
                id,
                value,
                version,
            });
        }
        Ok(entries)
    }
}
