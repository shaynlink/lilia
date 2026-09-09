#![allow(clippy::missing_errors_doc)]

use std::path::{Path, PathBuf};
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
}

impl DatabaseOptions {
    pub fn durable(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            durability: Durability::Durable,
            busy_timeout_ms: 5_000,
        }
    }
}

#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    writer: Mutex<Connection>,
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
        Ok(Self {
            path: options.path,
            writer: Mutex::new(connection),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn integrity_check(&self) -> Result<bool> {
        let result: String = self
            .lock()?
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn checkpoint(&self) -> Result<()> {
        self.lock()?
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
        prepare_parent(destination)?;
        let mut output = Connection::open(destination)?;
        secure_database_file(destination)?;
        let source = self.lock()?;
        let backup = rusqlite::backup::Backup::new(&source, &mut output)?;
        backup.run_to_completion(128, std::time::Duration::from_millis(10), None)?;
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
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let results = operations
            .iter()
            .map(|operation| apply(&transaction, operation))
            .collect::<Result<Vec<_>>>()?;
        transaction.commit()?;
        Ok(results)
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer
            .lock()
            .map_err(|_| LiliaError::new(ErrorCode::Storage, "database lock poisoned", false))
    }
}
