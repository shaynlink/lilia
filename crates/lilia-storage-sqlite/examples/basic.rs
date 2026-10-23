use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

#[allow(clippy::cast_possible_truncation)]
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("demo.lilia");

    println!("[1] Opening embedded LiliaDB at {}", db_path.display());
    let db = Database::open(DatabaseOptions::durable(&db_path))?;

    // 1. Key/Value operations
    println!("\n[2] Key/Value Operations:");
    db.batch(&[BatchOperation::KvSet {
        namespace: "config".into(),
        key: b"app.theme".to_vec(),
        value: b"dark".to_vec(),
        if_version: Some(0), // Only insert if key does not exist
        expires_at_ms: Some(now_ms() + 3_600_000), // 1 hour TTL
    }])?;
    println!("  Set 'config:app.theme' -> 'dark'");

    let entry = db
        .kv_get("config", b"app.theme")?
        .expect("entry should exist");
    println!(
        "  Get 'config:app.theme': value='{}', version={}, expires_at_ms={:?}",
        String::from_utf8_lossy(&entry.value),
        entry.version,
        entry.expires_at_ms
    );

    // 2. JSON Document operations
    println!("\n[3] JSON Document Operations:");
    db.batch(&[BatchOperation::JsonPut {
        space: "users".into(),
        id: "ada".into(),
        value: json!({
            "name": "Ada Lovelace",
            "role": "pioneer",
            "skills": ["math", "algorithms"]
        }),
        if_version: Some(0),
    }])?;
    println!("  Put document users/ada");

    let doc = db.json_get("users", "ada")?.expect("document should exist");
    println!("  Get document users/ada (v{}): {}", doc.version, doc.value);

    // 3. Multi-Model Atomic Transaction (KV + JSON in 1 commit)
    println!("\n[4] Multi-Model Atomic Transaction:");
    let results = db.batch(&[
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "ada".into(),
            value: json!({
                "name": "Ada Lovelace",
                "role": "pioneer",
                "skills": ["math", "algorithms"],
                "last_active": now_ms()
            }),
            if_version: Some(doc.version), // Optimistic concurrency check
        },
        BatchOperation::KvSet {
            namespace: "audit".into(),
            key: b"login:ada".to_vec(),
            value: b"success".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
    ])?;
    println!("  Committed {} operations atomically:", results.len());
    for (i, res) in results.iter().enumerate() {
        println!(
            "    Op {}: version={:?}, deleted={}",
            i + 1,
            res.version,
            res.deleted
        );
    }

    // 4. Integrity check & backup
    println!("\n[5] Integrity Check & Online Backup:");
    let is_ok = db.integrity_check()?;
    println!("  Integrity check: {}", if is_ok { "OK" } else { "FAILED" });

    let backup_path = temp_dir.path().join("demo_backup.lilia");
    db.backup(&backup_path)?;
    println!(
        "  Online backup successfully created at {}",
        backup_path.display()
    );

    println!("\n[6] Done! Database closed cleanly on drop.");
    Ok(())
}
