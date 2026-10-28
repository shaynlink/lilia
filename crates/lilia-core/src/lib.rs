mod codec;
mod error;
mod types;
mod version;

pub use codec::{Aes256GcmCodec, IdentityCodec, ValueCodec};
pub use error::{ErrorCode, LiliaError, Result};
pub use types::{
    BatchOperation, DatabaseEvent, Direction, Durability, FtsHit, GraphEdge, GraphNode, JsonEntry,
    JsonIndexInfo, KvEntry, MutationResult, SqlQueryResult,
};

#[cfg(windows)]
pub mod windows_security;

/// Synchronizes directory metadata to persistent storage.
///
/// On Unix, opens the directory with `File::open` and invokes `fsync` via `sync_all`.
/// On Windows, opens the directory handle via `CreateFileW` with `FILE_FLAG_BACKUP_SEMANTICS`
/// and flushes it using `FlushFileBuffers`.
///
/// # Errors
///
/// Returns an [`std::io::Error`] if the directory cannot be opened or if the filesystem
/// metadata flush operation fails.
pub fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()
    }
    #[cfg(windows)]
    {
        windows_security::sync_directory(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_directory_lifecycle() {
        let temp = std::env::temp_dir().join(format!("lilia-sync-core-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        assert!(sync_directory(&temp).is_ok());

        let nonexistent = temp.join("does_not_exist");
        assert!(sync_directory(&nonexistent).is_err());

        std::fs::remove_dir_all(&temp).unwrap();
    }
}
