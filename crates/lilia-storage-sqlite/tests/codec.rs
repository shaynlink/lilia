use lilia_core::{BatchOperation, Result, ValueCodec};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
struct MockRot13Codec;

impl ValueCodec for MockRot13Codec {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "rot13"
    }

    fn encode(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        Ok(plaintext.iter().map(|b| b.wrapping_add(13)).collect())
    }

    fn decode(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        Ok(ciphertext.iter().map(|b| b.wrapping_sub(13)).collect())
    }
}

#[test]
fn codec_encrypts_kv_and_json_at_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("encrypted.lilia");

    let codec = Arc::new(MockRot13Codec);
    let options = DatabaseOptions::durable(&db_path).with_codec(codec.clone());
    let db = Database::open(options).expect("open database with codec");

    // 1. Write KV and JSON
    db.batch(&[
        BatchOperation::KvSet {
            namespace: "secrets".into(),
            key: b"api_token".to_vec(),
            value: b"sk-super-secret-12345".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "credentials".into(),
            id: "user1".into(),
            value: json!({
                "username": "alice",
                "apikey": "secret-key-999"
            }),
            if_version: None,
        },
    ])
    .expect("batch insert");

    // 2. Read back via Database APIs -> transparently decoded
    let kv_val = db
        .kv_get("secrets", b"api_token")
        .expect("kv_get")
        .expect("exists");
    assert_eq!(kv_val.value, b"sk-super-secret-12345");

    let kv_scan = db.kv_scan("secrets", None, 10).expect("kv_scan");
    assert_eq!(kv_scan.len(), 1);
    assert_eq!(kv_scan[0].value, b"sk-super-secret-12345");

    let json_val = db
        .json_get("credentials", "user1")
        .expect("json_get")
        .expect("exists");
    assert_eq!(json_val.value["username"], "alice");
    assert_eq!(json_val.value["apikey"], "secret-key-999");

    let json_scan = db.json_scan("credentials", None, 10).expect("json_scan");
    assert_eq!(json_scan.len(), 1);
    assert_eq!(json_scan[0].value["username"], "alice");

    // Close and inspect raw SQLite storage
    drop(db);

    let raw_conn = rusqlite::Connection::open(&db_path).expect("open raw sqlite");

    // Verify KV row is rot13 encoded at rest
    let raw_kv: Vec<u8> = raw_conn
        .query_row(
            "SELECT value FROM _lilia_kv WHERE namespace = 'secrets' AND key = ?1",
            [b"api_token"],
            |row| row.get(0),
        )
        .expect("query raw kv");
    assert_ne!(raw_kv, b"sk-super-secret-12345");
    let decoded_kv = codec.decode(&raw_kv).expect("decode raw kv");
    assert_eq!(decoded_kv, b"sk-super-secret-12345");

    // Verify JSON row is stored as an envelope with $codec and $data
    let raw_json_str: String = raw_conn
        .query_row(
            "SELECT value FROM _lilia_json WHERE space = 'credentials' AND id = 'user1'",
            [],
            |row| row.get(0),
        )
        .expect("query raw json");

    let raw_envelope: serde_json::Value =
        serde_json::from_str(&raw_json_str).expect("envelope is valid json");
    assert_eq!(raw_envelope["$codec"], "rot13");
    assert!(raw_envelope["$data"].is_string());
    assert!(!raw_json_str.contains("secret-key-999"));
}
