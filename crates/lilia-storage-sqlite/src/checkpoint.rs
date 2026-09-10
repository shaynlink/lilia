use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lilia_core::{ErrorCode, LiliaError, Result};
use rusqlite::Connection;

use crate::engine::CheckpointPolicy;

/// Background attempts only; explicit checkpoints are not included.
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckpointStats {
    pub attempts: u64,
    pub incomplete: u64,
    pub log_frames: i64,
    pub checkpointed_frames: i64,
    pub last_error: Option<ErrorCode>,
}

#[derive(Debug)]
pub(crate) struct Checkpointer {
    wake: Option<mpsc::SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
    stats: Arc<Mutex<CheckpointStats>>,
}

impl Checkpointer {
    pub(crate) fn start(path: &Path, policy: CheckpointPolicy, synchronous: &str) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::ZERO)?;
        connection.pragma_update(None, "synchronous", synchronous)?;
        connection.pragma_update(None, "wal_autocheckpoint", 0)?;
        // Initialize this connection's WAL view before inspecting checkpoint state.
        connection.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })?;
        let page_size =
            u64::from(connection.query_row("PRAGMA page_size", [], |row| row.get::<_, u32>(0))?);
        let stats = Arc::new(Mutex::new(CheckpointStats::default()));
        let (wake, receive) = mpsc::sync_channel(1);
        let shared = Arc::clone(&stats);
        let worker = thread::Builder::new()
            .name("lilia-checkpoint".into())
            .spawn(move || run(&connection, page_size, policy, &receive, &shared))
            .map_err(|error| LiliaError::new(ErrorCode::Io, error.to_string(), false))?;
        let service = Self {
            wake: Some(wake),
            worker: Some(worker),
            stats,
        };
        // Also inspect WAL recovered at open, without waiting for a new mutation.
        service.notify();
        Ok(service)
    }

    pub(crate) fn notify(&self) {
        if let Some(wake) = &self.wake {
            // One pending notification is enough. Never block a committed writer.
            let _ = wake.try_send(());
        }
    }

    pub(crate) fn stats(&self) -> CheckpointStats {
        *self
            .stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for Checkpointer {
    fn drop(&mut self) {
        // Disconnect wakes recv_timeout even for a very long configured interval.
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run(
    connection: &Connection,
    page_size: u64,
    policy: CheckpointPolicy,
    receive: &mpsc::Receiver<()>,
    stats: &Mutex<CheckpointStats>,
) {
    let interval = Duration::from_millis(policy.interval_ms);
    let mut tick = Instant::now();
    let mut defer_until_tick = false;
    loop {
        if matches!(
            receive.recv_timeout(interval.saturating_sub(tick.elapsed())),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ) {
            return;
        }
        let elapsed = tick.elapsed() >= interval;
        if elapsed {
            tick = Instant::now();
        }
        if defer_until_tick && !elapsed {
            continue;
        }
        let result = maybe_checkpoint(connection, page_size, policy.wal_bytes, elapsed);
        let mut stats = stats
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok(Some((busy, log, copied))) => {
                stats.attempts = stats.attempts.saturating_add(1);
                stats.log_frames = log;
                stats.checkpointed_frames = copied;
                stats.last_error = None;
                defer_until_tick = busy != 0 || (log >= 0 && copied < log);
                if defer_until_tick {
                    stats.incomplete = stats.incomplete.saturating_add(1);
                }
            }
            Ok(None) => {}
            Err(error) => {
                stats.last_error = Some(LiliaError::from(error).code);
                defer_until_tick = true;
            }
        }
    }
}

fn maybe_checkpoint(
    connection: &Connection,
    page_size: u64,
    limit: u64,
    elapsed: bool,
) -> rusqlite::Result<Option<(i64, i64, i64)>> {
    let (_, log, copied) = state(connection, "PRAGMA wal_checkpoint(NOOP)")?;
    let outstanding = u64::try_from(log.saturating_sub(copied)).unwrap_or(0);
    // WAL frames contain a page plus a 24-byte header. Allocated file length may
    // stay large after checkpoint/reuse and must not trigger repeated checkpoints.
    if !elapsed && outstanding.saturating_mul(page_size.saturating_add(24)) < limit {
        return Ok(None);
    }
    state(connection, "PRAGMA wal_checkpoint(PASSIVE)").map(Some)
}

fn state(connection: &Connection, pragma: &str) -> rusqlite::Result<(i64, i64, i64)> {
    connection.query_row(pragma, [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
}

#[cfg(test)]
#[path = "checkpoint_tests.rs"]
mod tests;
