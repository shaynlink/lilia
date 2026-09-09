#![allow(clippy::needless_pass_by_value)]

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use base64::Engine;
use clap::Parser;
use lilia_core::{BatchOperation, LiliaError};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::{json, Value};

const MCP_VERSION: &str = "2026-07-28";

#[derive(Debug, Parser)]
#[command(version, about = "LiliaDB MCP stdio server")]
struct Arguments {
    #[arg(long = "database", value_parser = parse_alias)]
    databases: Vec<(String, PathBuf)>,
}

fn parse_alias(value: &str) -> Result<(String, PathBuf), String> {
    let (alias, path) = value.split_once('=').ok_or("expected ALIAS=PATH")?;
    if alias.is_empty()
        || !alias
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("alias must be alphanumeric with dashes or underscores".into());
    }
    Ok((alias.into(), PathBuf::from(path)))
}

fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    if arguments.databases.is_empty() {
        anyhow::bail!("at least one --database ALIAS=PATH is required");
    }
    let databases = arguments
        .databases
        .into_iter()
        .map(|(alias, path)| {
            Database::open(DatabaseOptions::durable(path)).map(|database| (alias, database))
        })
        .collect::<lilia_core::Result<HashMap<_, _>>>()?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                writeln!(
                    stdout,
                    "{}",
                    rpc_error(Value::Null, -32700, &error.to_string())
                )?;
                stdout.flush()?;
                continue;
            }
        };
        if request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| method.starts_with("notifications/"))
        {
            continue;
        }
        let response = handle(&databases, &request);
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle(databases: &HashMap<String, Database>, request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match method {
        "initialize" => rpc_result(
            id,
            json!({
                "protocolVersion": MCP_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "liliadb", "version": env!("CARGO_PKG_VERSION")}
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({"tools": tool_definitions()})),
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call_tool(databases, name, &arguments) {
                Ok(value) => rpc_result(id, tool_result(value, false)),
                Err(error) => rpc_result(id, tool_result(json!({"error": error}), true)),
            }
        }
        _ => rpc_error(id, -32601, "method not found"),
    }
}

fn call_tool(
    databases: &HashMap<String, Database>,
    name: &str,
    args: &Value,
) -> lilia_core::Result<Value> {
    let alias = required_str(args, "database")?;
    let database = databases.get(alias).ok_or_else(|| {
        lilia_core::LiliaError::new(
            lilia_core::ErrorCode::Unauthorized,
            "database alias is not allowed",
            false,
        )
    })?;
    match name {
        "lilia_schema" => Ok(json!({"models": ["kv", "json"], "database": alias})),
        "lilia_kv_get" => {
            let key = decode_base64(required_str(args, "key")?)?;
            serde_json::to_value(database.kv_get(required_str(args, "namespace")?, &key)?)
                .map_err(json_error)
        }
        "lilia_kv_scan" => serde_json::to_value(
            database.kv_scan(
                required_str(args, "namespace")?,
                args.get("after")
                    .and_then(Value::as_str)
                    .map(decode_base64)
                    .transpose()?
                    .as_deref(),
                optional_u32(args, "limit", 100),
            )?,
        )
        .map_err(json_error),
        "lilia_json_get" => serde_json::to_value(
            database.json_get(required_str(args, "space")?, required_str(args, "id")?)?,
        )
        .map_err(json_error),
        "lilia_json_scan" => serde_json::to_value(database.json_scan(
            required_str(args, "space")?,
            args.get("after").and_then(Value::as_str),
            optional_u32(args, "limit", 100),
        )?)
        .map_err(json_error),
        "lilia_batch" => {
            let operations: Vec<BatchOperation> =
                serde_json::from_value(args.get("operations").cloned().unwrap_or(Value::Null))
                    .map_err(json_error)?;
            serde_json::to_value(database.batch(&operations)?).map_err(json_error)
        }
        _ => Err(lilia_core::LiliaError::new(
            lilia_core::ErrorCode::NotFound,
            "unknown LiliaDB tool",
            false,
        )),
    }
}

fn tool_definitions() -> Vec<Value> {
    let string = || json!({"type": "string"});
    vec![
        tool("lilia_schema", json!({"database": string()}), &["database"]),
        tool(
            "lilia_kv_get",
            json!({"database": string(), "namespace": string(), "key": {"type":"string", "description":"base64 key"}}),
            &["database", "namespace", "key"],
        ),
        tool(
            "lilia_kv_scan",
            json!({"database": string(), "namespace": string(), "after": string(), "limit": {"type":"integer", "minimum":1, "maximum":1000}}),
            &["database", "namespace"],
        ),
        tool(
            "lilia_json_get",
            json!({"database": string(), "space": string(), "id": string()}),
            &["database", "space", "id"],
        ),
        tool(
            "lilia_json_scan",
            json!({"database": string(), "space": string(), "after": string(), "limit": {"type":"integer", "minimum":1, "maximum":1000}}),
            &["database", "space"],
        ),
        tool(
            "lilia_batch",
            json!({"database": string(), "operations": {"type":"array", "items":{"type":"object"}, "maxItems":1000}}),
            &["database", "operations"],
        ),
    ]
}

fn tool(name: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": format!("LiliaDB operation {name}"), "inputSchema": {
        "$schema": "https://json-schema.org/draft/2020-12/schema", "type": "object",
        "properties": properties, "required": required, "additionalProperties": false
    }})
}

fn required_str<'a>(value: &'a Value, field: &str) -> lilia_core::Result<&'a str> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        LiliaError::new(
            lilia_core::ErrorCode::InvalidInput,
            format!("missing string {field}"),
            false,
        )
    })
}

fn optional_u32(value: &Value, field: &str, fallback: u32) -> u32 {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(fallback)
}

fn decode_base64(value: &str) -> lilia_core::Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| {
            LiliaError::new(
                lilia_core::ErrorCode::InvalidInput,
                error.to_string(),
                false,
            )
        })
}

fn json_error(error: serde_json::Error) -> LiliaError {
    LiliaError::new(
        lilia_core::ErrorCode::InvalidInput,
        error.to_string(),
        false,
    )
}

fn tool_result(value: Value, is_error: bool) -> Value {
    json!({"content": [{"type":"text", "text": value.to_string()}], "structuredContent": value, "isError": is_error})
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "result":result})
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
}
