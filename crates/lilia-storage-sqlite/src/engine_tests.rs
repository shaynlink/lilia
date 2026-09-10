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
    apply(&transaction, &put(2)).unwrap();
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
