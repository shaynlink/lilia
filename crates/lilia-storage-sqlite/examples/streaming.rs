use lilia_core::BatchOperation;
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;

const TOTAL_ITEMS: usize = 300;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("streaming.lilia");

    println!("[1] Opening embedded LiliaDB at {}", db_path.display());
    let db = Database::open(DatabaseOptions::durable(&db_path))?;

    // 1. Seed 300 telemetry events and index keys
    println!("\n[2] Seeding {TOTAL_ITEMS} records in batches of 100...");

    let mut batch = Vec::with_capacity(100);
    for i in 0..TOTAL_ITEMS {
        let doc_id = format!("event_{i:04}");
        #[allow(clippy::cast_precision_loss)]
        let reading = 20.0 + (i as f64 * 0.1);
        batch.push(BatchOperation::JsonPut {
            space: "telemetry".into(),
            id: doc_id.clone(),
            value: json!({
                "sensor_id": format!("sensor_{}", i % 10),
                "reading": reading,
                "seq": i
            }),
            if_version: None,
        });

        if batch.len() == 100 || i == TOTAL_ITEMS - 1 {
            db.batch(&batch)?;
            batch.clear();
        }
    }
    println!("  Seeded {TOTAL_ITEMS} documents successfully.");

    // 2. Keyset pagination streaming in Rust:
    //    Fetch pages of 50 items using `after` cursor until finished.
    println!("\n[3] Streaming JSON documents using keyset pagination (chunk size: 50):");
    let mut cursor: Option<String> = None;
    let mut total_streamed = 0;
    let mut page_number = 1;

    loop {
        let entries = db.json_scan("telemetry", cursor.as_deref(), 50)?;
        if entries.is_empty() {
            break;
        }

        println!(
            "  Page {}: fetched {} documents (first: {}, last: {})",
            page_number,
            entries.len(),
            entries.first().unwrap().id,
            entries.last().unwrap().id
        );

        total_streamed += entries.len();
        cursor = Some(entries.last().unwrap().id.clone());
        page_number += 1;
    }
    println!("  Total documents streamed: {total_streamed}");

    // 3. Streaming with a global limit and resume cursor
    println!("\n[4] Streaming with resume cursor 'event_0100' and global limit 10:");
    let resume_after = Some("event_0100");
    let partial_entries = db.json_scan("telemetry", resume_after, 10)?;
    for entry in &partial_entries {
        println!(
            "    - ID: {}, reading: {}",
            entry.id, entry.value["reading"]
        );
    }
    println!("  Fetched {} items.", partial_entries.len());

    println!("\n[5] Streaming completed cleanly!");
    Ok(())
}
