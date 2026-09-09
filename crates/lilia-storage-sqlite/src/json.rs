#![allow(clippy::missing_errors_doc)]

use rusqlite::{params, OptionalExtension};

use crate::storage::{normalize_limit, row_version, validate_name};
use crate::Database;
use lilia_core::{ErrorCode, JsonEntry, LiliaError, Result};

impl Database {
    pub fn json_get(&self, space: &str, id: &str) -> Result<Option<JsonEntry>> {
        validate_name("space", space)?;
        validate_name("id", id)?;
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value, version FROM _lilia_json WHERE space = ?1 AND id = ?2",
                params![space, id],
                |row| Ok((row.get::<_, String>(0)?, row_version(row, 1)?)),
            )
            .optional()?
            .map(|(encoded, version)| {
                serde_json::from_str(&encoded)
                    .map(|value| JsonEntry {
                        space: space.to_owned(),
                        id: id.to_owned(),
                        value,
                        version,
                    })
                    .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false))
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
        let connection = self.lock()?;
        let mut statement = connection.prepare(
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
        rows.map(|row| {
            let (id, encoded, version) = row?;
            let value = serde_json::from_str(&encoded).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    1,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(JsonEntry {
                space: space.to_owned(),
                id,
                value,
                version,
            })
        })
        .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
        .map_err(Into::into)
    }
}
