//! Fixed private Linear recording URLs and authenticated byte readback.
use super::*;

/// Parsed private Linear storage URL. This is a read target, not host publication
/// authority; only the backend can verify owned receipts and seal an asset.
#[derive(Clone, Debug)]
pub struct LinearAssetUrl(Url);
impl LinearAssetUrl {
    pub fn parse(value: &str) -> Result<Self> {
        let url = Url::parse(value)
            .map_err(|_| Error::new("asset_scope", "Linear asset URL is invalid"))?;
        validate_https(&url)?;
        if url.host_str() != Some("uploads.linear.app")
            || url.port_or_known_default() != Some(443)
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() == "/"
            || value
                .chars()
                .any(|c| c.is_whitespace() || "()<>[]\\".contains(c))
        {
            return Err(Error::new(
                "asset_scope",
                "only canonical private Linear storage URLs are supported",
            ));
        }
        Ok(Self(url))
    }
}

/// Fixed first-line association syntax, followed by a blank line and signed host
/// publication. An incidental URL elsewhere in a comment is not association.
pub fn linear_video_markdown(value: &str) -> Result<String> {
    LinearAssetUrl::parse(value)?;
    Ok(format!("![Kegari validation recording]({value})"))
}

impl LinearClient {
    /// Read exactly the supplied recording bytes from private Linear storage.
    /// This does not verify publication receipts or an issue/comment association.
    pub async fn read_recording(
        &self,
        url: &LinearAssetUrl,
        expected_size: usize,
        expected_digest: &str,
    ) -> Result<Vec<u8>> {
        if self.http.policy.endpoint.as_str() != "https://api.linear.app/graphql" {
            return Err(Error::new(
                "asset_scope",
                "private storage authentication is supported only for the documented native Linear endpoint",
            ));
        }
        if expected_size == 0 || expected_size > MAX_VIDEO_BYTES {
            return Err(Error::new(
                "asset_scope",
                "recording readback requires a positive bounded size",
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert("authorization", self.http.authorization.clone());
        headers.insert("accept", HeaderValue::from_static("video/mp4"));
        let response = tokio::time::timeout(
            self.http.policy.timeout,
            self.asset_transport.send(HttpRequest {
                method: Method::Get,
                url: url.0.clone(),
                headers,
                body: vec![],
            }),
        )
        .await
        .map_err(|_| Error::new("provider_transport", "bounded recording readback timed out"))??;
        *self
            .http
            .rate_limit
            .lock()
            .map_err(|_| Error::new("provider_transport", "rate metadata unavailable"))? =
            RateLimit::from_headers(&response.headers, false);
        if response.status != 200 {
            return Err(status_error(&response, false));
        }
        if response.body.len() != expected_size
            || response.body.len() > MAX_VIDEO_BYTES
            || response
                .headers
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                != Some("video/mp4")
            || response.headers.get("content-length").is_some_and(|v| {
                v.to_str().ok().and_then(|v| v.parse::<usize>().ok()) != Some(expected_size)
            })
            || bytes_digest(&response.body) != expected_digest
        {
            return Err(Error::new(
                "asset_content",
                "downloaded recording differs from exact uploaded bytes/type/size",
            ));
        }
        Ok(response.body)
    }
}
