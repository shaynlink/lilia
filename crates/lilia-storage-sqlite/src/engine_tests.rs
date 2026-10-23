use super::*;
use std::time::Duration;

fn put(value: u8) -> BatchOperation {
    BatchOperation::KvSet {
        namespace: "queue".into(),
        key: b"key".to_vec(),
        value: vec![value],
        if_version: None,
        expires_at_ms: None,
    }
}

#[test]
fn readers_continue_and_expired_queued_batch_never_mutates() {
    let directory = tempfile::tempdir().unwrap();
    let database =
        Database::open(DatabaseOptions::durable(directory.path().join("db.lilia"))).unwrap();
    database.batch(&[put(1)]).unwrap();
    let mut writer = database.lock_writer().unwrap();
    let transaction = writer.transaction().unwrap();
    apply(&transaction, &put(2), &lilia_core::IdentityCodec).unwrap();
    std::thread::scope(|scope| {
        let reader = scope.spawn(|| database.kv_get("queue", b"key").unwrap().unwrap());
        assert_eq!(reader.join().unwrap().value, vec![1]);
        let queued = scope.spawn(|| {
            database
                .batch_with_deadline(&[put(3)], Some(Instant::now() + Duration::from_millis(50)))
        });
        let error = queued.join().unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert!(error.retryable);
    });
    transaction.commit().unwrap();
    drop(writer);
    assert_eq!(
        database.kv_get("queue", b"key").unwrap().unwrap().value,
        vec![2]
    );
    database.batch(&[put(4)]).unwrap();
    assert!(database.integrity_check().unwrap());
}

#[test]
fn concurrent_batches_are_serialized_and_errors_release_admission() {
    let directory = tempfile::tempdir().unwrap();
    let database =
        Database::open(DatabaseOptions::durable(directory.path().join("db.lilia"))).unwrap();
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for value in 0..8 {
            let database = &database;
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..20 {
                    database.batch(&[put(value)]).unwrap();
                }
            });
        }
    });
    assert_eq!(
        database.kv_get("queue", b"key").unwrap().unwrap().version,
        160
    );
    let invalid = BatchOperation::KvDelete {
        namespace: "queue".into(),
        key: b"key".to_vec(),
        if_version: Some(999),
    };
    assert_eq!(
        database.batch(&[put(9), invalid]).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        database.kv_get("queue", b"key").unwrap().unwrap().version,
        160
    );
    database.batch(&[put(10)]).unwrap();
    assert_eq!(
        database.kv_get("queue", b"key").unwrap().unwrap().version,
        161
    );
}

#[test]
fn close_releases_connections_is_idempotent_and_preserves_commits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    database.batch(&[put(1)]).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| database.close(Duration::from_secs(5)).unwrap());
        }
    });
    database.close(Duration::ZERO).unwrap();
    assert!(database.resources.writer.lock().unwrap().is_none());
    assert!(database.resources.checkpointer.lock().unwrap().is_none());
    assert!(database
        .resources
        .readers
        .iter()
        .all(|reader| reader.lock().unwrap().is_none()));
    assert_eq!(
        database.batch(&[put(2)]).unwrap_err().code,
        ErrorCode::Closed
    );
    assert_eq!(
        database.kv_get("queue", b"key").unwrap_err().code,
        ErrorCode::Closed
    );
    let backup = directory.path().join("closed-backup.lilia");
    assert_eq!(
        database.backup(&backup).unwrap_err().code,
        ErrorCode::Closed
    );
    assert!(!backup.exists());
    let reopened = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert_eq!(
        reopened.kv_get("queue", b"key").unwrap().unwrap().value,
        vec![1]
    );
    assert!(reopened.integrity_check().unwrap());
    reopened.close(Duration::from_secs(5)).unwrap();
    directory.close().unwrap();
}

#[test]
fn close_timeout_keeps_accepted_writer_queued_and_closes_after_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    let held = database.lock_writer().unwrap();
    std::thread::scope(|scope| {
        let queued = scope.spawn(|| database.batch(&[put(7)]).unwrap());
        database.resources.writer_queue.wait_for_waiters(1);
        let error = database.close(Duration::from_millis(10)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert!(error.retryable);
        assert_eq!(
            database.batch(&[put(8)]).unwrap_err().code,
            ErrorCode::Closed
        );
        drop(held);
        queued.join().unwrap();
        database.close(Duration::from_secs(5)).unwrap();
    });
    let reopened = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert_eq!(
        reopened.kv_get("queue", b"key").unwrap().unwrap().value,
        vec![7]
    );
}

#[test]
fn close_drains_readers_and_tolerates_external_wal_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    database.batch(&[put(1)]).unwrap();
    let reader = database.read_lock().unwrap();
    assert_eq!(
        database.close(Duration::from_millis(10)).unwrap_err().code,
        ErrorCode::Timeout
    );
    assert!(reader
        .query_row("SELECT count(*) FROM _lilia_kv", [], |row| row
            .get::<_, i64>(0))
        .is_ok());
    let external = Connection::open(&path).unwrap();
    external
        .execute_batch("BEGIN; SELECT * FROM _lilia_kv;")
        .unwrap();
    drop(reader);
    database.close(Duration::from_secs(5)).unwrap();
    external.execute_batch("ROLLBACK").unwrap();
    let reopened = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert!(reopened.integrity_check().unwrap());
}

#[test]
fn in_memory_database_executes_batches_and_queries() {
    let database = Database::open(DatabaseOptions::in_memory()).unwrap();
    database.batch(&[put(42)]).unwrap();
    let entry = database.kv_get("queue", b"key").unwrap().unwrap();
    assert_eq!(entry.value, vec![42]);
    assert!(database.quick_check().unwrap());
    assert!(database.integrity_check().unwrap());
    database.close(Duration::from_secs(1)).unwrap();
}

#[test]
fn defensive_mode_rejects_direct_schema_writes() {
    let database = Database::open(DatabaseOptions::in_memory()).unwrap();
    let writer = database.lock_writer().unwrap();
    let err = writer
        .execute(
            "UPDATE sqlite_schema SET sql = 'corrupt' WHERE name = '_lilia_kv'",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("may not be modified")
            || err.to_string().contains("not authorized")
    );
}

#[test]
fn deadline_watchdog_interrupts_active_batch_and_rolls_back() {
    let database = Database::open(DatabaseOptions::in_memory()).unwrap();
    database.batch(&[put(1)]).unwrap();

    let ops: Vec<BatchOperation> = (0..1000)
        .map(|i| BatchOperation::KvSet {
            namespace: "slow".into(),
            key: format!("k{i}").into_bytes(),
            value: vec![0u8; 10_000],
            if_version: None,
            expires_at_ms: None,
        })
        .collect();

    let err = database
        .batch_with_deadline(&ops, Some(Instant::now() + Duration::from_nanos(1)))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Timeout);
    assert!(err.retryable);

    assert_eq!(
        database.kv_get("queue", b"key").unwrap().unwrap().value,
        vec![1]
    );
    assert!(database.quick_check().unwrap());
}
