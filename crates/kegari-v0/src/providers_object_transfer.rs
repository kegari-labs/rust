//! Fixed native GitHub smart-HTTP read transport. Credentials never leave this
//! provider module or reach Git subprocesses, URLs, receipts or diagnostics.
use super::*;
use base64::Engine;

pub(crate) struct GithubObjectSession {
    transport: Arc<dyn HttpTransport>,
    base: Url,
    authorization: HeaderValue,
    timeout: Duration,
}
impl GithubClient {
    pub(crate) fn object_origin_identity(&self) -> (String, String) {
        (
            self.http.policy.endpoint.as_str().into(),
            self.http.policy.account_id.clone(),
        )
    }

    pub(crate) async fn object_session(
        &self,
        repository: &RepositoryIdentity,
        hostname: &str,
    ) -> Result<GithubObjectSession> {
        if self.http.policy.endpoint.as_str() != "https://api.github.com/"
            || hostname != "github.com"
        {
            return Err(Error::new(
                "unsupported_git_origin",
                "object acquisition requires the native api.github.com to github.com HTTPS contract",
            ));
        }
        self.repository(repository).await?;
        let account = self.identity().await?;
        if account.display_name.is_empty() || account.display_name.contains([':', '\r', '\n']) {
            return Err(Error::new(
                "identity_mismatch",
                "verified GitHub account login is invalid",
            ));
        }
        let token = self
            .http
            .authorization
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| {
                Error::new(
                    "missing_secret",
                    "GitHub token cannot be used for HTTPS Git",
                )
            })?;
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{token}", account.display_name));
        let mut authorization = HeaderValue::from_str(&format!("Basic {encoded}"))
            .map_err(|_| Error::new("missing_secret", "GitHub token cannot be encoded safely"))?;
        authorization.set_sensitive(true);
        let mut base =
            Url::parse("https://github.com/").map_err(|_| Error::invalid("native GitHub URL"))?;
        for component in [&repository.owner, &repository.name] {
            if component.is_empty()
                || component.contains(['/', '\\', '\0', '\r', '\n'])
                || component == "."
                || component == ".."
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "repository path component is invalid",
                ));
            }
        }
        base.path_segments_mut()
            .map_err(|_| Error::invalid("GitHub path"))?
            .push(&repository.owner)
            .push(&format!("{}.git", repository.name));
        Ok(GithubObjectSession {
            transport: self.object_transport.clone(),
            base,
            authorization,
            timeout: self.http.policy.timeout,
        })
    }
}
impl GithubObjectSession {
    pub(crate) async fn advertisement(&self, max: usize) -> Result<Vec<u8>> {
        self.send(None, max).await
    }
    pub(crate) async fn fetch(&self, request: Vec<u8>, max: usize) -> Result<Vec<u8>> {
        self.send(Some(request), max).await
    }
    async fn send(&self, request: Option<Vec<u8>>, max: usize) -> Result<Vec<u8>> {
        if max == 0
            || max > MAX_VIDEO_BYTES
            || request.as_ref().is_some_and(|v| v.len() > 16 * 1024)
        {
            return Err(Error::invalid("Git HTTP bound is invalid"));
        }
        let is_fetch = request.is_some();
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Error::invalid("Git HTTP path"))?;
            if is_fetch {
                path.push("git-upload-pack");
            } else {
                path.push("info").push("refs");
            }
        }
        if !is_fetch {
            url.set_query(Some("service=git-upload-pack"));
        }
        let expected = if is_fetch {
            "application/x-git-upload-pack-result"
        } else {
            "application/x-git-upload-pack-advertisement"
        };
        let mut headers = HeaderMap::new();
        headers.insert("authorization", self.authorization.clone());
        headers.insert("git-protocol", HeaderValue::from_static("version=2"));
        headers.insert("accept", HeaderValue::from_static(expected));
        if is_fetch {
            headers.insert(
                "content-type",
                HeaderValue::from_static("application/x-git-upload-pack-request"),
            );
        }
        let response = tokio::time::timeout(
            self.timeout,
            self.transport.send_bounded(
                HttpRequest {
                    method: if is_fetch { Method::Post } else { Method::Get },
                    url,
                    headers,
                    body: request.unwrap_or_default(),
                },
                max,
            ),
        )
        .await
        .map_err(|_| Error::new("provider_transport", "bounded Git HTTPS read timed out"))??;
        if response.status != 200 {
            return Err(status_error(&response, true));
        }
        if response.body.len() > max {
            return Err(Error::new(
                "source_limit_exceeded",
                "Git HTTP response exceeds its bound",
            ));
        }
        if response
            .headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            != Some(expected)
            || response.headers.contains_key("location")
        {
            return Err(Error::new(
                "provider_response",
                "Git HTTPS response type or redirect is invalid",
            ));
        }
        Ok(response.body)
    }
}
