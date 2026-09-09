use lilia_core::{BatchOperation, ErrorCode};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;

fn database() -> (tempfile::TempDir, Database) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = Database::open(DatabaseOptions::durable(
        directory.path().join("test.lilia"),
    ))
    .expect("open database");
    (directory, database)
}

#[test]
fn kv_round_trip_and_expiry() {
    let (_directory, database) = database();
    database
        .batch(&[BatchOperation::KvSet {
            namespace: "cache".into(),
            key: b"answer".to_vec(),
            value: b"42".to_vec(),
            if_version: Some(0),
            expires_at_ms: None,
        }])
        .expect("set value");
    let entry = database
        .kv_get("cache", b"answer")
        .expect("get value")
        .expect("entry");
    assert_eq!(entry.value, b"42");
    assert_eq!(entry.version, 1);

    database
        .batch(&[BatchOperation::KvSet {
            namespace: "cache".into(),
            key: b"expired".to_vec(),
            value: vec![1],
            if_version: None,
            expires_at_ms: Some(0),
        }])
        .expect("set expired value");
    assert!(database
        .kv_get("cache", b"expired")
        .expect("get expired")
        .is_none());
}

#[test]
fn batch_rolls_back_on_conflict() {
    let (_directory, database) = database();
    let error = database
        .batch(&[
            BatchOperation::JsonPut {
                space: "people".into(),
                id: "ada".into(),
                value: json!({"name":"Ada"}),
                if_version: Some(0),
            },
            BatchOperation::KvDelete {
                namespace: "missing".into(),
                key: b"key".to_vec(),
                if_version: Some(1),
            },
        ])
        .expect_err("version conflict");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(database
        .json_get("people", "ada")
        .expect("get rolled back value")
        .is_none());
}

#[test]
fn scans_are_stable_and_bounded() {
    let (_directory, database) = database();
    let operations = ["a", "b", "c"].map(|id| BatchOperation::JsonPut {
        space: "ordered".into(),
        id: id.into(),
        value: json!({"id": id}),
        if_version: None,
    });
    database.batch(&operations).expect("seed values");
    let first = database.json_scan("ordered", None, 2).expect("first page");
    assert_eq!(
        first
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let second = database
        .json_scan("ordered", Some("b"), 2)
        .expect("second page");
    assert_eq!(second[0].id, "c");
    assert!(database.integrity_check().expect("integrity check"));
}

#[test]
fn scans_are_stable_and_integrity_is_valid() {
    let (_directory, database) = database();
    for id in ["a", "b", "c"] {
        database
            .batch(&[BatchOperation::JsonPut {
                space: "items".into(),
                id: id.into(),
                value: json!({"id": id}),
                if_version: None,
            }])
            .expect("put json");
    }
    let page = database
        .json_scan("items", Some("a"), 2)
        .expect("scan json");
    assert_eq!(
        page.iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["b", "c"]
    );
    assert!(database.integrity_check().expect("integrity check"));
}

#[test]
fn online_backup_contains_confirmed_commits() {
    let (directory, database) = database();
    database
        .batch(&[BatchOperation::JsonPut {
            space: "people".into(),
            id: "ada".into(),
            value: json!({"name":"Ada"}),
            if_version: Some(0),
        }])
        .expect("put json");
    let backup_path = directory.path().join("backup.lilia");
    database.backup(&backup_path).expect("online backup");

    let backup = Database::open(DatabaseOptions::durable(backup_path)).expect("open backup");
    assert_eq!(
        backup
            .json_get("people", "ada")
            .expect("read backup")
            .expect("backed up record")
            .value,
        json!({"name":"Ada"})
    );
    assert!(backup.integrity_check().expect("backup integrity"));
}
