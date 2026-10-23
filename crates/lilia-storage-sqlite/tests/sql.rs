use lilia_core::{BatchOperation, Result as LiliaResult, ValueCodec};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
struct TestXorCodec {
    key: u8,
}

impl ValueCodec for TestXorCodec {
    fn name(&self) -> &'static str {
        "test-xor"
    }

    fn encode(&self, plaintext: &[u8]) -> LiliaResult<Vec<u8>> {
        Ok(plaintext.iter().map(|b| b ^ self.key).collect())
    }

    fn decode(&self, ciphertext: &[u8]) -> LiliaResult<Vec<u8>> {
        Ok(ciphertext.iter().map(|b| b ^ self.key).collect())
    }
}

#[test]
fn test_sql_basic_queries_and_json_extract() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(
        temp_dir.path().join("sql_test.lilia"),
    ))
    .unwrap();

    // Seed KV and JSON data
    db.batch(&[
        BatchOperation::KvSet {
            namespace: "settings".into(),
            key: b"theme".to_vec(),
            value: b"dark".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::KvSet {
            namespace: "settings".into(),
            key: b"lang".to_vec(),
            value: b"fr".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "user_1".into(),
            value: json!({ "name": "Ada", "role": "admin", "age": 36 }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "user_2".into(),
            value: json!({ "name": "Alan", "role": "dev", "age": 41 }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "user_3".into(),
            value: json!({ "name": "Grace", "role": "admin", "age": 50 }),
            if_version: None,
        },
    ])
    .unwrap();

    // 1. SELECT with json_extract and WHERE
    let res = db
        .sql_query(
            "SELECT id, json_extract(value, '$.name') AS name, json_extract(value, '$.age') AS age
             FROM lilia_json
             WHERE space = 'users' AND json_extract(value, '$.role') = ?1
             ORDER BY age ASC",
            &[json!("admin")],
        )
        .unwrap();

    assert_eq!(res.columns, vec!["id", "name", "age"]);
    assert_eq!(res.rows.len(), 2);
    assert_eq!(res.rows[0], vec![json!("user_1"), json!("Ada"), json!(36)]);
    assert_eq!(
        res.rows[1],
        vec![json!("user_3"), json!("Grace"), json!(50)]
    );
    assert_eq!(res.rows_affected, 0);

    // 2. Aggregate query COUNT(*), AVG(age)
    let agg = db
        .sql_query(
            "SELECT count(*) as total, avg(json_extract(value, '$.age')) as avg_age
             FROM lilia_json
             WHERE space = 'users'",
            &[],
        )
        .unwrap();
    assert_eq!(agg.columns, vec!["total", "avg_age"]);
    assert_eq!(agg.rows.len(), 1);
    assert_eq!(agg.rows[0][0], json!(3));

    // 3. Query KV view
    let kv_res = db
        .sql_query(
            "SELECT namespace, hex(key) AS hex_key, hex(value) AS hex_val
             FROM lilia_kv
             WHERE namespace = 'settings'
             ORDER BY key ASC",
            &[],
        )
        .unwrap();
    assert_eq!(kv_res.rows.len(), 2);
    assert_eq!(kv_res.rows[0][0], json!("settings"));
}

#[test]
fn test_sql_with_codec_transparent_decryption() {
    let temp_dir = tempfile::tempdir().unwrap();
    let codec = Arc::new(TestXorCodec { key: 0x42 });
    let options =
        DatabaseOptions::durable(temp_dir.path().join("encrypted_sql.lilia")).with_codec(codec);
    let db = Database::open(options).unwrap();

    // Insert sensitive documents
    db.batch(&[
        BatchOperation::KvSet {
            namespace: "secrets".into(),
            key: b"api_token".to_vec(),
            value: b"super-secret-token".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "vault".into(),
            id: "card_1".into(),
            value: json!({ "pan": "4111-2222-3333-4444", "cvv": "123", "owner": "Satoshi" }),
            if_version: None,
        },
    ])
    .unwrap();

    // Query lilia_json view -> automatically transparently decrypted!
    let res = db
        .sql_query(
            "SELECT id, json_extract(value, '$.pan') as pan, json_extract(value, '$.owner') as owner
             FROM lilia_json
             WHERE space = 'vault'",
            &[],
        )
        .unwrap();

    assert_eq!(res.rows.len(), 1);
    assert_eq!(
        res.rows[0],
        vec![
            json!("card_1"),
            json!("4111-2222-3333-4444"),
            json!("Satoshi")
        ]
    );

    // Query raw _lilia_json table -> contains the ciphertext envelope
    let raw = db
        .sql_query("SELECT value FROM _lilia_json WHERE space = 'vault'", &[])
        .unwrap();
    let raw_str = raw.rows[0][0].as_str().unwrap();
    assert!(raw_str.contains("$codec"));
    assert!(raw_str.contains("$data"));
    assert!(!raw_str.contains("4111-2222-3333-4444"));

    // Query lilia_kv view -> automatically decrypted bytes
    let kv_res = db
        .sql_query(
            "SELECT namespace, cast(value as text) as plain_val FROM lilia_kv WHERE namespace = 'secrets'",
            &[],
        )
        .unwrap();
    assert_eq!(kv_res.rows.len(), 1);
    assert_eq!(kv_res.rows[0][1], json!("super-secret-token"));
}

#[test]
fn test_sql_mutations_and_rows_affected() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(
        temp_dir.path().join("mutation_sql.lilia"),
    ))
    .unwrap();

    // Mutating query via SQL
    let insert_res = db
        .sql_query(
            "INSERT INTO _lilia_json(space, id, value, version) VALUES('tasks', 'task_1', '{\"title\":\"Buy milk\"}', 1)",
            &[],
        )
        .unwrap();
    assert_eq!(insert_res.rows_affected, 1);

    // Read back via json_get
    let doc = db.json_get("tasks", "task_1").unwrap().expect("must exist");
    assert_eq!(doc.value["title"], "Buy milk");

    // Update via SQL
    let update_res = db
        .sql_query(
            "UPDATE _lilia_json SET value = '{\"title\":\"Buy coffee\"}', version = 2 WHERE space = 'tasks' AND id = 'task_1'",
            &[],
        )
        .unwrap();
    assert_eq!(update_res.rows_affected, 1);

    let doc_updated = db.json_get("tasks", "task_1").unwrap().unwrap();
    assert_eq!(doc_updated.value["title"], "Buy coffee");
    assert_eq!(doc_updated.version, 2);

    // Delete via SQL
    let delete_res = db
        .sql_query(
            "DELETE FROM _lilia_json WHERE space = 'tasks' AND id = 'task_1'",
            &[],
        )
        .unwrap();
    assert_eq!(delete_res.rows_affected, 1);
    assert!(db.json_get("tasks", "task_1").unwrap().is_none());
}
