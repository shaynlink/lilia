#![allow(clippy::missing_errors_doc, clippy::needless_pass_by_value)]

use std::sync::Arc;

use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use napi::bindgen_prelude::{Buffer, Error, Result, Status};
use napi_derive::napi;

#[napi]
#[derive(Debug)]
pub struct NativeDatabase {
    inner: Arc<Database>,
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

#[napi]
impl NativeDatabase {
    #[napi(factory)]
    pub async fn open(path: String) -> Result<Self> {
        let database = tokio::task::spawn_blocking(move || {
            Database::open(DatabaseOptions::durable(path)).map_err(napi_error)
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
