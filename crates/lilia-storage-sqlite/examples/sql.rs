use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;

#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("sql_demo.lilia");

    println!("[1] Opening embedded LiliaDB at {}", db_path.display());
    let db = Database::open(DatabaseOptions::durable(&db_path))?;

    // 1. Seed JSON Documents and KV entries
    println!("\n[2] Seeding documents and KV data...");
    db.batch(&[
        BatchOperation::JsonPut {
            space: "devices".into(),
            id: "sensor-01".into(),
            value: json!({
                "model": "SenseX",
                "temperature": 23.4,
                "battery": 92,
                "status": "online"
            }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "devices".into(),
            id: "sensor-02".into(),
            value: json!({
                "model": "SenseX",
                "temperature": 28.1,
                "battery": 45,
                "status": "online"
            }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "devices".into(),
            id: "sensor-03".into(),
            value: json!({
                "model": "SenseLite",
                "temperature": 21.0,
                "battery": 15,
                "status": "low_battery"
            }),
            if_version: None,
        },
        BatchOperation::KvSet {
            namespace: "cluster".into(),
            key: b"region".to_vec(),
            value: b"eu-central".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
    ])?;
    println!("  Seeded 3 IoT device documents and 1 cluster KV entry.");

    // 2. Querying documents using SQLite json_extract()
    println!("\n[3] Querying lilia_json view with SQL filtering and ordering:");
    let result = db.sql_query(
        "SELECT id, json_extract(value, '$.model') AS model, json_extract(value, '$.temperature') AS temp, json_extract(value, '$.battery') AS battery
         FROM lilia_json
         WHERE space = ?1 AND json_extract(value, '$.battery') > ?2
         ORDER BY temp DESC",
        &[json!("devices"), json!(40)],
    )?;

    println!("  Columns: {:?}", result.columns);
    println!("  Devices with battery > 40%:");
    for row in &result.rows {
        println!("    Row: {row:?}");
    }

    // 3. Running SQL aggregations across documents
    println!("\n[4] Running SQL aggregations across documents:");
    let stats = db.sql_query(
        "SELECT
           json_extract(value, '$.model') AS model,
           COUNT(*) AS count,
           ROUND(AVG(json_extract(value, '$.temperature')), 2) AS avg_temp,
           MIN(json_extract(value, '$.battery')) AS min_battery
         FROM lilia_json
         WHERE space = ?1
         GROUP BY model",
        &[json!("devices")],
    )?;

    println!("  Model Summary (columns: {:?}):", stats.columns);
    for row in &stats.rows {
        println!("    Model: {}", row[0]);
        println!("      Count: {}", row[1]);
        println!("      Avg Temp: {}°C", row[2]);
        println!("      Min Battery: {}%", row[3]);
    }

    // 4. Querying Key/Value pairs via lilia_kv view
    println!("\n[5] Querying lilia_kv view:");
    let kv_result = db.sql_query(
        "SELECT namespace, CAST(key AS TEXT) AS key_str, CAST(value AS TEXT) AS val_str
         FROM lilia_kv
         WHERE namespace = ?1",
        &[json!("cluster")],
    )?;
    println!("  KV rows: {:?}", kv_result.rows);

    // 5. Executing SQL mutations
    println!("\n[6] Executing SQL mutations:");
    let mutation = db.sql_query(
        "UPDATE _lilia_json
         SET value = json_set(value, '$.status', 'alert')
         WHERE space = ?1 AND json_extract(value, '$.temperature') > ?2",
        &[json!("devices"), json!(25.0)],
    )?;
    println!("  Rows affected: {}", mutation.rows_affected);

    // Verify through json_get
    let updated_sensor = db.json_get("devices", "sensor-02")?.expect("exists");
    println!(
        "  Sensor-02 status after SQL update: {}",
        updated_sensor.value["status"]
    );

    println!("\n[✓] Native Rust SQL example completed successfully!");
    Ok(())
}
