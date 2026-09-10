use std::io;

use lilia_core::{BatchOperation, JsonEntry, KvEntry, LiliaError, MutationResult};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 0;
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub token: String,
    /// Optional relative admission budget, starting after the full frame is decoded.
    /// Includes waiting for a daemon blocking worker, but does not cancel running storage work.
    pub deadline_ms: Option<u64>,
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Operation {
    Handshake {
        major: u16,
        minor: u16,
    },
    Shutdown,
    IntegrityCheck,
    Backup {
        destination: String,
    },
    KvGet {
        namespace: String,
        #[serde(with = "serde_bytes")]
        key: Vec<u8>,
    },
    KvScan {
        namespace: String,
        after: Option<Vec<u8>>,
        limit: u32,
    },
    JsonGet {
        space: String,
        id: String,
    },
    JsonScan {
        space: String,
        after: Option<String>,
        limit: u32,
    },
    Batch {
        operations: Vec<BatchOperation>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub result: std::result::Result<ResponseValue, LiliaError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ResponseValue {
    Handshake {
        major: u16,
        minor: u16,
        capabilities: Vec<String>,
    },
    Shutdown,
    Integrity(bool),
    Ack,
    Kv(Option<KvEntry>),
    KvPage(Vec<KvEntry>),
    Json(Option<JsonEntry>),
    JsonPage(Vec<JsonEntry>),
    Mutations(Vec<MutationResult>),
}

/// Decode one size-prefixed `MessagePack` frame.
///
/// # Errors
/// Returns an I/O error for truncated, oversized, empty, or malformed frames.
pub async fn read_frame<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
) -> io::Result<T> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    rmp_serde::from_slice(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Encode and write one size-prefixed `MessagePack` frame.
///
/// # Errors
/// Returns an I/O error when serialization or writing fails, or when the frame is oversized.
pub async fn write_frame<T: Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &T,
) -> io::Result<()> {
    let bytes = rmp_serde::to_vec_named(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds size limit",
        ));
    }
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame exceeds u32 length"))?;
    stream.write_u32(length).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await
}
