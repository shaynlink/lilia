#![allow(clippy::missing_errors_doc)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::storage::{migrate, prepare_parent, secure_database_file};
use crate::transaction::apply;
use lilia_core::{BatchOperation, Durability, ErrorCode, LiliaError, MutationResult, Result};

#[derive(Debug, Clone)]
pub struct DatabaseOptions {
    pub path: PathBuf,
    pub durability: Durability,
    pub busy_timeout_ms: u64,
    pub writer_queue_capacity: usize,
    pub read_pool_size: usize,
    pub checkpoint_policy: CheckpointPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointPolicy {
    pub wal_bytes: u64,
    pub interval_ms: u64,
}

impl DatabaseOptions {
    pub fn durable(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            durability: Durability::Durable,
            busy_timeout_ms: 5_000,
            writer_queue_capacity: 64,
            read_pool_size: 4,
            checkpoint_policy: CheckpointPolicy {
                wal_bytes: 16 * 1024 * 1024,
                interval_ms: 5_000,
            },
        }
    }
}

#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
    writer_queue_capacity: usize,
}

impl Database {
    pub fn open(options: DatabaseOptions) -> Result<Self> {
        prepare_parent(&options.path)?;
        let connection = Connection::open(&options.path)?;
        secure_database_file(&options.path)?;
        connection.busy_timeout(std::time::Duration::from_millis(options.busy_timeout_ms))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        let synchronous = match options.durability {
            Durability::Durable => "FULL",
            Durability::Balanced => "NORMAL",
            Durability::Performance => "OFF",
        };
        connection.pragma_update(None, "synchronous", synchronous)?;
        migrate(&connection)?;
        let reader_count = options.read_pool_size.max(1);
        let mut readers = Vec::with_capacity(reader_count);
        for _ in 0..reader_count {
            let reader = Connection::open(&options.path)?;
            reader.busy_timeout(std::time::Duration::from_millis(options.busy_timeout_ms))?;
            reader.pragma_update(None, "journal_mode", "WAL")?;
            reader.pragma_update(None, "query_only", true)?;
            readers.push(Mutex::new(reader));
        }
        Ok(Self {
            path: options.path,
            writer: Mutex::new(connection),
            readers,
            next_reader: AtomicUsize::new(0),
            writer_queue_capacity: options.writer_queue_capacity,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn integrity_check(&self) -> Result<bool> {
        let result: String = self
            .read_lock()?
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.lock_writer()?
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        Ok(())
    }

    pub fn backup(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();
        if destination == self.path {
            return Err(LiliaError::new(
                ErrorCode::InvalidInput,
                "backup destination must differ from the database path",
                false,
            ));
        }
        // Refuse every existing entry, including dangling symlinks and source aliases.
        // Publication below also enforces this atomically against concurrent creators.
        match std::fs::symlink_metadata(destination) {
            Ok(_) => {
                return Err(LiliaError::new(
                    ErrorCode::InvalidInput,
                    "backup destination already exists",
                    false,
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(backup_io(error)),
        }
        prepare_parent(destination)?;
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        // A private sibling directory also protects SQLite's temporary sidecars.
        let staging = tempfile::Builder::new()
            .prefix(".lilia-backup-")
            .tempdir_in(parent)
            .map_err(backup_io)?;
        let temporary = staging.path().join("snapshot.lilia");
        let mut output = Connection::open(&temporary)?;
        secure_database_file(&temporary)?;
        let source = self.lock_writer()?;
        let backup = rusqlite::backup::Backup::new(&source, &mut output)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
        drop(backup);
        let integrity: String = output.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        drop(source);
        output
            .close()
            .map_err(|(_, error)| LiliaError::from(error))?;
        if integrity != "ok" {
            return Err(LiliaError::new(
                ErrorCode::Corrupt,
                "backup integrity check failed",
                false,
            ));
        }
        // Windows FlushFileBuffers requires a handle opened with write access.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .map_err(backup_io)?
            .sync_all()
            .map_err(backup_io)?;
        // Unlike rename on Unix, hard_link never replaces an existing destination.
        // Both names are on the same filesystem; only a complete snapshot becomes visible.
        std::fs::hard_link(&temporary, destination).map_err(backup_io)?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .map_err(backup_io)?
            .sync_all()
            .map_err(backup_io)?;
        Ok(())
    }

    pub fn batch(&self, operations: &[BatchOperation]) -> Result<Vec<MutationResult>> {
        if operations.is_empty() {
            return Err(LiliaError::new(
                ErrorCode::InvalidInput,
                "batch cannot be empty",
                false,
            ));
        }
        if operations.len() > 1_000 {
            return Err(LiliaError::new(
                ErrorCode::InvalidInput,
                "batch exceeds 1000 operations",
                false,
            ));
        }
        let mut connection = self.try_lock_writer()?;
        let transaction = connection.transaction()?;
        let results = operations
            .iter()
            .map(|operation| apply(&transaction, operation))
            .collect::<Result<Vec<_>>>()?;
        transaction.commit()?;
        Ok(results)
    }

    pub(crate) fn lock_writer(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|_| LiliaError::new(ErrorCode::Storage, "database lock poisoned", false))
    }

    pub(crate) fn read_lock(&self) -> Result<MutexGuard<'_, Connection>> {
        let index = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
        self.readers[index]
            .lock()
            .map_err(|_| LiliaError::new(ErrorCode::Storage, "read database lock poisoned", false))
    }

    pub(crate) fn try_lock_writer(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer.try_lock().map_err(|_| {
            LiliaError::new(
                ErrorCode::Busy,
                format!(
                    "writer queue is saturated (capacity {})",
                    self.writer_queue_capacity
                ),
                true,
            )
        })
    }
}

// Match Result::map_err's owned error callback without repeating conversion closures.
#[allow(clippy::needless_pass_by_value)]
fn backup_io(error: std::io::Error) -> LiliaError {
    LiliaError::new(ErrorCode::Io, error.to_string(), false)
}
