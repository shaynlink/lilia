#![allow(clippy::missing_errors_doc)]

use lilia_core::{Result, SqlQueryResult, ValueCodec};
use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, Error as RusqliteError};
use std::sync::Arc;

pub(crate) fn register_sql_functions(
    conn: &Connection,
    codec: &Arc<dyn ValueCodec>,
) -> std::result::Result<(), RusqliteError> {
    let codec_kv = Arc::clone(codec);
    conn.create_scalar_function(
        "lilia_decode_kv",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let val = ctx.get_raw(0);
            match val {
                rusqlite::types::ValueRef::Blob(b) => match codec_kv.decode(b) {
                    Ok(dec) => Ok(rusqlite::types::Value::Blob(dec)),
                    Err(e) => Err(RusqliteError::UserFunctionError(e.to_string().into())),
                },
                rusqlite::types::ValueRef::Null => Ok(rusqlite::types::Value::Null),
                _ => Ok(rusqlite::types::Value::Blob(val.as_bytes()?.to_vec())),
            }
        },
    )?;

    let codec_json = Arc::clone(codec);
    conn.create_scalar_function(
        "lilia_decode_json",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        move |ctx| {
            let val = ctx.get_raw(0);
            match val {
                rusqlite::types::ValueRef::Text(t) => {
                    let text = std::str::from_utf8(t)
                        .map_err(|e| RusqliteError::UserFunctionError(e.to_string().into()))?;
                    if text.starts_with("{\"$codec\":") {
                        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(text) {
                            if let Some(b64) = json_val.get("$data").and_then(|v| v.as_str()) {
                                use base64::Engine;
                                if let Ok(ciphertext) =
                                    base64::engine::general_purpose::STANDARD.decode(b64)
                                {
                                    if let Ok(plaintext) = codec_json.decode(&ciphertext) {
                                        if let Ok(plain_str) = String::from_utf8(plaintext) {
                                            return Ok(rusqlite::types::Value::Text(plain_str));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Ok(rusqlite::types::Value::Text(text.to_owned()))
                }
                rusqlite::types::ValueRef::Null => Ok(rusqlite::types::Value::Null),
                _ => Ok(rusqlite::types::Value::Text(
                    String::from_utf8_lossy(val.as_bytes()?).into_owned(),
                )),
            }
        },
    )?;
    Ok(())
}

fn json_value_to_sqlite(val: &serde_json::Value) -> rusqlite::types::Value {
    match val {
        serde_json::Value::Null => rusqlite::types::Value::Null,
        serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(i64::from(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                rusqlite::types::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                rusqlite::types::Value::Real(f)
            } else {
                rusqlite::types::Value::Text(n.to_string())
            }
        }
        serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            rusqlite::types::Value::Text(val.to_string())
        }
    }
}

fn sqlite_value_to_json(val: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    match val {
        rusqlite::types::ValueRef::Null => serde_json::Value::Null,
        rusqlite::types::ValueRef::Integer(i) => serde_json::Value::Number(i.into()),
        rusqlite::types::ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        rusqlite::types::ValueRef::Text(t) => {
            let s = std::str::from_utf8(t).unwrap_or_default();
            serde_json::Value::String(s.to_owned())
        }
        rusqlite::types::ValueRef::Blob(b) => serde_json::Value::String(hex::encode(b)),
    }
}

impl crate::Database {
    /// Execute a SQL query or mutation.
    /// Read-only queries (SELECT, EXPLAIN) are executed on the concurrent reader pool.
    /// Mutating queries (INSERT, UPDATE, DELETE) are executed via the exclusive writer queue.
    #[allow(clippy::too_many_lines)]
    pub fn sql_query(&self, sql: &str, params: &[serde_json::Value]) -> Result<SqlQueryResult> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Ok(SqlQueryResult {
                columns: Vec::new(),
                rows: Vec::new(),
                rows_affected: 0,
            });
        }

        // Prepare on reader to check statement read-only flag
        let is_readonly = {
            let conn = self.read_lock()?;
            let stmt = conn.prepare(trimmed)?;
            stmt.readonly()
        };

        let sqlite_params: Vec<rusqlite::types::Value> =
            params.iter().map(json_value_to_sqlite).collect();
        let param_refs: Vec<&dyn rusqlite::ToSql> = sqlite_params
            .iter()
            .map(|v| v as &dyn rusqlite::ToSql)
            .collect();

        if is_readonly {
            let conn = self.read_lock()?;
            let mut stmt = conn.prepare(trimmed)?;
            let col_count = stmt.column_count();
            let columns: Vec<String> = stmt
                .column_names()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect();

            let mut rows_iter = stmt.query(param_refs.as_slice())?;
            let mut rows = Vec::new();
            while let Some(row) = rows_iter.next()? {
                let mut row_vals = Vec::with_capacity(col_count);
                for i in 0..col_count {
                    let val_ref = row.get_ref(i)?;
                    row_vals.push(sqlite_value_to_json(val_ref));
                }
                rows.push(row_vals);
            }

            Ok(SqlQueryResult {
                columns,
                rows,
                rows_affected: 0,
            })
        } else {
            let mut writer = self.lock_writer()?;
            let tx = writer.transaction()?;
            let mut stmt = tx.prepare(trimmed)?;
            let col_count = stmt.column_count();
            let columns: Vec<String> = stmt
                .column_names()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect();

            let mut rows = Vec::new();
            let rows_affected = if col_count > 0 {
                let mut rows_iter = stmt.query(param_refs.as_slice())?;
                while let Some(row) = rows_iter.next()? {
                    let mut row_vals = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let val_ref = row.get_ref(i)?;
                        row_vals.push(sqlite_value_to_json(val_ref));
                    }
                    rows.push(row_vals);
                }
                0
            } else {
                let count = stmt.execute(param_refs.as_slice())?;
                u64::try_from(count).unwrap_or(0)
            };
            drop(stmt);
            tx.commit()?;

            Ok(SqlQueryResult {
                columns,
                rows,
                rows_affected,
            })
        }
    }
}
