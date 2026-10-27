use std::fs;
use std::path::Path;

use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::{json, Value};

use base64::Engine;

use crate::args::{Command, Encoding, FtsCommand, GraphCommand, JsonCommand, KvCommand};

pub(crate) fn resolve_plugin_path(plugin: &Path) -> anyhow::Result<std::path::PathBuf> {
    if plugin.exists() {
        if plugin.join("package").join("manifest.json").is_file() {
            return Ok(plugin.join("package"));
        }
        return Ok(plugin.to_path_buf());
    }

    let query = plugin.to_str().unwrap_or_default();
    if let Ok(root) = lilia_plugin_api::default_plugin_root() {
        if let Ok(Some(installed)) = lilia_plugin_api::find_installed(&root, query) {
            return Ok(installed.package_path);
        }
    }

    anyhow::bail!(
        "plugin `{}` not found as a file or directory, and not installed in default plugin store",
        plugin.display()
    );
}

#[allow(unsafe_code)]
pub(crate) fn execute(
    command: Command,
    path: &Path,
    plugin: Option<&Path>,
) -> anyhow::Result<Value> {
    let mut options = DatabaseOptions::durable(path);
    let mut sql_plugin = None;
    let _loaded = if let Some(plugin_arg) = plugin {
        let plugin_path = resolve_plugin_path(plugin_arg)?;
        let loaded = unsafe { lilia_plugin_api::load_plugin(&plugin_path)? };
        if let Some(codec) = loaded.codec() {
            options = options.with_codec(std::sync::Arc::new(codec));
        }
        if let Some(sql) = loaded.sql() {
            sql_plugin = Some(sql);
        }
        Some(loaded)
    } else {
        None
    };
    let database = Database::open(options)?;
    Ok(match command {
        Command::Init => json!({"ok": true, "path": database.path()}),
        Command::Doctor { deep } => json!({
            "ok": database.integrity_check()?,
            "path": database.path(),
            "journal": "wal",
            "synchronous": "full",
            "deep": deep,
            "checkpoint": if deep { database.checkpoint().is_ok() } else { true }
        }),
        Command::Backup { destination } => {
            database.backup(&destination)?;
            json!({"ok": true, "destination": destination})
        }
        Command::Kv { command } => kv(&database, command)?,
        Command::Json { command } => json_model(&database, command)?,
        Command::Graph { command } => graph(&database, command)?,
        Command::Fts { command } => fts(&database, command)?,
        Command::Batch { file } => {
            let operations: Vec<BatchOperation> = serde_json::from_slice(&fs::read(file)?)?;
            serde_json::to_value(database.batch(&operations)?)?
        }
        Command::Sql { query, params } => {
            if let Some(sql) = &sql_plugin {
                sql.validate_query(&query)?;
            }
            let json_params: Vec<Value> = params
                .iter()
                .map(|p| serde_json::from_str(p).unwrap_or_else(|_| Value::String(p.clone())))
                .collect();
            let result = database.sql_query(&query, &json_params)?;
            serde_json::to_value(result)?
        }
        Command::Plugin { .. } | Command::Daemon { .. } => unreachable!(),
    })
}

fn decode_binary(raw: &str, encoding: Encoding, field: &str) -> lilia_core::Result<Vec<u8>> {
    match encoding {
        Encoding::Text => Ok(raw.as_bytes().to_vec()),
        Encoding::Hex => hex::decode(raw).map_err(|error| {
            lilia_core::LiliaError::new(
                lilia_core::ErrorCode::InvalidInput,
                format!("invalid {field} hex encoding: {error}"),
                false,
            )
        }),
        Encoding::Base64 => base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|error| {
                lilia_core::LiliaError::new(
                    lilia_core::ErrorCode::InvalidInput,
                    format!("invalid {field} base64 encoding: {error}"),
                    false,
                )
            }),
    }
}

fn kv(database: &Database, command: KvCommand) -> anyhow::Result<Value> {
    let mutate =
        |operation| Ok::<_, anyhow::Error>(serde_json::to_value(database.batch(&[operation])?)?);
    match command {
        KvCommand::Get {
            namespace,
            key,
            encoding,
            key_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            Ok(serde_json::to_value(database.kv_get(&namespace, &key)?)?)
        }
        KvCommand::Set {
            namespace,
            key,
            value,
            if_version,
            expires_at_ms,
            encoding,
            key_encoding,
            value_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            let value = decode_binary(&value, value_encoding.unwrap_or(encoding), "value")?;
            mutate(BatchOperation::KvSet {
                namespace,
                key,
                value,
                if_version,
                expires_at_ms,
            })
        }
        KvCommand::Delete {
            namespace,
            key,
            if_version,
            encoding,
            key_encoding,
        } => {
            let key = decode_binary(&key, key_encoding.unwrap_or(encoding), "key")?;
            mutate(BatchOperation::KvDelete {
                namespace,
                key,
                if_version,
            })
        }
        KvCommand::Scan {
            namespace,
            after,
            limit,
            encoding,
            key_encoding,
        } => {
            let key_enc = key_encoding.unwrap_or(encoding);
            let after = after
                .as_deref()
                .map(|cursor| decode_binary(cursor, key_enc, "after"))
                .transpose()?;
            Ok(serde_json::to_value(database.kv_scan(
                &namespace,
                after.as_deref(),
                limit,
            )?)?)
        }
    }
}

fn json_model(database: &Database, command: JsonCommand) -> anyhow::Result<Value> {
    let mutate =
        |operation| Ok::<_, anyhow::Error>(serde_json::to_value(database.batch(&[operation])?)?);
    match command {
        JsonCommand::Get { space, id } => {
            Ok(serde_json::to_value(database.json_get(&space, &id)?)?)
        }
        JsonCommand::Put {
            space,
            id,
            value,
            if_version,
        } => mutate(BatchOperation::JsonPut {
            space,
            id,
            value: serde_json::from_str(&value)?,
            if_version,
        }),
        JsonCommand::Delete {
            space,
            id,
            if_version,
        } => mutate(BatchOperation::JsonDelete {
            space,
            id,
            if_version,
        }),
        JsonCommand::Scan {
            space,
            after,
            limit,
        } => Ok(serde_json::to_value(database.json_scan(
            &space,
            after.as_deref(),
            limit,
        )?)?),
    }
}

fn graph(database: &Database, command: GraphCommand) -> anyhow::Result<Value> {
    match command {
        GraphCommand::Node {
            graph,
            id,
            kind,
            metadata,
        } => {
            let meta_val = match metadata {
                Some(s) => serde_json::from_str(&s)?,
                None => json!({}),
            };
            database.graph_set_node(&graph, &id, &kind, &meta_val)?;
            Ok(json!({
                "ok": true,
                "graph": graph,
                "id": id,
                "kind": kind
            }))
        }
        GraphCommand::Edge {
            graph,
            source,
            target,
            kind,
        } => {
            database.graph_add_edge(&graph, &source, &target, &kind)?;
            Ok(json!({
                "ok": true,
                "graph": graph,
                "source": source,
                "target": target,
                "kind": kind
            }))
        }
        GraphCommand::Neighbors {
            graph,
            node,
            direction,
            kind,
        } => {
            let neighbors =
                database.graph_neighbors(&graph, &node, direction.into(), kind.as_deref())?;
            Ok(serde_json::to_value(neighbors)?)
        }
        GraphCommand::Reach {
            graph,
            start_nodes,
            direction,
        } => {
            let reachable = database.graph_reachability(&graph, &start_nodes, direction.into())?;
            Ok(serde_json::to_value(reachable)?)
        }
    }
}

fn fts(database: &Database, command: FtsCommand) -> anyhow::Result<Value> {
    match command {
        FtsCommand::Index {
            collection,
            doc_id,
            content,
        } => {
            database.fts_index(&collection, &doc_id, &content)?;
            Ok(json!({
                "ok": true,
                "collection": collection,
                "doc_id": doc_id
            }))
        }
        FtsCommand::Search {
            collection,
            query,
            limit,
        } => {
            let hits = database.fts_search(&collection, &query, limit)?;
            Ok(serde_json::to_value(hits)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn decode_binary_formats() {
        assert_eq!(
            decode_binary("hello", Encoding::Text, "key").unwrap(),
            b"hello"
        );
        assert_eq!(
            decode_binary("0080ff", Encoding::Hex, "key").unwrap(),
            vec![0, 128, 255]
        );
        assert_eq!(
            decode_binary("AID/", Encoding::Base64, "key").unwrap(),
            vec![0, 128, 255]
        );

        let err_hex = decode_binary("zz", Encoding::Hex, "key").unwrap_err();
        assert_eq!(err_hex.code, lilia_core::ErrorCode::InvalidInput);

        let err_b64 = decode_binary("!!!", Encoding::Base64, "key").unwrap_err();
        assert_eq!(err_b64.code, lilia_core::ErrorCode::InvalidInput);
    }

    #[test]
    fn kv_hex_and_base64_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.lilia");
        let database = Database::open(DatabaseOptions::durable(&path)).unwrap();

        let res = kv(
            &database,
            KvCommand::Set {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                value: "gICAgA==".into(),
                if_version: None,
                expires_at_ms: None,
                encoding: Encoding::Text,
                key_encoding: Some(Encoding::Hex),
                value_encoding: Some(Encoding::Base64),
            },
        )
        .unwrap();
        assert_eq!(res[0]["deleted"], false);

        let entry = kv(
            &database,
            KvCommand::Get {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        assert_eq!(entry["key"], serde_json::json!([0, 128, 255]));
        assert_eq!(entry["value"], serde_json::json!([128, 128, 128, 128]));

        let scan = kv(
            &database,
            KvCommand::Scan {
                namespace: "test_ns".into(),
                after: Some("00".into()),
                limit: 10,
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        let items: Vec<Value> = serde_json::from_value(scan).unwrap();
        assert_eq!(items.len(), 1);

        let del = kv(
            &database,
            KvCommand::Delete {
                namespace: "test_ns".into(),
                key: "0080ff".into(),
                if_version: None,
                encoding: Encoding::Hex,
                key_encoding: None,
            },
        )
        .unwrap();
        assert_eq!(del[0]["deleted"], true);
    }

    #[test]
    fn sql_command_execution() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cli_sql.lilia");

        let init = execute(Command::Init, &db_path, None).unwrap();
        assert_eq!(init["ok"], true);

        // Put a document
        let put = execute(
            Command::Json {
                command: JsonCommand::Put {
                    space: "users".into(),
                    id: "bob".into(),
                    value: "{\"name\":\"Bob\",\"score\":99}".into(),
                    if_version: None,
                },
            },
            &db_path,
            None,
        )
        .unwrap();
        assert_eq!(put[0]["version"], "1");

        // Query via SQL
        let sql_res = execute(
            Command::Sql {
                query: "SELECT id, json_extract(value, '$.score') as score FROM lilia_json WHERE space = 'users'".into(),
                params: vec![],
            },
            &db_path,
            None,
        )
        .unwrap();

        assert_eq!(sql_res["columns"], json!(["id", "score"]));
        assert_eq!(sql_res["rows"], json!([["bob", 99]]));
    }

    #[test]
    fn graph_and_fts_command_execution() {
        use crate::args::DirectionArg;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cli_graph_fts.lilia");

        // Graph node & edge
        execute(
            Command::Graph {
                command: GraphCommand::Node {
                    graph: "deps".into(),
                    id: "app".into(),
                    kind: "binary".into(),
                    metadata: Some(r#"{"version":"1.0"}"#.into()),
                },
            },
            &db_path,
            None,
        )
        .unwrap();

        execute(
            Command::Graph {
                command: GraphCommand::Node {
                    graph: "deps".into(),
                    id: "lib".into(),
                    kind: "library".into(),
                    metadata: None,
                },
            },
            &db_path,
            None,
        )
        .unwrap();

        execute(
            Command::Graph {
                command: GraphCommand::Edge {
                    graph: "deps".into(),
                    source: "app".into(),
                    target: "lib".into(),
                    kind: "uses".into(),
                },
            },
            &db_path,
            None,
        )
        .unwrap();

        let neighbors = execute(
            Command::Graph {
                command: GraphCommand::Neighbors {
                    graph: "deps".into(),
                    node: "app".into(),
                    direction: DirectionArg::Outgoing,
                    kind: None,
                },
            },
            &db_path,
            None,
        )
        .unwrap();
        assert_eq!(neighbors, json!(["lib"]));

        let reach = execute(
            Command::Graph {
                command: GraphCommand::Reach {
                    graph: "deps".into(),
                    start_nodes: vec!["app".into()],
                    direction: DirectionArg::Outgoing,
                },
            },
            &db_path,
            None,
        )
        .unwrap();
        assert_eq!(reach, json!(["app", "lib"]));

        // FTS index & search
        execute(
            Command::Fts {
                command: FtsCommand::Index {
                    collection: "articles".into(),
                    doc_id: "doc1".into(),
                    content: "liliadb high performance engine".into(),
                },
            },
            &db_path,
            None,
        )
        .unwrap();

        let search = execute(
            Command::Fts {
                command: FtsCommand::Search {
                    collection: "articles".into(),
                    query: "engine".into(),
                    limit: 5,
                },
            },
            &db_path,
            None,
        )
        .unwrap();
        let hits = search.as_array().unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["doc_id"], "doc1");
    }

    #[test]
    fn resolve_plugin_path_priority() {
        let dir = tempfile::tempdir().unwrap();
        let direct_file = dir.path().join("libplugin.dylib");
        std::fs::write(&direct_file, b"content").unwrap();

        // 1. Existing file resolves directly
        let resolved = resolve_plugin_path(&direct_file).unwrap();
        assert_eq!(resolved, direct_file);

        // 2. Directory with package/manifest.json resolves to package subdir
        let pkg_dir = dir.path().join("my-plugin");
        let inner_pkg = pkg_dir.join("package");
        std::fs::create_dir_all(&inner_pkg).unwrap();
        std::fs::write(inner_pkg.join("manifest.json"), b"{}").unwrap();
        let resolved_pkg = resolve_plugin_path(&pkg_dir).unwrap();
        assert_eq!(resolved_pkg, inner_pkg);

        // 3. Non-existent path returns descriptive error
        let err =
            resolve_plugin_path(Path::new("definitely-nonexistent-plugin-12345")).unwrap_err();
        assert!(err.to_string().contains("not found as a file or directory"));
    }
}
