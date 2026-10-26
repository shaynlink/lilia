mod codec;
mod error;
mod types;
mod version;

pub use codec::{IdentityCodec, ValueCodec};
pub use error::{ErrorCode, LiliaError, Result};
pub use types::{
    BatchOperation, DatabaseEvent, Direction, Durability, FtsHit, GraphEdge, GraphNode, JsonEntry,
    KvEntry, MutationResult, SqlQueryResult,
};

#[cfg(windows)]
pub mod windows_security;
