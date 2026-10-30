use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use lilia_core::{BatchOperation, Direction, ErrorCode, LiliaError};
use lilia_storage_sqlite::Database;
use serde::Deserialize;
use serde_json::{json, Value};
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct HttpState {
    pub database: Arc<Database>,
    pub token: String,
    pub shutdown: Arc<Notify>,
}

#[derive(Debug, Deserialize)]
pub struct KvGetRequest {
    pub namespace: String,
    pub key: String,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KvSetRequest {
    pub namespace: String,
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub if_version: Option<u64>,
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KvDeleteRequest {
    pub namespace: String,
    pub key: String,
    #[serde(default)]
    pub if_version: Option<u64>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct KvScanRequest {
    pub namespace: String,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct JsonGetRequest {
    pub space: String,
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonPutRequest {
    pub space: String,
    pub id: String,
    pub value: Value,
    #[serde(default)]
    pub if_version: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct JsonDeleteRequest {
    pub space: String,
    pub id: String,
    #[serde(default)]
    pub if_version: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct JsonScanRequest {
    pub space: String,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct JsonCreateIndexRequest {
    pub space: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonDropIndexRequest {
    pub space: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonListIndexesRequest {
    pub space: String,
}

#[derive(Debug, Deserialize)]
pub struct JsonScanByIndexRequest {
    pub space: String,
    pub index_name: String,
    pub value: String,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct JsonFindRequest {
    pub space: String,
    pub query: serde_json::Map<String, Value>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct GraphNodeRequest {
    pub graph: String,
    pub id: String,
    #[serde(default = "default_node_kind")]
    pub kind: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

fn default_node_kind() -> String {
    "default".into()
}

#[derive(Debug, Deserialize)]
pub struct GraphEdgeRequest {
    pub graph: String,
    pub source: String,
    pub target: String,
    #[serde(default = "default_edge_kind")]
    pub kind: String,
}

fn default_edge_kind() -> String {
    "edge".into()
}

#[derive(Debug, Deserialize)]
pub struct GraphNeighborsRequest {
    pub graph: String,
    pub node: String,
    #[serde(default)]
    pub direction: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GraphReachabilityRequest {
    pub graph: String,
    pub start_nodes: Vec<String>,
    #[serde(default)]
    pub direction: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FtsIndexRequest {
    pub collection: String,
    pub doc_id: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FtsSearchRequest {
    pub collection: String,
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HttpBatchOperation {
    KvSet {
        #[serde(default, alias = "space")]
        namespace: String,
        key: String,
        value: String,
        #[serde(default)]
        if_version: Option<u64>,
        #[serde(default)]
        expires_at_ms: Option<i64>,
        #[serde(default)]
        encoding: Option<String>,
    },
    KvDelete {
        #[serde(default, alias = "space")]
        namespace: String,
        key: String,
        #[serde(default)]
        if_version: Option<u64>,
        #[serde(default)]
        encoding: Option<String>,
    },
    JsonPut {
        space: String,
        id: String,
        value: Value,
        #[serde(default)]
        if_version: Option<u64>,
    },
    JsonDelete {
        space: String,
        id: String,
        #[serde(default)]
        if_version: Option<u64>,
    },
    GraphSetNode {
        graph: String,
        id: String,
        kind: String,
        #[serde(default)]
        metadata: Option<Value>,
    },
    GraphDeleteNode {
        graph: String,
        id: String,
    },
    GraphAddEdge {
        graph: String,
        source: String,
        target: String,
        kind: String,
    },
    GraphRemoveEdge {
        graph: String,
        source: String,
        target: String,
        kind: String,
    },
    FtsIndex {
        collection: String,
        doc_id: String,
        content: String,
    },
    FtsDelete {
        collection: String,
        doc_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct BatchRequest {
    pub operations: Vec<HttpBatchOperation>,
}

#[derive(Debug, Deserialize)]
pub struct SqlRequest {
    pub query: String,
    #[serde(default)]
    pub params: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub struct BackupRequest {
    pub destination: String,
}

#[allow(clippy::needless_pass_by_value)]
fn map_error(error: LiliaError) -> (StatusCode, Json<Value>) {
    let status = match error.code {
        ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::Timeout => StatusCode::REQUEST_TIMEOUT,
        ErrorCode::Unsupported => StatusCode::NOT_IMPLEMENTED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(json!({
            "ok": false,
            "error": error
        })),
    )
}

fn decode_bytes(input: &str, encoding: Option<&str>, field: &str) -> lilia_core::Result<Vec<u8>> {
    use base64::Engine;
    match encoding.unwrap_or("text") {
        "text" => Ok(input.as_bytes().to_vec()),
        "hex" => hex::decode(input).map_err(|e| {
            LiliaError::new(
                ErrorCode::InvalidInput,
                format!("invalid {field} hex encoding: {e}"),
                false,
            )
        }),
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(input)
            .map_err(|e| {
                LiliaError::new(
                    ErrorCode::InvalidInput,
                    format!("invalid {field} base64 encoding: {e}"),
                    false,
                )
            }),
        other => Err(LiliaError::new(
            ErrorCode::InvalidInput,
            format!("unsupported encoding: {other}"),
            false,
        )),
    }
}

fn encode_bytes(bytes: &[u8], encoding: Option<&str>) -> Value {
    use base64::Engine;
    match encoding.unwrap_or("text") {
        "hex" => Value::String(hex::encode(bytes)),
        "base64" => Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => Value::String(s.to_owned()),
            Err(_) => Value::String(base64::engine::general_purpose::STANDARD.encode(bytes)),
        },
    }
}

async fn auth_middleware(
    State(state): State<HttpState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<Value>)> {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|val| val.to_str().ok());

    let authenticated = if let Some(header_val) = auth_header {
        if let Some(token) = header_val.strip_prefix("Bearer ") {
            token.as_bytes().ct_eq(state.token.as_bytes()).into()
        } else {
            false
        }
    } else if let Some(query) = request.uri().query() {
        query.split('&').any(|pair| {
            if let Some((k, v)) = pair.split_once('=') {
                k == "token" && v.as_bytes().ct_eq(state.token.as_bytes()).into()
            } else {
                false
            }
        })
    } else {
        false
    };

    if !authenticated {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "AUTHENTICATION",
                    "message": "invalid or missing bearer token",
                    "retryable": false
                }
            })),
        ));
    }

    Ok(next.run(request).await)
}

// Health check endpoint (public, unauthenticated)
async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "status": "running",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

// KV Handlers
async fn kv_get_handler(
    State(state): State<HttpState>,
    Json(payload): Json<KvGetRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let key = decode_bytes(&payload.key, payload.encoding.as_deref(), "key").map_err(map_error)?;
    let db = state.database.clone();
    let entry = tokio::task::spawn_blocking(move || db.kv_get(&payload.namespace, &key))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    match entry {
        Some(e) => {
            let val = encode_bytes(&e.value, payload.encoding.as_deref());
            let k = encode_bytes(&e.key, payload.encoding.as_deref());
            Ok(Json(json!({
                "ok": true,
                "entry": {
                    "namespace": e.namespace,
                    "key": k,
                    "value": val,
                    "version": e.version.to_string(),
                    "expires_at_ms": e.expires_at_ms
                }
            })))
        }
        None => Ok(Json(json!({
            "ok": true,
            "entry": null
        }))),
    }
}

async fn kv_set_handler(
    State(state): State<HttpState>,
    Json(payload): Json<KvSetRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let key = decode_bytes(&payload.key, payload.encoding.as_deref(), "key").map_err(map_error)?;
    let value =
        decode_bytes(&payload.value, payload.encoding.as_deref(), "value").map_err(map_error)?;
    let db = state.database.clone();
    let op = BatchOperation::KvSet {
        namespace: payload.namespace,
        key,
        value,
        if_version: payload.if_version,
        expires_at_ms: payload.expires_at_ms,
    };

    let results = tokio::task::spawn_blocking(move || db.batch(&[op]))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let version = results
        .first()
        .and_then(|r| r.version)
        .map(|v| v.to_string());
    Ok(Json(json!({
        "ok": true,
        "version": version
    })))
}

async fn kv_delete_handler(
    State(state): State<HttpState>,
    Json(payload): Json<KvDeleteRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let key = decode_bytes(&payload.key, payload.encoding.as_deref(), "key").map_err(map_error)?;
    let db = state.database.clone();
    let op = BatchOperation::KvDelete {
        namespace: payload.namespace,
        key,
        if_version: payload.if_version,
    };

    let results = tokio::task::spawn_blocking(move || db.batch(&[op]))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let deleted = results.first().is_some_and(|r| r.deleted);
    Ok(Json(json!({
        "ok": true,
        "deleted": deleted
    })))
}

async fn kv_scan_handler(
    State(state): State<HttpState>,
    Json(payload): Json<KvScanRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let after = payload
        .after
        .as_deref()
        .map(|a| decode_bytes(a, payload.encoding.as_deref(), "after"))
        .transpose()
        .map_err(map_error)?;
    let limit = payload.limit.unwrap_or(100);
    let db = state.database.clone();

    let entries = tokio::task::spawn_blocking(move || {
        db.kv_scan(&payload.namespace, after.as_deref(), limit)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    let formatted: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "namespace": e.namespace,
                "key": encode_bytes(&e.key, payload.encoding.as_deref()),
                "value": encode_bytes(&e.value, payload.encoding.as_deref()),
                "version": e.version.to_string(),
                "expires_at_ms": e.expires_at_ms
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "entries": formatted,
        "count": formatted.len()
    })))
}

// JSON Handlers
async fn json_get_path_handler(
    State(state): State<HttpState>,
    Path((space, id)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let doc = tokio::task::spawn_blocking(move || db.json_get(&space, &id))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    match doc {
        Some(doc) => Ok(Json(json!({
            "ok": true,
            "document": {
                "space": doc.space,
                "id": doc.id,
                "value": doc.value,
                "version": doc.version.to_string()
            }
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": {
                    "code": "NOT_FOUND",
                    "message": "document not found",
                    "retryable": false
                }
            })),
        )),
    }
}

async fn json_get_post_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonGetRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let doc = tokio::task::spawn_blocking(move || db.json_get(&payload.space, &payload.id))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    match doc {
        Some(doc) => Ok(Json(json!({
            "ok": true,
            "document": {
                "space": doc.space,
                "id": doc.id,
                "value": doc.value,
                "version": doc.version.to_string()
            }
        }))),
        None => Ok(Json(json!({
            "ok": true,
            "document": null
        }))),
    }
}

async fn json_put_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonPutRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let op = BatchOperation::JsonPut {
        space: payload.space,
        id: payload.id,
        value: payload.value,
        if_version: payload.if_version,
    };

    let results = tokio::task::spawn_blocking(move || db.batch(&[op]))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let version = results
        .first()
        .and_then(|r| r.version)
        .map(|v| v.to_string());
    Ok(Json(json!({
        "ok": true,
        "version": version
    })))
}

async fn json_delete_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonDeleteRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let op = BatchOperation::JsonDelete {
        space: payload.space,
        id: payload.id,
        if_version: payload.if_version,
    };

    let results = tokio::task::spawn_blocking(move || db.batch(&[op]))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let deleted = results.first().is_some_and(|r| r.deleted);
    Ok(Json(json!({
        "ok": true,
        "deleted": deleted
    })))
}

async fn json_scan_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonScanRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = payload.limit.unwrap_or(100);
    let db = state.database.clone();

    let docs = tokio::task::spawn_blocking(move || {
        db.json_scan(&payload.space, payload.after.as_deref(), limit)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    let formatted: Vec<Value> = docs
        .iter()
        .map(|d| {
            json!({
                "space": d.space,
                "id": d.id,
                "value": d.value,
                "version": d.version.to_string()
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "documents": formatted,
        "count": formatted.len()
    })))
}

async fn json_create_index_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonCreateIndexRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    tokio::task::spawn_blocking(move || {
        db.create_json_index(&payload.space, &payload.name, &payload.path)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true })))
}

async fn json_drop_index_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonDropIndexRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let deleted =
        tokio::task::spawn_blocking(move || db.drop_json_index(&payload.space, &payload.name))
            .await
            .map_err(|_| {
                map_error(LiliaError::new(
                    ErrorCode::Storage,
                    "worker task failed",
                    false,
                ))
            })?
            .map_err(map_error)?;

    Ok(Json(json!({ "ok": true, "deleted": deleted })))
}

async fn json_list_indexes_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonListIndexesRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let indexes = tokio::task::spawn_blocking(move || db.list_json_indexes(&payload.space))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let formatted: Vec<Value> = indexes
        .into_iter()
        .map(|idx| {
            json!({
                "space": idx.space,
                "index_name": idx.index_name,
                "json_path": idx.json_path
            })
        })
        .collect();

    Ok(Json(json!({ "ok": true, "indexes": formatted })))
}

async fn json_scan_by_index_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonScanByIndexRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = payload.limit.unwrap_or(100);
    let db = state.database.clone();
    let docs = tokio::task::spawn_blocking(move || {
        db.json_scan_by_index(
            &payload.space,
            &payload.index_name,
            &payload.value,
            payload.after.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    let formatted: Vec<Value> = docs
        .iter()
        .map(|d| {
            json!({
                "space": d.space,
                "id": d.id,
                "value": d.value,
                "version": d.version.to_string()
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "documents": formatted,
        "count": formatted.len()
    })))
}

async fn json_find_handler(
    State(state): State<HttpState>,
    Json(payload): Json<JsonFindRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let limit = payload.limit.unwrap_or(100);
    let db = state.database.clone();
    let docs = tokio::task::spawn_blocking(move || {
        db.json_find(
            &payload.space,
            &payload.query,
            payload.after.as_deref(),
            limit,
        )
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    let formatted: Vec<Value> = docs
        .iter()
        .map(|d| {
            json!({
                "space": d.space,
                "id": d.id,
                "value": d.value,
                "version": d.version.to_string()
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "documents": formatted,
        "count": formatted.len()
    })))
}

// Batch Handler
#[allow(clippy::too_many_lines)]
async fn batch_handler(
    State(state): State<HttpState>,
    Json(payload): Json<BatchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    const MAX_BATCH_OPERATIONS: usize = 10_000;
    if payload.operations.len() > MAX_BATCH_OPERATIONS {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "error": format!("Batch operations count ({}) exceeds maximum limit of {MAX_BATCH_OPERATIONS}", payload.operations.len())
            })),
        ));
    }
    let mut ops = Vec::new();
    for op in payload.operations {
        match op {
            HttpBatchOperation::KvSet {
                namespace,
                key,
                value,
                if_version,
                expires_at_ms,
                encoding,
            } => {
                let key_bytes =
                    decode_bytes(&key, encoding.as_deref(), "key").map_err(map_error)?;
                let value_bytes =
                    decode_bytes(&value, encoding.as_deref(), "value").map_err(map_error)?;
                ops.push(BatchOperation::KvSet {
                    namespace,
                    key: key_bytes,
                    value: value_bytes,
                    if_version,
                    expires_at_ms,
                });
            }
            HttpBatchOperation::KvDelete {
                namespace,
                key,
                if_version,
                encoding,
            } => {
                let key_bytes =
                    decode_bytes(&key, encoding.as_deref(), "key").map_err(map_error)?;
                ops.push(BatchOperation::KvDelete {
                    namespace,
                    key: key_bytes,
                    if_version,
                });
            }
            HttpBatchOperation::JsonPut {
                space,
                id,
                value,
                if_version,
            } => {
                ops.push(BatchOperation::JsonPut {
                    space,
                    id,
                    value,
                    if_version,
                });
            }
            HttpBatchOperation::JsonDelete {
                space,
                id,
                if_version,
            } => {
                ops.push(BatchOperation::JsonDelete {
                    space,
                    id,
                    if_version,
                });
            }
            HttpBatchOperation::GraphSetNode {
                graph,
                id,
                kind,
                metadata,
            } => {
                ops.push(BatchOperation::GraphSetNode {
                    graph,
                    id,
                    kind,
                    metadata: metadata.unwrap_or_else(|| json!({})),
                });
            }
            HttpBatchOperation::GraphDeleteNode { graph, id } => {
                ops.push(BatchOperation::GraphDeleteNode { graph, id });
            }
            HttpBatchOperation::GraphAddEdge {
                graph,
                source,
                target,
                kind,
            } => {
                ops.push(BatchOperation::GraphAddEdge {
                    graph,
                    source,
                    target,
                    kind,
                });
            }
            HttpBatchOperation::GraphRemoveEdge {
                graph,
                source,
                target,
                kind,
            } => {
                ops.push(BatchOperation::GraphRemoveEdge {
                    graph,
                    source,
                    target,
                    kind,
                });
            }
            HttpBatchOperation::FtsIndex {
                collection,
                doc_id,
                content,
            } => {
                ops.push(BatchOperation::FtsIndex {
                    collection,
                    doc_id,
                    content,
                });
            }
            HttpBatchOperation::FtsDelete { collection, doc_id } => {
                ops.push(BatchOperation::FtsDelete { collection, doc_id });
            }
        }
    }

    let db = state.database.clone();
    let results = tokio::task::spawn_blocking(move || db.batch(&ops))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    let formatted: Vec<Value> = results
        .into_iter()
        .map(|r| {
            json!({
                "version": r.version.map(|v| v.to_string()),
                "deleted": r.deleted
            })
        })
        .collect();

    Ok(Json(json!({
        "ok": true,
        "results": formatted
    })))
}

// SQL Handler
async fn sql_handler(
    State(state): State<HttpState>,
    Json(payload): Json<SqlRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let result = tokio::task::spawn_blocking(move || db.sql_query(&payload.query, &payload.params))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    Ok(Json(json!({
        "ok": true,
        "columns": result.columns,
        "rows": result.rows,
        "rows_affected": result.rows_affected
    })))
}

fn parse_direction(dir: Option<&str>) -> Direction {
    match dir {
        Some("incoming") => Direction::Incoming,
        Some("both") => Direction::Both,
        _ => Direction::Outgoing,
    }
}

// Graph Handlers
async fn graph_node_handler(
    State(state): State<HttpState>,
    Json(payload): Json<GraphNodeRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let meta = payload.metadata.unwrap_or_else(|| json!({}));
    tokio::task::spawn_blocking(move || {
        db.graph_set_node(&payload.graph, &payload.id, &payload.kind, &meta)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true })))
}

async fn graph_edge_handler(
    State(state): State<HttpState>,
    Json(payload): Json<GraphEdgeRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    tokio::task::spawn_blocking(move || {
        db.graph_add_edge(
            &payload.graph,
            &payload.source,
            &payload.target,
            &payload.kind,
        )
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true })))
}

async fn graph_neighbors_handler(
    State(state): State<HttpState>,
    Json(payload): Json<GraphNeighborsRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let direction = parse_direction(payload.direction.as_deref());
    let neighbors = tokio::task::spawn_blocking(move || {
        db.graph_neighbors(
            &payload.graph,
            &payload.node,
            direction,
            payload.kind.as_deref(),
        )
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true, "neighbors": neighbors })))
}

async fn graph_reachability_handler(
    State(state): State<HttpState>,
    Json(payload): Json<GraphReachabilityRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let direction = parse_direction(payload.direction.as_deref());
    let reachable = tokio::task::spawn_blocking(move || {
        db.graph_reachability(&payload.graph, &payload.start_nodes, direction)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true, "reachable": reachable })))
}

// FTS Handlers
async fn fts_index_handler(
    State(state): State<HttpState>,
    Json(payload): Json<FtsIndexRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    tokio::task::spawn_blocking(move || {
        db.fts_index(&payload.collection, &payload.doc_id, &payload.content)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true })))
}

async fn fts_search_handler(
    State(state): State<HttpState>,
    Json(payload): Json<FtsSearchRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let limit = payload.limit.unwrap_or(10);
    let hits = tokio::task::spawn_blocking(move || {
        db.fts_search(&payload.collection, &payload.query, limit)
    })
    .await
    .map_err(|_| {
        map_error(LiliaError::new(
            ErrorCode::Storage,
            "worker task failed",
            false,
        ))
    })?
    .map_err(map_error)?;

    Ok(Json(json!({ "ok": true, "hits": hits })))
}

// Doctor Handler
async fn doctor_handler(
    State(state): State<HttpState>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let integrity = tokio::task::spawn_blocking(move || db.integrity_check())
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    Ok(Json(json!({
        "ok": integrity,
        "integrity": integrity
    })))
}

// Backup Handler
async fn backup_handler(
    State(state): State<HttpState>,
    Json(payload): Json<BackupRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db = state.database.clone();
    let dest = PathBuf::from(payload.destination);
    tokio::task::spawn_blocking(move || db.backup(&dest))
        .await
        .map_err(|_| {
            map_error(LiliaError::new(
                ErrorCode::Storage,
                "worker task failed",
                false,
            ))
        })?
        .map_err(map_error)?;

    Ok(Json(json!({
        "ok": true
    })))
}

// Shutdown Handler
async fn shutdown_handler(State(state): State<HttpState>) -> impl IntoResponse {
    state.shutdown.notify_one();
    Json(json!({
        "ok": true,
        "message": "daemon shutting down"
    }))
}

#[derive(Debug, Deserialize)]
pub struct WatchParams {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub token: Option<String>,
}

async fn watch_handler(
    State(state): State<HttpState>,
    Query(params): Query<WatchParams>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.database.subscribe();
    let filter = params.namespace;
    let stream = futures_util::stream::unfold(rx, move |mut rx| {
        let filter = filter.clone();
        async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event.matches_filter(filter.as_deref()) {
                            if let Ok(json_str) = serde_json::to_string(&event) {
                                let sse_event = Event::default().event("mutation").data(json_str);
                                return Some((Ok(sse_event), rx));
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                }
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub fn router(state: HttpState) -> Router {
    let public_routes = Router::new().route("/health", get(health_handler));

    let protected_routes = Router::new()
        // KV
        .route("/v1/kv/get", post(kv_get_handler))
        .route("/v1/kv/set", post(kv_set_handler))
        .route("/v1/kv/delete", post(kv_delete_handler))
        .route("/v1/kv/scan", post(kv_scan_handler))
        // JSON
        .route("/v1/json/{space}/{id}", get(json_get_path_handler))
        .route("/v1/json/get", post(json_get_post_handler))
        .route("/v1/json/put", post(json_put_handler))
        .route("/v1/json/delete", post(json_delete_handler))
        .route("/v1/json/scan", post(json_scan_handler))
        .route("/v1/json/indexes/create", post(json_create_index_handler))
        .route("/v1/json/indexes/drop", post(json_drop_index_handler))
        .route("/v1/json/indexes/list", post(json_list_indexes_handler))
        .route("/v1/json/scan_by_index", post(json_scan_by_index_handler))
        .route("/v1/json/find", post(json_find_handler))
        // Graph
        .route("/v1/graph/node", post(graph_node_handler))
        .route("/v1/graph/edge", post(graph_edge_handler))
        .route("/v1/graph/neighbors", post(graph_neighbors_handler))
        .route("/v1/graph/reachability", post(graph_reachability_handler))
        // FTS
        .route("/v1/fts/index", post(fts_index_handler))
        .route("/v1/fts/search", post(fts_search_handler))
        // Batch
        .route("/v1/batch", post(batch_handler))
        // SQL
        .route("/v1/sql", post(sql_handler))
        // Doctor & Backup
        .route("/v1/doctor", get(doctor_handler))
        .route("/v1/backup", post(backup_handler))
        .route("/v1/shutdown", post(shutdown_handler))
        // Watch (SSE Live CDC Events)
        .route("/v1/watch", get(watch_handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    public_routes.merge(protected_routes).with_state(state)
}

pub async fn serve_http(
    listener: tokio::net::TcpListener,
    database: Arc<Database>,
    token: String,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    let state = HttpState {
        database,
        token,
        shutdown: shutdown.clone(),
    };
    let app = router(state);

    let shutdown_signal = async move {
        shutdown.notified().await;
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await?;

    Ok(())
}
