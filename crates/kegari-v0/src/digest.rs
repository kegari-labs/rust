use crate::error::{Error, Result};
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
    let bytes = canonical_bytes(&serde_json::json!({"schema_version": schema, "value": value}))?;
    Ok(format!(
        "sha256:v{schema}:{}",
        hex::encode(Sha256::digest(bytes))
    ))
}
