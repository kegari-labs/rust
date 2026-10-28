use kegari_types::error::{Error, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn bytes_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Canonical JSON: sorted UTF-8 object keys, meaningful array order, no whitespace.
/// Floating point identities are prohibited: evidence uses integers and strings.
pub fn canonical_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    fn ordered(value: Value) -> Result<Value> {
        Ok(match value {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<_, _> = map.into_iter().collect();
                let mut result = serde_json::Map::new();
                for (k, v) in sorted {
                    result.insert(k, ordered(v)?);
                }
                Value::Object(result)
            }
            Value::Array(values) => {
                Value::Array(values.into_iter().map(ordered).collect::<Result<_>>()?)
            }
            Value::Number(ref n) if n.is_f64() => {
                return Err(Error::invalid(
                    "floating point evidence identities are not supported",
                ));
            }
            other => other,
        })
    }
    let value = serde_json::to_value(value)
        .map_err(|_| Error::invalid("identity is not JSON serializable"))?;
    serde_json::to_vec(&ordered(value)?).map_err(|_| Error::invalid("identity cannot be encoded"))
}

pub fn canonical_digest<T: Serialize + ?Sized>(schema: u32, value: &T) -> Result<String> {
    if schema == 0 {
        return Err(Error::invalid("digest schema must be positive"));
    }
    #[derive(Serialize)]
    struct Identity<'a, T: ?Sized> {
        schema_version: u32,
        value: &'a T,
    }
    let bytes = canonical_bytes(&Identity {
        schema_version: schema,
        value,
    })?;
    Ok(format!(
        "sha256:v{schema}:{}",
        hex::encode(Sha256::digest(bytes))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_byte_and_canonical_vectors_preserve_hash_identity() {
        assert_eq!(
            bytes_digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            bytes_digest(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let value = serde_json::json!({"b":[1,"two"],"a":true});
        assert_eq!(
            canonical_bytes(&value).unwrap(),
            br#"{"a":true,"b":[1,"two"]}"#
        );
        assert_eq!(
            canonical_digest(1, &value).unwrap(),
            "sha256:v1:e75f1c66fb37047ed8c904526793282ef3a3b95b8735159e40d2d9d9fda945ee"
        );
        assert_ne!(
            canonical_digest(1, &value).unwrap(),
            canonical_digest(2, &value).unwrap()
        );
        assert_ne!(
            canonical_digest(1, &[1, 2]).unwrap(),
            canonical_digest(1, &[2, 1]).unwrap()
        );
    }
    #[test]
    fn zero_schema_and_nested_floating_point_identities_are_rejected() {
        assert_eq!(
            canonical_digest(0, &true).unwrap_err(),
            Error::invalid("digest schema must be positive")
        );
        for value in [serde_json::json!(1.0), serde_json::json!({"nested":[1.5]})] {
            assert_eq!(
                canonical_bytes(&value).unwrap_err(),
                Error::invalid("floating point evidence identities are not supported")
            );
            assert!(canonical_digest(1, &value).is_err());
        }
    }
    #[test]
    fn invalid_serialization_returns_error_and_valid_digest_bytes_stay_unchanged() {
        let invalid = std::collections::BTreeMap::from([(vec![1u8, 2], 3)]);
        assert_eq!(
            canonical_digest(1, &invalid).unwrap_err().code,
            "invalid_config"
        );
        let value = serde_json::json!({"b":[1,"two"],"a":true});
        let old = canonical_bytes(&serde_json::json!({"schema_version":1,"value":value})).unwrap();
        assert_eq!(
            canonical_digest(1, &value).unwrap(),
            format!("sha256:v1:{}", hex::encode(Sha256::digest(old)))
        );
    }
}
