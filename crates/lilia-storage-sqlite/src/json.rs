#![allow(clippy::missing_errors_doc)]

use rusqlite::{params, OptionalExtension};

use crate::storage::{normalize_limit, row_version, validate_name};
use crate::Database;
use lilia_core::{ErrorCode, JsonEntry, JsonIndexInfo, LiliaError, Result};

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

fn doc_matches_query(
    doc: &serde_json::Value,
    query: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    for (k, expected_v) in query {
        let actual_v = if let Some(direct) = doc.get(k) {
            Some(direct)
        } else {
            let ptr = if let Some(stripped) = k.strip_prefix("$.") {
                format!("/{}", stripped.replace('.', "/"))
            } else if k.contains('.') {
                format!("/{}", k.replace('.', "/"))
            } else if k.starts_with('/') {
                k.clone()
            } else {
                format!("/{k}")
            };
            doc.pointer(&ptr)
        };

        if let Some(actual) = actual_v {
            if actual == expected_v {
                continue;
            }
            if let serde_json::Value::Array(items) = actual {
                if items.iter().any(|item| item == expected_v) {
                    continue;
                }
            }
            return false;
        } else if !expected_v.is_null() {
            return false;
        }
    }
    true
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
            "INSERT OR IGNORE INTO _lilia_json_index_entries(space, index_name, idx_value, doc_id) VALUES(?1, ?2, ?3, ?4)",
        )?;
        for (doc_id, encoded) in entries {
            let decoded = decode_json_value(&encoded, self.codec.as_ref())?;
            if let Some(target_val) = decoded.pointer(&path) {
                if let serde_json::Value::Array(items) = target_val {
                    for item in items {
                        let val_str = match item {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Null => "null".to_string(),
                            _ => item.to_string(),
                        };
                        ins.execute(params![space, index_name, val_str, doc_id])?;
                    }
                } else {
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
        }
        drop(ins);
        tx.commit()?;
        Ok(())
    }

    pub fn drop_json_index(&self, space: &str, index_name: &str) -> Result<bool> {
        validate_name("space", space)?;
        validate_name("index_name", index_name)?;
        let mut writer = self.lock_writer()?;
        let tx = writer.transaction()?;
        let deleted = tx.execute(
            "DELETE FROM _lilia_json_indexes WHERE space = ?1 AND index_name = ?2",
            params![space, index_name],
        )? > 0;
        if deleted {
            tx.execute(
                "DELETE FROM _lilia_json_index_entries WHERE space = ?1 AND index_name = ?2",
                params![space, index_name],
            )?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn list_json_indexes(&self, space: &str) -> Result<Vec<JsonIndexInfo>> {
        validate_name("space", space)?;
        let connection = self.read_lock()?;
        let mut statement = connection.prepare_cached(
            "SELECT space, index_name, json_path FROM _lilia_json_indexes WHERE space = ?1 ORDER BY index_name",
        )?;
        let rows = statement.query_map([space], |row| {
            Ok(JsonIndexInfo {
                space: row.get(0)?,
                index_name: row.get(1)?,
                json_path: row.get(2)?,
            })
        })?;
        let mut indexes = Vec::new();
        for row in rows {
            indexes.push(row?);
        }
        Ok(indexes)
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

    pub fn json_find(
        &self,
        space: &str,
        query: &serde_json::Map<String, serde_json::Value>,
        after_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<JsonEntry>> {
        validate_name("space", space)?;
        if query.is_empty() {
            return self.json_scan(space, after_id, limit);
        }

        let max_results = usize::try_from(normalize_limit(limit)).unwrap_or(usize::MAX);
        let indexes = self.list_json_indexes(space)?;

        let matched_index = query.iter().find_map(|(k, v)| {
            for idx in &indexes {
                let matches_name = &idx.index_name == k;
                let matches_exact_path = &idx.json_path == k;
                let matches_prefixed = idx.json_path.strip_prefix("$.") == Some(k.as_str());
                let matches_slash = idx.json_path.strip_prefix('/') == Some(k.as_str());
                if matches_name || matches_exact_path || matches_prefixed || matches_slash {
                    return Some((idx.clone(), v));
                }
            }
            None
        });

        let mut results = Vec::new();
        let mut cursor = after_id.map(ToOwned::to_owned);

        if let Some((idx, target_val)) = matched_index {
            let val_str = match target_val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "null".to_string(),
                _ => target_val.to_string(),
            };

            if query.len() == 1 {
                return self.json_scan_by_index(space, &idx.index_name, &val_str, after_id, limit);
            }

            while results.len() < max_results {
                let batch = self.json_scan_by_index(
                    space,
                    &idx.index_name,
                    &val_str,
                    cursor.as_deref(),
                    100,
                )?;
                if batch.is_empty() {
                    break;
                }
                for entry in batch {
                    cursor = Some(entry.id.clone());
                    if doc_matches_query(&entry.value, query) {
                        results.push(entry);
                        if results.len() == max_results {
                            break;
                        }
                    }
                }
            }
        } else {
            while results.len() < max_results {
                let batch = self.json_scan(space, cursor.as_deref(), 100)?;
                if batch.is_empty() {
                    break;
                }
                for entry in batch {
                    cursor = Some(entry.id.clone());
                    if doc_matches_query(&entry.value, query) {
                        results.push(entry);
                        if results.len() == max_results {
                            break;
                        }
                    }
                }
            }
        }

        Ok(results)
    }
}
