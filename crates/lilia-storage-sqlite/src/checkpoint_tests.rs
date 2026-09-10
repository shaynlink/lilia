use super::*;
use crate::{Database, DatabaseOptions};
use lilia_core::BatchOperation;

fn wait_stats(
    service: &Checkpointer,
    predicate: impl Fn(CheckpointStats) -> bool,
) -> CheckpointStats {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let stats = service.stats();
        if predicate(stats) {
            return stats;
        }
        assert!(
            Instant::now() < deadline,
            "checkpoint did not progress: {stats:?}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn setup(path: &Path) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection.busy_timeout(Duration::from_millis(50)).unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE data(value); PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    connection
}

#[test]
fn interval_flushes_small_wal_without_new_notifications() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let connection = setup(&path);
    let service = Checkpointer::start(
        &path,
        CheckpointPolicy {
            wal_bytes: u64::MAX,
            interval_ms: 20,
        },
        "FULL",
    )
    .unwrap();
    connection
        .execute("INSERT INTO data VALUES(1)", [])
        .unwrap();
    let stats = wait_stats(&service, |stats| {
        stats.log_frames > 0 && stats.log_frames == stats.checkpointed_frames
    });
    assert_eq!(stats.last_error, None);
    assert_eq!(
        state(&connection, "PRAGMA wal_checkpoint(NOOP)").unwrap().1,
        stats.log_frames
    );
}

#[test]
fn threshold_uses_outstanding_frames_not_allocated_file_length() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let connection = setup(&path);
    connection
        .execute("INSERT INTO data VALUES(zeroblob(32768))", [])
        .unwrap();
    assert!(maybe_checkpoint(&connection, 4096, 1, false)
        .unwrap()
        .is_some());
    assert!(
        std::fs::metadata(path.with_extension("lilia-wal"))
            .unwrap()
            .len()
            > 0
    );
    assert!(maybe_checkpoint(&connection, 4096, 1, false)
        .unwrap()
        .is_none());
}

#[test]
fn reader_starvation_does_not_block_writes_and_retries_on_interval() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let connection = setup(&path);
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN; SELECT * FROM data;").unwrap();
    connection
        .execute("INSERT INTO data VALUES(1)", [])
        .unwrap();
    let service = Checkpointer::start(
        &path,
        CheckpointPolicy {
            wal_bytes: 1,
            interval_ms: 20,
        },
        "FULL",
    )
    .unwrap();
    wait_stats(&service, |stats| stats.incomplete > 0);
    connection
        .execute("INSERT INTO data VALUES(2)", [])
        .unwrap();
    reader.execute_batch("ROLLBACK").unwrap();
    wait_stats(&service, |stats| {
        stats.log_frames > 0 && stats.log_frames == stats.checkpointed_frames
    });
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM data", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn disconnect_stops_worker_even_with_long_interval() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let connection = setup(&path);
    let service = Checkpointer::start(
        &path,
        CheckpointPolicy {
            wal_bytes: u64::MAX,
            interval_ms: 60_000,
        },
        "FULL",
    )
    .unwrap();
    let stats = Arc::clone(&service.stats);
    let started = Instant::now();
    drop(service);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(Arc::strong_count(&stats), 1);
    drop(connection);
    directory.close().unwrap();
}

#[test]
fn engine_disables_commit_checkpoint_and_notifies_background_worker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let mut options = DatabaseOptions::durable(&path);
    options.checkpoint_policy = CheckpointPolicy {
        wal_bytes: 1,
        interval_ms: 60_000,
    };
    let database = Database::open(options).unwrap();
    // Maintenance also handles the migration/recovered WAL before any new batch.
    let deadline = Instant::now() + Duration::from_secs(5);
    while database.checkpoint_stats().attempts == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let baseline = database.checkpoint_stats().attempts;
    assert_eq!(
        database
            .lock_writer()
            .unwrap()
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    database
        .batch(&[BatchOperation::JsonPut {
            space: "test".into(),
            id: "doc".into(),
            value: serde_json::json!({"ok": true}),
            if_version: None,
        }])
        .unwrap();
    while database.checkpoint_stats().attempts == baseline {
        assert!(
            Instant::now() < deadline,
            "committed batch did not wake maintenance"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(database.integrity_check().unwrap());
}

#[test]
fn invalid_policy_fails_before_creating_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    for policy in [
        CheckpointPolicy {
            wal_bytes: 0,
            interval_ms: 10,
        },
        CheckpointPolicy {
            wal_bytes: 1,
            interval_ms: 0,
        },
        CheckpointPolicy {
            wal_bytes: 1,
            interval_ms: u64::MAX,
        },
    ] {
        let mut options = DatabaseOptions::durable(&path);
        options.checkpoint_policy = policy;
        assert_eq!(
            Database::open(options).unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert!(!path.exists());
    }
}
