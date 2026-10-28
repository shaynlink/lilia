#![allow(
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::unused_async,
    clippy::needless_continue
)]

use std::sync::{Arc, Mutex};

use lilia_core::{BatchOperation, Durability};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use napi::bindgen_prelude::{Buffer, Error, Result, Status};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi_derive::napi;

#[napi]
#[derive(Debug)]
pub struct NativeDatabase {
    inner: Arc<Database>,
}

#[napi]
#[derive(Debug)]
pub struct NativeSubscription {
    cancel_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
}

#[napi]
impl NativeSubscription {
    #[napi]
    pub fn unsubscribe(&self) {
        if let Ok(mut guard) = self.cancel_tx.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }
    }
}

#[napi(object)]
#[derive(Debug)]
pub struct NativeOpenOptions {
    pub path: String,
    pub durability: Option<String>,
    pub busy_timeout_ms: Option<u32>,
    pub writer_queue_capacity: Option<u32>,
    pub read_pool_size: Option<u32>,
    pub checkpoint_policy: Option<NativeCheckpointPolicy>,
    pub encryption_key: Option<String>,
}

#[napi(object)]
#[derive(Debug)]
pub struct NativeCheckpointPolicy {
    pub wal_bytes: Option<u32>,
    pub interval_ms: Option<u32>,
}

#[napi(object)]
pub struct NativeKvEntry {
    pub namespace: String,
    pub key: Buffer,
    pub value: Buffer,
    pub version: String,
    pub expires_at_ms: Option<i64>,
}

impl std::fmt::Debug for NativeKvEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeKvEntry")
            .field("namespace", &self.namespace)
            .field("key_bytes", &self.key.len())
            .field("value_bytes", &self.value.len())
            .field("version", &self.version)
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct NativeFtsHit {
    pub collection: String,
    pub doc_id: String,
    pub snippet: String,
    pub rank: f64,
}

#[napi]
impl NativeDatabase {
    #[napi(factory)]
    pub async fn open(options: NativeOpenOptions) -> Result<Self> {
        let database = tokio::task::spawn_blocking(move || {
            let mut settings = DatabaseOptions::durable(options.path);
            settings.durability = match options.durability.as_deref() {
                Some("balanced") => Durability::Balanced,
                Some("performance") => Durability::Performance,
                _ => Durability::Durable,
            };
            if let Some(value) = options.busy_timeout_ms {
                settings.busy_timeout_ms = u64::from(value);
            }
            if let Some(value) = options.writer_queue_capacity {
                settings.writer_queue_capacity = value as usize;
            }
            if let Some(value) = options.read_pool_size {
                settings.read_pool_size = value as usize;
            }
            if let Some(policy) = options.checkpoint_policy {
                if let Some(value) = policy.wal_bytes {
                    settings.checkpoint_policy.wal_bytes = u64::from(value);
                }
                if let Some(value) = policy.interval_ms {
                    settings.checkpoint_policy.interval_ms = u64::from(value);
                }
            }
            if let Some(key) = options.encryption_key {
                settings = settings
                    .with_codec(Arc::new(lilia_core::Aes256GcmCodec::from_passphrase(&key)));
            }
            Database::open(settings).map_err(napi_error)
        })
        .await
        .map_err(join_error)??;
        Ok(Self {
            inner: Arc::new(database),
        })
    }

    #[napi]
    pub async fn kv_get(&self, namespace: String, key: Buffer) -> Result<Option<NativeKvEntry>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            database
                .kv_get(&namespace, &key)
                .map(|entry| {
                    entry.map(|entry| NativeKvEntry {
                        namespace: entry.namespace,
                        key: entry.key.into(),
                        value: entry.value.into(),
                        version: entry.version.to_string(),
                        expires_at_ms: entry.expires_at_ms,
                    })
                })
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_get(&self, space: String, id: String) -> Result<Option<String>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            database
                .json_get(&space, &id)
                .and_then(|entry| {
                    entry
                        .map(|entry| {
                            serde_json::to_string(&entry).map_err(|error| {
                                lilia_core::LiliaError::new(
                                    lilia_core::ErrorCode::Storage,
                                    error.to_string(),
                                    false,
                                )
                            })
                        })
                        .transpose()
                })
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn kv_scan(
        &self,
        namespace: String,
        after: Option<Buffer>,
        limit: u32,
    ) -> Result<Vec<NativeKvEntry>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            database
                .kv_scan(&namespace, after.as_deref(), limit)
                .map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| NativeKvEntry {
                            namespace: entry.namespace,
                            key: entry.key.into(),
                            value: entry.value.into(),
                            version: entry.version.to_string(),
                            expires_at_ms: entry.expires_at_ms,
                        })
                        .collect()
                })
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_scan(
        &self,
        space: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let entries = database
                .json_scan(&space, after.as_deref(), limit)
                .map_err(napi_error)?;
            serde_json::to_string(&entries)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_create_index(&self, space: String, name: String, path: String) -> Result<()> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            database
                .create_json_index(&space, &name, &path)
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_drop_index(&self, space: String, name: String) -> Result<bool> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            database.drop_json_index(&space, &name).map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_list_indexes(&self, space: String) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let indexes = database.list_json_indexes(&space).map_err(napi_error)?;
            serde_json::to_string(&indexes)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_scan_by_index(
        &self,
        space: String,
        index_name: String,
        value: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let entries = database
                .json_scan_by_index(&space, &index_name, &value, after.as_deref(), limit)
                .map_err(napi_error)?;
            serde_json::to_string(&entries)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn json_find(
        &self,
        space: String,
        query_json: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let query_val: serde_json::Value = serde_json::from_str(&query_json)
                .map_err(|e| Error::new(Status::InvalidArg, e.to_string()))?;
            let query_map = query_val
                .as_object()
                .ok_or_else(|| Error::new(Status::InvalidArg, "query must be a JSON object"))?;
            let entries = database
                .json_find(&space, query_map, after.as_deref(), limit)
                .map_err(napi_error)?;
            serde_json::to_string(&entries)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn batch(&self, operations_json: String) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let operations: Vec<BatchOperation> = serde_json::from_str(&operations_json)
                .map_err(|error| Error::new(Status::InvalidArg, error.to_string()))?;
            let result = database.batch(&operations).map_err(napi_error)?;
            serde_json::to_string(&result)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn integrity_check(&self) -> Result<bool> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || database.integrity_check().map_err(napi_error))
            .await
            .map_err(join_error)?
    }

    #[napi]
    pub async fn backup(&self, destination: String) -> Result<()> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || database.backup(destination).map_err(napi_error))
            .await
            .map_err(join_error)?
    }

    #[napi]
    pub async fn sql_query(&self, sql: String, params_json: Option<String>) -> Result<String> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let params: Vec<serde_json::Value> = match params_json {
                Some(ref s) if !s.trim().is_empty() => serde_json::from_str(s)
                    .map_err(|e| Error::new(Status::InvalidArg, e.to_string()))?,
                _ => Vec::new(),
            };
            let result = database.sql_query(&sql, &params).map_err(napi_error)?;
            serde_json::to_string(&result)
                .map_err(|error| Error::new(Status::GenericFailure, error.to_string()))
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn graph_neighbors(
        &self,
        graph: String,
        node: String,
        direction: Option<String>,
        kind: Option<String>,
    ) -> Result<Vec<String>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let dir = match direction.as_deref() {
                Some("incoming") => lilia_core::Direction::Incoming,
                Some("both") => lilia_core::Direction::Both,
                _ => lilia_core::Direction::Outgoing,
            };
            database
                .graph_neighbors(&graph, &node, dir, kind.as_deref())
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn graph_reachability(
        &self,
        graph: String,
        start_nodes: Vec<String>,
        direction: Option<String>,
    ) -> Result<Vec<String>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let dir = match direction.as_deref() {
                Some("incoming") => lilia_core::Direction::Incoming,
                Some("both") => lilia_core::Direction::Both,
                _ => lilia_core::Direction::Outgoing,
            };
            database
                .graph_reachability(&graph, &start_nodes, dir)
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }

    #[napi]
    pub async fn fts_search(
        &self,
        collection: String,
        query: String,
        limit: Option<u32>,
    ) -> Result<Vec<NativeFtsHit>> {
        let database = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let hits = database
                .fts_search(&collection, &query, limit.unwrap_or(10))
                .map_err(napi_error)?;
            Ok(hits
                .into_iter()
                .map(|h| NativeFtsHit {
                    collection: h.collection,
                    doc_id: h.doc_id,
                    snippet: h.snippet,
                    rank: h.rank,
                })
                .collect())
        })
        .await
        .map_err(join_error)?
    }

    #[napi(ts_args_type = "callback: (event: string) => void, namespace?: string")]
    pub async fn watch(
        &self,
        callback: ThreadsafeFunction<String>,
        namespace: Option<String>,
    ) -> Result<NativeSubscription> {
        let (tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
        let cancel_tx = Arc::new(Mutex::new(Some(tx)));
        let mut rx = self.inner.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut cancel_rx => {
                        break;
                    }
                    evt = rx.recv() => {
                        match evt {
                            Ok(event) => {
                                if event.matches_filter(namespace.as_deref()) {
                                    if let Ok(json) = serde_json::to_string(&event) {
                                        callback.call(Ok(json), ThreadsafeFunctionCallMode::NonBlocking);
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        Ok(NativeSubscription { cancel_tx })
    }

    #[napi]
    pub async fn close(&self, timeout_ms: Option<u32>) -> Result<()> {
        let database = Arc::clone(&self.inner);
        let started = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(u64::from(timeout_ms.unwrap_or(5_000)));
        tokio::task::spawn_blocking(move || {
            database
                .close(timeout.saturating_sub(started.elapsed()))
                .map_err(napi_error)
        })
        .await
        .map_err(join_error)?
    }
}

fn napi_error(error: lilia_core::LiliaError) -> Error {
    Error::new(
        if error.retryable {
            Status::WouldDeadlock
        } else {
            Status::GenericFailure
        },
        serde_json::to_string(&error).unwrap_or_else(|_| error.to_string()),
    )
}

fn join_error(error: tokio::task::JoinError) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
