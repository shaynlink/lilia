use super::*;
use std::io::{Seek, Write};
use std::process::{Child, Command};

const CHILD_PATH: &str = "LILIA_RECOVERY_TEST_PATH";
const CHILD_PHASE: &str = "LILIA_RECOVERY_TEST_PHASE";

fn mutations(id: &str, value: u8) -> [BatchOperation; 2] {
    [
        BatchOperation::KvSet {
            namespace: "recovery".into(),
            key: id.as_bytes().to_vec(),
            value: vec![value; 4096],
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "recovery".into(),
            id: id.into(),
            value: serde_json::json!({"value": value}),
            if_version: None,
        },
    ]
}

// Run only in the child process launched below. Waiting at a handshake makes the
// crash location deterministic; there is no production fault-injection API.
#[test]
#[ignore = "subprocess helper; executed by killed_writer_recovers_confirmed_multimodel_commits"]
fn crash_writer_child() {
    let path = PathBuf::from(std::env::var_os(CHILD_PATH).expect("child database path"));
    let phase = std::env::var(CHILD_PHASE).unwrap();
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    // Keep the initial snapshot pinned so maintenance cannot copy later commits
    // into the main file before the process is killed.
    let snapshot = Connection::open(&path).unwrap();
    snapshot
        .execute_batch("BEGIN; SELECT * FROM _lilia_kv;")
        .unwrap();
    for id in 0..8 {
        database.batch(&mutations(&id.to_string(), id)).unwrap();
    }
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    let committed_wal_bytes = std::fs::metadata(&wal_path).unwrap().len();
    let mut writer = database.lock_writer().unwrap();
    let transaction = writer.transaction().unwrap();
    if phase == "uncommitted" {
        for operation in mutations("0", 99)
            .iter()
            .chain(mutations("partial", 99).iter())
        {
            apply(&transaction, operation).unwrap();
        }
        // Force dirty transaction pages into WAL without committing them.
        transaction.cache_flush().unwrap();
        assert!(std::fs::metadata(&wal_path).unwrap().len() > committed_wal_bytes);
    }
    std::fs::write(path.with_extension("ready"), b"8 confirmed commits").unwrap();
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
    }
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_writer_recovers_confirmed_multimodel_commits() {
    for phase in ["committed", "uncommitted"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db.lilia");
        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "engine::recovery_tests::crash_writer_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD_PATH, &path)
                .env(CHILD_PHASE, phase)
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        while !path.with_extension("ready").exists() {
            assert!(child.0.try_wait().unwrap().is_none(), "child exited early");
            assert!(Instant::now() < deadline, "child handshake timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        let wal = directory.path().join("db.lilia-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 32);
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        assert!(wal.exists(), "crash must leave WAL for recovery");
        let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
        for id in 0..8u8 {
            let key = id.to_string();
            let kv = database
                .kv_get("recovery", key.as_bytes())
                .unwrap()
                .unwrap();
            assert_eq!(kv.value, vec![id; 4096]);
            assert_eq!(kv.version, 1);
            let json = database.json_get("recovery", &key).unwrap().unwrap();
            assert_eq!(json.value, serde_json::json!({"value": id}));
            assert_eq!(json.version, 1);
        }
        assert!(database.kv_get("recovery", b"partial").unwrap().is_none());
        assert!(database.json_get("recovery", "partial").unwrap().is_none());
        assert!(database.integrity_check().unwrap());
        database.batch(&mutations("after-recovery", 8)).unwrap();
        database.close(Duration::from_secs(5)).unwrap();
        directory.close().unwrap();
    }
}

#[test]
fn page_quota_failure_rolls_back_batch_and_releases_writer() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    database.batch(&mutations("confirmed", 1)).unwrap();
    {
        let writer = database.lock_writer().unwrap();
        let pages: u32 = writer
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        writer
            .pragma_update(None, "max_page_count", pages + 1)
            .unwrap();
    }
    let mut operations = mutations("confirmed", 2).to_vec();
    operations.push(BatchOperation::KvSet {
        namespace: "recovery".into(),
        key: b"oversized".to_vec(),
        value: vec![3; 1024 * 1024],
        if_version: None,
        expires_at_ms: None,
    });
    let error = database.batch(&operations).unwrap_err();
    assert_eq!(error.code, ErrorCode::DiskFull);
    assert!(!error.retryable);
    assert!(!error.request_id.is_nil());
    assert_eq!(
        database
            .kv_get("recovery", b"confirmed")
            .unwrap()
            .unwrap()
            .value,
        vec![1; 4096]
    );
    assert_eq!(
        database
            .json_get("recovery", "confirmed")
            .unwrap()
            .unwrap()
            .value,
        serde_json::json!({"value": 1})
    );
    assert!(database.kv_get("recovery", b"oversized").unwrap().is_none());
    database
        .lock_writer()
        .unwrap()
        .pragma_update(None, "max_page_count", 1_000_000)
        .unwrap();
    database.batch(&mutations("after-full", 4)).unwrap();
    database.close(Duration::from_secs(5)).unwrap();
    let reopened = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert!(reopened.integrity_check().unwrap());
    assert!(reopened
        .json_get("recovery", "after-full")
        .unwrap()
        .is_some());
}

#[test]
fn external_writer_lock_is_retryable_and_preserves_atomicity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let mut options = DatabaseOptions::durable(&path);
    options.busy_timeout_ms = 10;
    let database = Database::open(options).unwrap();
    let external = Connection::open(&path).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = database.batch(&mutations("locked", 1)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Busy);
    assert!(error.retryable);
    assert!(!error.request_id.is_nil());
    assert!(database.kv_get("recovery", b"locked").unwrap().is_none());
    assert!(database.json_get("recovery", "locked").unwrap().is_none());
    external.execute_batch("ROLLBACK").unwrap();
    database.batch(&mutations("locked", 1)).unwrap();
    assert!(database.integrity_check().unwrap());
}

#[test]
fn damaged_database_is_reported_as_corrupt_without_reinitialization() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    database.batch(&mutations("confirmed", 1)).unwrap();
    database.close(Duration::from_secs(5)).unwrap();
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.rewind().unwrap();
    file.write_all(b"invalid database").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let damaged = std::fs::read(&path).unwrap();
    let error = Database::open(DatabaseOptions::durable(&path)).unwrap_err();
    assert_eq!(error.code, ErrorCode::Corrupt);
    assert!(!error.retryable);
    assert!(!error.request_id.is_nil());
    assert_eq!(std::fs::read(&path).unwrap(), damaged);
}
