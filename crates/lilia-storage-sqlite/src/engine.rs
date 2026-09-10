#![allow(clippy::missing_errors_doc)]

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::checkpoint::{CheckpointStats, Checkpointer};
use crate::lifecycle::{Lifecycle, Operation};
use crate::storage::{migrate, prepare_parent, secure_database_file};
use crate::transaction::apply;
use crate::writer_queue::{Permit, WriterQueue};
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
    lifecycle: Arc<Lifecycle>,
    resources: Arc<Resources>,
}

#[derive(Debug)]
struct Resources {
    // Stop/join maintenance before dropping the database connections.
    checkpointer: Mutex<Option<Checkpointer>>,
    writer: Mutex<Option<Connection>>,
    readers: Vec<Mutex<Option<Connection>>>,
    next_reader: AtomicUsize,
    writer_queue: WriterQueue,
}

pub(crate) struct WriterGuard<'a> {
    // Drop the connection lock before making the next FIFO permit available.
    connection: MutexGuard<'a, Option<Connection>>,
    _permit: Option<Permit<'a>>,
    _operation: Operation<'a>,
}

impl Deref for WriterGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("admitted connection must remain open")
    }
}

impl DerefMut for WriterGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("admitted connection must remain open")
    }
}

impl Database {
    pub fn open(options: DatabaseOptions) -> Result<Self> {
        if options.checkpoint_policy.wal_bytes == 0
            || !(1..=u64::from(u32::MAX)).contains(&options.checkpoint_policy.interval_ms)
        {
            return Err(LiliaError::new(
                ErrorCode::InvalidInput,
                "checkpoint wal_bytes must be positive and interval_ms must be in 1..=4294967295",
                false,
            ));
        }
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
        // Disable SQLite's commit-path checkpoint in favor of background work.
        connection.pragma_update(None, "wal_autocheckpoint", 0)?;
        migrate(&connection)?;
        let reader_count = options.read_pool_size.max(1);
        let mut readers = Vec::with_capacity(reader_count);
        for _ in 0..reader_count {
            let reader = Connection::open(&options.path)?;
            reader.busy_timeout(std::time::Duration::from_millis(options.busy_timeout_ms))?;
            reader.pragma_update(None, "journal_mode", "WAL")?;
            reader.pragma_update(None, "query_only", true)?;
            readers.push(Mutex::new(Some(reader)));
        }
        Ok(Self {
            lifecycle: Arc::new(Lifecycle::default()),
            resources: Arc::new(Resources {
                checkpointer: Mutex::new(Some(Checkpointer::start(
                    &options.path,
                    options.checkpoint_policy,
                    synchronous,
                )?)),
                writer: Mutex::new(Some(connection)),
                readers,
                next_reader: AtomicUsize::new(0),
                writer_queue: WriterQueue::new(options.writer_queue_capacity),
            }),
            path: options.path,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn checkpoint_stats(&self) -> CheckpointStats {
        self.resources
            .checkpointer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(Checkpointer::stats)
            .unwrap_or_default()
    }

    /// Stop admission and wait at most `timeout`; cleanup continues after TIMEOUT.
    pub fn close(&self, timeout: Duration) -> Result<()> {
        let resources = Arc::clone(&self.resources);
        self.lifecycle.close(timeout, move || resources.close())
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
        // Admission covers validation, snapshotting and publication, not only SQL.
        let source = self.lock_writer()?;
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
        let backup = rusqlite::backup::Backup::new(&source, &mut output)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
        drop(backup);
        let integrity: String = output.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
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
        self.batch_with_deadline(operations, None)
    }

    /// Deadline applies to writer admission, not an already executing transaction.
    pub fn batch_with_deadline(
        &self,
        operations: &[BatchOperation],
        deadline: Option<Instant>,
    ) -> Result<Vec<MutationResult>> {
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
        let mut connection = self.writer_lock(deadline)?;
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(LiliaError::new(
                ErrorCode::Timeout,
                "request expired before transaction",
                true,
            ));
        }
        let transaction = connection.transaction()?;
        let results = operations
            .iter()
            .map(|operation| apply(&transaction, operation))
            .collect::<Result<Vec<_>>>()?;
        transaction.commit()?;
        drop(connection);
        if let Some(checkpointer) = self
            .resources
            .checkpointer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            checkpointer.notify();
        }
        Ok(results)
    }

    pub(crate) fn lock_writer(&self) -> Result<WriterGuard<'_>> {
        self.writer_lock(None)
    }

    fn writer_lock(&self, deadline: Option<Instant>) -> Result<WriterGuard<'_>> {
        let operation = self.lifecycle.enter()?;
        let permit = self.resources.writer_queue.acquire(deadline)?;
        let connection =
            self.resources.writer.lock().map_err(|_| {
                LiliaError::new(ErrorCode::Storage, "database lock poisoned", false)
            })?;
        Ok(WriterGuard {
            connection,
            _permit: Some(permit),
            _operation: operation,
        })
    }

    pub(crate) fn read_lock(&self) -> Result<WriterGuard<'_>> {
        let operation = self.lifecycle.enter()?;
        let index = self.resources.next_reader.fetch_add(1, Ordering::Relaxed)
            % self.resources.readers.len();
        let connection = self.resources.readers[index].lock().map_err(|_| {
            LiliaError::new(ErrorCode::Storage, "read database lock poisoned", false)
        })?;
        Ok(WriterGuard {
            connection,
            _permit: None,
            _operation: operation,
        })
    }
}

impl Resources {
    fn close(&self) -> Result<()> {
        // No admitted operations remain. Potentially slow I/O runs on the cleanup
        // thread so the caller's timeout is independent of filesystem progress.
        let worker = self
            .checkpointer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(worker);
        let mut result = Ok(());
        for reader in &self.readers {
            if let Some(reader) = reader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                let closed = reader.close().map_err(|(_, error)| LiliaError::from(error));
                result = result.and(closed);
            }
        }
        if let Some(writer) = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let checkpoint = writer
                .busy_timeout(Duration::ZERO)
                .and_then(|()| writer.execute_batch("PRAGMA wal_checkpoint(PASSIVE)"))
                .map_err(LiliaError::from);
            result = result.and(checkpoint);
            let closed = writer.close().map_err(|(_, error)| LiliaError::from(error));
            result = result.and(closed);
        }
        result
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

// Match Result::map_err's owned error callback without repeating conversion closures.
#[allow(clippy::needless_pass_by_value)]
fn backup_io(error: std::io::Error) -> LiliaError {
    LiliaError::new(ErrorCode::Io, error.to_string(), false)
}
