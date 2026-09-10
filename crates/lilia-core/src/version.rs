//! Decimal versions in JSON; native u64 integers in binary protocols.
use serde::{Deserialize, Deserializer, Serializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    Integer(u64),
    Decimal(String),
}

fn parse<E: serde::de::Error>(input: Input) -> Result<u64, E> {
    match input {
        Input::Integer(value) => Ok(value),
        Input::Decimal(value) => {
            if value.is_empty()
                || !value.bytes().all(|byte| byte.is_ascii_digit())
                || (value.len() > 1 && value.starts_with('0'))
            {
                return Err(E::custom("version must be a canonical unsigned decimal"));
            }
            value.parse().map_err(|_| E::custom("version exceeds u64"))
        }
    }
}

// Serde's with-module callback requires a reference to the field type.
#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
        serializer.serialize_str(&value.to_string())
    } else {
        serializer.serialize_u64(*value)
    }
}

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    parse(Input::deserialize(deserializer)?)
}

pub(crate) mod optional {
    use super::{parse, Input};
    use serde::{Deserialize, Deserializer, Serializer};

    // Serde's with-module callback requires a reference to the complete field.
    #[allow(clippy::ref_option)]
    pub(crate) fn serialize<S: Serializer>(
        value: &Option<u64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            None => serializer.serialize_none(),
            Some(value) if serializer.is_human_readable() => {
                serializer.serialize_some(&value.to_string())
            }
            Some(value) => serializer.serialize_some(value),
        }
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        Option::<Input>::deserialize(deserializer)?
            .map(parse)
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use crate::BatchOperation;
    use serde_json::json;

    #[test]
    fn decimal_versions_are_exact_and_legacy_integers_remain_readable() {
        for value in [0, 1, (1_u64 << 53) + 1, u64::MAX] {
            for input in [json!(value), json!(value.to_string())] {
                let operation: BatchOperation = serde_json::from_value(json!({
                    "model":"json_delete", "space":"docs", "id":"a", "if_version":input
                }))
                .unwrap();
                let BatchOperation::JsonDelete { if_version, .. } = operation else {
                    panic!()
                };
                assert_eq!(if_version, Some(value));
                assert_eq!(
                    serde_json::to_value(operation).unwrap()["if_version"],
                    value.to_string()
                );
            }
        }
        for input in [
            json!(-1),
            json!(1.5),
            json!("+1"),
            json!("01"),
            json!(""),
            json!(" 1"),
            json!("18446744073709551616"),
        ] {
            assert!(serde_json::from_value::<BatchOperation>(json!({
                "model":"json_delete", "space":"docs", "id":"a", "if_version":input
            }))
            .is_err());
        }
    }
}
