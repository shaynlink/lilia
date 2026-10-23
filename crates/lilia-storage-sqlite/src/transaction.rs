use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::json;

use crate::storage::{row_version, store_version, validate_name};
use lilia_core::{BatchOperation, ErrorCode, LiliaError, MutationResult, Result};

#[allow(clippy::too_many_lines)]
pub(crate) fn apply(
    tx: &Transaction<'_>,
    operation: &BatchOperation,
    codec: &dyn lilia_core::ValueCodec,
) -> Result<MutationResult> {
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
            let stored_value = codec.encode(value)?;
            let mut stmt = tx.prepare_cached(
                "INSERT INTO _lilia_kv(namespace,key,value,version,expires_at_ms) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(namespace,key) DO UPDATE SET value=excluded.value,version=excluded.version,expires_at_ms=excluded.expires_at_ms",
            )?;
            stmt.execute(params![
                namespace,
                key,
                stored_value,
                store_version(version)?,
                expires_at_ms
            ])?;
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
            let mut stmt =
                tx.prepare_cached("DELETE FROM _lilia_kv WHERE namespace = ?1 AND key = ?2")?;
            let deleted = stmt.execute(params![namespace, key])? > 0;
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
            let encoded = if codec.name() == "identity" {
                serde_json::to_string(value).map_err(|error| {
                    LiliaError::new(ErrorCode::InvalidInput, error.to_string(), false)
                })?
            } else {
                use base64::Engine;
                let raw_bytes = serde_json::to_vec(value).map_err(|error| {
                    LiliaError::new(ErrorCode::InvalidInput, error.to_string(), false)
                })?;
                let cipher_bytes = codec.encode(&raw_bytes)?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&cipher_bytes);
                serde_json::to_string(&json!({
                    "$codec": codec.name(),
                    "$data": b64,
                }))
                .map_err(|error| LiliaError::new(ErrorCode::Storage, error.to_string(), false))?
            };
            let mut stmt = tx.prepare_cached(
                "INSERT INTO _lilia_json(space,id,value,version) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(space,id) DO UPDATE SET value=excluded.value,version=excluded.version",
            )?;
            stmt.execute(params![space, id, encoded, store_version(version)?])?;
            update_json_indexes(tx, space, id, value)?;
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
            let mut stmt =
                tx.prepare_cached("DELETE FROM _lilia_json WHERE space = ?1 AND id = ?2")?;
            let deleted = stmt.execute(params![space, id])? > 0;
            if deleted {
                delete_json_indexes(tx, space, id)?;
            }
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
        BatchOperation::GraphSetNode {
            graph,
            id,
            kind,
            metadata,
        } => {
            validate_name("graph", graph)?;
            validate_name("id", id)?;
            let meta_str = serde_json::to_string(metadata).map_err(|error| {
                LiliaError::new(ErrorCode::InvalidInput, error.to_string(), false)
            })?;
            let mut stmt = tx.prepare_cached(
                "INSERT INTO _lilia_graph_nodes(graph, id, kind, metadata) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(graph, id) DO UPDATE SET kind=excluded.kind, metadata=excluded.metadata",
            )?;
            stmt.execute(params![graph, id, kind, meta_str])?;
            Ok(MutationResult {
                version: None,
                deleted: false,
            })
        }
        BatchOperation::GraphDeleteNode { graph, id } => {
            validate_name("graph", graph)?;
            validate_name("id", id)?;
            let mut stmt_edges = tx.prepare_cached(
                "DELETE FROM _lilia_graph_edges WHERE graph = ?1 AND (source = ?2 OR target = ?2)",
            )?;
            stmt_edges.execute(params![graph, id])?;
            let mut stmt =
                tx.prepare_cached("DELETE FROM _lilia_graph_nodes WHERE graph = ?1 AND id = ?2")?;
            let deleted = stmt.execute(params![graph, id])? > 0;
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
        BatchOperation::GraphAddEdge {
            graph,
            source,
            target,
            kind,
        } => {
            validate_name("graph", graph)?;
            validate_name("source", source)?;
            validate_name("target", target)?;
            let mut stmt = tx.prepare_cached(
                "INSERT OR IGNORE INTO _lilia_graph_edges(graph, source, target, kind) VALUES(?1, ?2, ?3, ?4)",
            )?;
            stmt.execute(params![graph, source, target, kind])?;
            Ok(MutationResult {
                version: None,
                deleted: false,
            })
        }
        BatchOperation::GraphRemoveEdge {
            graph,
            source,
            target,
            kind,
        } => {
            validate_name("graph", graph)?;
            validate_name("source", source)?;
            validate_name("target", target)?;
            let mut stmt = tx.prepare_cached(
                "DELETE FROM _lilia_graph_edges WHERE graph = ?1 AND source = ?2 AND target = ?3 AND kind = ?4",
            )?;
            let deleted = stmt.execute(params![graph, source, target, kind])? > 0;
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
        BatchOperation::FtsIndex {
            collection,
            doc_id,
            content,
        } => {
            validate_name("collection", collection)?;
            validate_name("doc_id", doc_id)?;
            let mut stmt_del =
                tx.prepare_cached("DELETE FROM _lilia_fts WHERE collection = ?1 AND doc_id = ?2")?;
            stmt_del.execute(params![collection, doc_id])?;
            let mut stmt_ins = tx.prepare_cached(
                "INSERT INTO _lilia_fts(collection, doc_id, content) VALUES(?1, ?2, ?3)",
            )?;
            stmt_ins.execute(params![collection, doc_id, content])?;
            Ok(MutationResult {
                version: None,
                deleted: false,
            })
        }
        BatchOperation::FtsDelete { collection, doc_id } => {
            validate_name("collection", collection)?;
            validate_name("doc_id", doc_id)?;
            let mut stmt_del =
                tx.prepare_cached("DELETE FROM _lilia_fts WHERE collection = ?1 AND doc_id = ?2")?;
            let deleted = stmt_del.execute(params![collection, doc_id])? > 0;
            Ok(MutationResult {
                version: None,
                deleted,
            })
        }
    }
}

fn delete_json_indexes(tx: &Transaction<'_>, space: &str, doc_id: &str) -> Result<()> {
    let mut stmt = tx
        .prepare_cached("DELETE FROM _lilia_json_index_entries WHERE space = ?1 AND doc_id = ?2")?;
    stmt.execute(params![space, doc_id])?;
    Ok(())
}

fn update_json_indexes(
    tx: &Transaction<'_>,
    space: &str,
    doc_id: &str,
    value: &serde_json::Value,
) -> Result<()> {
    delete_json_indexes(tx, space, doc_id)?;
    let mut stmt = tx
        .prepare_cached("SELECT index_name, json_path FROM _lilia_json_indexes WHERE space = ?1")?;
    let indexes: Vec<(String, String)> = stmt
        .query_map([space], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if indexes.is_empty() {
        return Ok(());
    }
    let mut insert_stmt = tx.prepare_cached(
        "INSERT INTO _lilia_json_index_entries(space, index_name, idx_value, doc_id) VALUES(?1, ?2, ?3, ?4)",
    )?;
    for (index_name, json_path) in indexes {
        let path = if let Some(stripped) = json_path.strip_prefix("$.") {
            format!("/{}", stripped.replace('.', "/"))
        } else if !json_path.starts_with('/') {
            format!("/{json_path}")
        } else {
            json_path.clone()
        };
        if let Some(target_val) = value.pointer(&path) {
            let val_str = match target_val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Null => "null".to_string(),
                _ => target_val.to_string(),
            };
            insert_stmt.execute(params![space, index_name, val_str, doc_id])?;
        }
    }
    Ok(())
}

fn kv_version(tx: &Transaction<'_>, namespace: &str, key: &[u8]) -> Result<Option<u64>> {
    let mut stmt =
        tx.prepare_cached("SELECT version FROM _lilia_kv WHERE namespace = ?1 AND key = ?2")?;
    stmt.query_row(params![namespace, key], |row| row_version(row, 0))
        .optional()
        .map_err(Into::into)
}

fn json_version(tx: &Transaction<'_>, space: &str, id: &str) -> Result<Option<u64>> {
    let mut stmt =
        tx.prepare_cached("SELECT version FROM _lilia_json WHERE space = ?1 AND id = ?2")?;
    stmt.query_row(params![space, id], |row| row_version(row, 0))
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
