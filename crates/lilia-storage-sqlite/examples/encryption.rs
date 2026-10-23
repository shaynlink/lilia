use lilia_core::{BatchOperation, Result as LiliaResult, ValueCodec};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;
use std::sync::Arc;

/// Simple sample XOR cipher demonstrating the custom `ValueCodec` pluggable interface.
/// In production, use the `lilia-plugin-codec` AES-256-GCM AEAD native plugin.
#[derive(Debug)]
struct SampleXorCodec {
    key: u8,
}

impl ValueCodec for SampleXorCodec {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "sample-xor"
    }

    fn encode(&self, plaintext: &[u8]) -> LiliaResult<Vec<u8>> {
        Ok(plaintext.iter().map(|b| b ^ self.key).collect())
    }

    fn decode(&self, ciphertext: &[u8]) -> LiliaResult<Vec<u8>> {
        Ok(ciphertext.iter().map(|b| b ^ self.key).collect())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("encrypted.lilia");

    println!("[1] Opening LiliaDB with custom at-rest encryption codec...");
    let codec = Arc::new(SampleXorCodec { key: 0x5A });

    let options = DatabaseOptions::durable(&db_path).with_codec(codec);
    let db = Database::open(options)?;

    // 1. Write confidential KV secret and JSON credential
    println!("\n[2] Writing secret credentials through LiliaDB engine:");
    db.batch(&[
        BatchOperation::KvSet {
            namespace: "secrets".into(),
            key: b"aws:access_key".to_vec(),
            value: b"AKIAIOSFODNN7EXAMPLE".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "vault".into(),
            id: "customer_42".into(),
            value: json!({
                "account_number": "1234-5678-9012-3456",
                "pin": "9876",
                "balance": 1_500_000
            }),
            if_version: None,
        },
    ])?;
    println!("  Inserted secret KV and JSON document.");

    // 2. Read back through Database API -> transparently decrypted
    println!("\n[3] Reading through Database API (transparent decryption):");
    let kv_val = db.kv_get("secrets", b"aws:access_key")?.unwrap();
    println!("  KV secret: {}", String::from_utf8_lossy(&kv_val.value));

    let json_val = db.json_get("vault", "customer_42")?.unwrap();
    println!("  JSON doc:  {}", json_val.value);

    // Drop database to flush SQLite pages and release locks
    drop(db);

    // 3. Inspect raw SQLite file directly on disk
    println!("\n[4] Inspecting RAW SQLite file on disk directly (without codec):");
    let raw_conn = rusqlite::Connection::open(&db_path)?;

    let raw_kv: Vec<u8> = raw_conn.query_row(
        "SELECT value FROM _lilia_kv WHERE namespace = 'secrets' AND key = ?1",
        [b"aws:access_key"],
        |row| row.get(0),
    )?;
    println!("  Raw KV bytes on disk: {:02x?}", &raw_kv[..8]);
    assert_ne!(raw_kv, b"AKIAIOSFODNN7EXAMPLE");

    let raw_json: String = raw_conn.query_row(
        "SELECT value FROM _lilia_json WHERE space = 'vault' AND id = 'customer_42'",
        [],
        |row| row.get(0),
    )?;
    println!("  Raw JSON envelope on disk:\n    {raw_json}");
    assert!(!raw_json.contains("1234-5678-9012-3456"));
    assert!(raw_json.contains("$codec"));
    assert!(raw_json.contains("$data"));

    println!("\n[5] Encryption at rest confirmed! Cleartext never touched disk.");
    Ok(())
}
