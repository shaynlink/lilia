use lilia_core::{BatchOperation, Result};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::{json, Value};

#[test]
fn shared_conformance_fixtures() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../../tools/conformance/fixtures.json")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let database =
        Database::open(DatabaseOptions::durable(directory.path().join("db.lilia"))).unwrap();
    for step in fixtures {
        let name = step["name"].as_str().unwrap();
        match execute(&database, &step) {
            Ok(value) => {
                assert!(step.get("error").is_none(), "{name}: expected error");
                assert_eq!(value, step["expected"], "{name}");
            }
            Err(error) => {
                assert_eq!(
                    json!({"code":error.code,"retryable":error.retryable}),
                    step["error"],
                    "{name}"
                );
                assert!(!error.request_id.is_nil(), "{name}: missing request ID");
            }
        }
    }
    assert!(database.integrity_check().unwrap());
    database.close(std::time::Duration::from_secs(5)).unwrap();
}

fn execute(database: &Database, step: &Value) -> Result<Value> {
    let text = |field: &str| step[field].as_str().unwrap();
    let bytes = |field: &str| serde_json::from_value::<Vec<u8>>(step[field].clone()).unwrap();
    let limit = || u32::try_from(step["limit"].as_u64().unwrap()).unwrap();
    Ok(match text("op") {
        "batch" => {
            let operations: Vec<BatchOperation> =
                serde_json::from_value(step["operations"].clone()).unwrap();
            json!(database.batch(&operations)?)
        }
        "kv_get" => json!(database.kv_get(text("namespace"), &bytes("key"))?),
        "kv_scan" => {
            let after = step.get("after").map(|_| bytes("after"));
            json!(database.kv_scan(text("namespace"), after.as_deref(), limit())?)
        }
        "json_get" => json!(database.json_get(text("space"), text("id"))?),
        "json_scan" => json!(database.json_scan(text("space"), step["after"].as_str(), limit())?),
        other => panic!("unknown fixture operation {other}"),
    })
}
