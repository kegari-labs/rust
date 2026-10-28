//! Pure provider wire validation; no I/O or authority.
use super::validate_https;
use kegari_types::error::{Error, Result};
use serde_json::Value;
use url::Url;

pub fn text_at<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && !s.contains('\0'))
        .ok_or_else(|| {
            Error::new(
                "provider_response",
                format!("required provider field {path} is missing or malformed"),
            )
        })
}

pub fn numeric_id(s: &str) -> Result<u64> {
    s.parse::<u64>()
        .ok()
        .filter(|id| *id > 0 && id.to_string() == s)
        .ok_or_else(|| Error::invalid("canonical positive decimal provider ID required"))
}

pub fn sha(value: &str) -> Result<&str> {
    if value.len() != 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::invalid("exact 40-character Git SHA is required"));
    }
    Ok(value)
}

pub fn bounded_text(s: &str) -> Result<()> {
    if s.trim().is_empty() || s.len() > 1_048_576 || s.contains('\0') {
        return Err(Error::invalid(
            "provider text must be nonempty, bounded and contain no NUL",
        ));
    }
    Ok(())
}

pub fn validated_url(value: &str) -> Result<String> {
    let url = Url::parse(value)
        .map_err(|_| Error::new("provider_response", "provider reference URL is malformed"))?;
    // Native GitHub comment references include an issuecomment fragment. These
    // URLs are references, never authenticated request endpoints.
    let mut endpoint = url.clone();
    endpoint.set_fragment(None);
    validate_https(&endpoint).map_err(|_| {
        Error::new(
            "provider_response",
            "provider reference URL is not safe HTTPS",
        )
    })?;
    Ok(url.to_string())
}

pub fn reference_components(reference: &str) -> Result<Vec<&str>> {
    if !(reference.starts_with("heads/") || reference.starts_with("tags/"))
        || reference.contains(['\0', '\\', '~', '^', ':', '?', '*', '[', ' '])
        || reference.contains("..")
        || reference.contains("@{")
        || reference.ends_with('/')
        || reference.ends_with('.')
        || reference.ends_with(".lock")
        || reference
            .split('/')
            .any(|s| s.is_empty() || s.starts_with('.'))
    {
        return Err(Error::invalid("exact head/tag reference required"));
    }
    Ok(reference.split('/').collect())
}
