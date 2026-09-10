use lilia_core::{BatchOperation, MutationResult};
use serde_json::json;

#[test]
fn binary_versions_remain_native_u64_and_json_versions_are_decimal() {
    for value in [0, 1, (1_u64 << 53) + 1, u64::MAX] {
        let operation = BatchOperation::JsonDelete {
            space: "v".into(),
            id: "a".into(),
            if_version: Some(value),
        };
        let bytes = rmp_serde::to_vec_named(&operation).unwrap();
        let wire: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(wire["if_version"].as_u64(), Some(value));
        let decoded: BatchOperation = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json::to_value(decoded).unwrap()["if_version"],
            value.to_string()
        );
        let request = lilia_protocol::Request {
            id: "test".into(),
            token: "test".into(),
            deadline_ms: Some(5000),
            operation: lilia_protocol::Operation::Batch {
                operations: vec![operation],
            },
        };
        let bytes = rmp_serde::to_vec_named(&request).unwrap();
        let _: lilia_protocol::Request = rmp_serde::from_slice(&bytes).unwrap();
        let result = MutationResult {
            version: Some(value),
            deleted: false,
        };
        let bytes = rmp_serde::to_vec_named(&result).unwrap();
        let wire: serde_json::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(wire["version"].as_u64(), Some(value));
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({"version": value.to_string(), "deleted": false})
        );
    }
}
