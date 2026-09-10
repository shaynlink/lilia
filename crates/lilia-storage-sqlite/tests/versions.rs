use lilia_core::{BatchOperation, ErrorCode};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use rusqlite::{params, Connection};
use std::path::Path;
use std::time::Duration;

const HIGH: u64 = (1_u64 << 53) + 1;

fn put(id: &str, version: Option<u64>) -> [BatchOperation; 2] {
    [
        BatchOperation::KvSet {
            namespace: "versions".into(),
            key: id.as_bytes().to_vec(),
            value: vec![42],
            if_version: version,
            expires_at_ms: Some(4_102_444_800_000),
        },
        BatchOperation::JsonPut {
            space: "versions".into(),
            id: id.into(),
            value: serde_json::json!({"number": 4_294_967_297_u64}),
            if_version: version,
        },
    ]
}

fn seed(path: &Path) {
    assert!(
        !path.exists(),
        "fixture must never overwrite an existing database"
    );
    let database = Database::open(DatabaseOptions::durable(path)).unwrap();
    for id in ["high", "edge"] {
        database.batch(&put(id, None)).unwrap();
    }
    database.close(Duration::from_secs(5)).unwrap();
    let connection = Connection::open(path).unwrap();
    for (id, version) in [
        ("high", i64::try_from(HIGH).unwrap()),
        ("edge", i64::MAX - 1),
    ] {
        connection
            .execute(
                "UPDATE _lilia_kv SET version=?1 WHERE namespace='versions' AND key=?2",
                params![version, id.as_bytes()],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE _lilia_json SET version=?1 WHERE space='versions' AND id=?2",
                params![version, id],
            )
            .unwrap();
    }
    connection.close().unwrap();
}

#[test]
#[ignore = "private fixture producer invoked by the five-surface conformance runner"]
fn seed_precision_fixture() {
    let directory = std::env::var_os("LILIA_VERSION_FIXTURE_DIR").expect("fixture directory");
    for name in ["embedded", "daemon", "cli", "mcp"] {
        seed(&Path::new(&directory).join(format!("{name}.lilia")));
    }
}

#[test]
fn high_versions_conflicts_and_exhaustion_are_exact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db.lilia");
    seed(&path);
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert_eq!(
        database
            .kv_get("versions", b"high")
            .unwrap()
            .unwrap()
            .version,
        HIGH
    );
    assert_eq!(
        database
            .json_get("versions", "high")
            .unwrap()
            .unwrap()
            .version,
        HIGH
    );
    assert_eq!(
        database.batch(&put("high", Some(HIGH))).unwrap()[0].version,
        Some(HIGH + 1)
    );
    let error = database.batch(&put("high", Some(u64::MAX))).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        error.details.unwrap(),
        serde_json::json!({"expected": u64::MAX.to_string(), "actual": (HIGH + 1).to_string()})
    );
    let maximum = u64::try_from(i64::MAX).unwrap();
    database.batch(&put("edge", Some(maximum - 1))).unwrap();
    let mut operations = put("rolled-back", None).to_vec();
    operations.extend(put("edge", Some(maximum)));
    let error = database.batch(&operations).unwrap_err();
    assert_eq!(error.code, ErrorCode::Storage);
    assert!(!error.retryable);
    assert!(database
        .json_get("versions", "rolled-back")
        .unwrap()
        .is_none());
    assert!(database
        .kv_get("versions", b"rolled-back")
        .unwrap()
        .is_none());
    database.close(Duration::from_secs(5)).unwrap();
    let database = Database::open(DatabaseOptions::durable(&path)).unwrap();
    assert_eq!(
        database.kv_scan("versions", Some(b"edge"), 1).unwrap()[0].version,
        HIGH + 1
    );
    assert_eq!(
        database.json_scan("versions", Some("edge"), 1).unwrap()[0].version,
        HIGH + 1
    );
    assert!(database.integrity_check().unwrap());
}
