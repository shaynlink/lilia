use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{message}")]
pub struct LiliaError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub request_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Closed,
    Busy,
    Conflict,
    Corrupt,
    DiskFull,
    InvalidInput,
    Io,
    NotFound,
    Storage,
    Timeout,
    Unauthorized,
    PluginIncompatible,
    PluginMigrationFailed,
    Unsupported,
}

impl LiliaError {
    pub fn new(code: ErrorCode, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            details: None,
            request_id: Uuid::new_v4(),
        }
    }

    #[must_use]
    pub fn with_request_id(mut self, request_id: Uuid) -> Self {
        self.request_id = request_id;
        self
    }

    #[must_use]
    pub fn details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[cfg(feature = "sqlite-errors")]
impl From<rusqlite::Error> for LiliaError {
    fn from(error: rusqlite::Error) -> Self {
        let code = match error {
            rusqlite::Error::SqliteFailure(ref inner, _) => match inner.code {
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                    ErrorCode::Busy
                }
                rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase => {
                    ErrorCode::Corrupt
                }
                rusqlite::ErrorCode::DiskFull => ErrorCode::DiskFull,
                _ => ErrorCode::Storage,
            },
            _ => ErrorCode::Storage,
        };
        Self::new(code, error.to_string(), code == ErrorCode::Busy)
    }
}

pub type Result<T> = std::result::Result<T, LiliaError>;
