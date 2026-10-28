//! Hosted proof verification and durable replay admission remain host-owned.
use super::*;
use reqwest::header::HeaderMap;
use url::Url;

pub const GITHUB_OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";
pub const GITHUB_OIDC_JWKS: &str = "https://token.actions.githubusercontent.com/.well-known/jwks";
/// Expected values are captured by the host before dispatch, not copied from a
/// supplied token. sha is the triggering revision; tested_source_sha is bound in
/// the audience by the reviewed reusable workflow after checking its checkout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedProofExpectation {
    /// Additional exact evidence purpose, independently derived by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_digest: Option<String>,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub challenge: Uuid,
    pub repository_id: u64,
    pub repository_owner_id: u64,
    pub run_id: u64,
    pub run_attempt: u64,
    pub check_run_id: u64,
    pub actor_id: u64,
    pub trigger_sha: String,
    pub tested_source_sha: String,
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub job_workflow_ref: String,
    pub job_workflow_sha: String,
    pub event_name: String,
}
impl HostedProofExpectation {
    pub fn audience(&self) -> Result<String> {
        if self.runtime_id.is_nil()
            || self.operation_id.is_nil()
            || self.challenge.is_nil()
            || [
                self.repository_id,
                self.repository_owner_id,
                self.run_id,
                self.run_attempt,
                self.check_run_id,
                self.actor_id,
            ]
            .contains(&0)
        {
            return Err(Error::invalid(
                "hosted proof requires complete canonical ownership and job identities",
            ));
        }
        for s in [
            &self.trigger_sha,
            &self.tested_source_sha,
            &self.workflow_sha,
            &self.job_workflow_sha,
        ] {
            sha(s)?;
        }
        if !self.workflow_ref.contains("/.github/workflows/")
            || !self.job_workflow_ref.contains("/.github/workflows/")
            || !self
                .job_workflow_ref
                .ends_with(&format!("@{}", self.job_workflow_sha))
            || !matches!(
                self.event_name.as_str(),
                "workflow_dispatch" | "pull_request" | "push"
            )
        {
            return Err(Error::invalid(
                "hosted proof requires pinned reusable workflow and supported event",
            ));
        }
        let mut audience = json!({"runtime_id":self.runtime_id,"operation_id":self.operation_id,"challenge":self.challenge,"repository_id":self.repository_id,"source_sha":self.tested_source_sha});
        if let Some(context) = &self.context_digest {
            crate::evidence::validate_digest(context)?;
            audience["context_digest"] = json!(context);
        }
        Ok(format!(
            "urn:kegari:v0:hosted-proof:{}",
            canonical_digest(1, &audience)?
        ))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JwtHeader {
    alg: String,
    kid: String,
    typ: String,
    #[serde(default, rename = "x5t")]
    _thumbprint: Option<String>,
}
#[derive(Deserialize)]
struct GithubOidcClaims {
    iss: String,
    aud: String,
    sub: String,
    jti: String,
    iat: i64,
    nbf: i64,
    exp: i64,
    repository_id: String,
    repository_owner_id: String,
    run_id: String,
    run_attempt: String,
    check_run_id: String,
    actor_id: String,
    sha: String,
    workflow_ref: String,
    workflow_sha: String,
    job_workflow_ref: String,
    job_workflow_sha: String,
    runner_environment: String,
    event_name: String,
}
#[derive(Clone)]
struct RsaVerificationKey {
    n: Vec<u8>,
    e: Vec<u8>,
    digest: String,
}
pub struct HostedProofVerifier {
    keys: BTreeMap<String, RsaVerificationKey>,
}
/// This record contains no bearer token. It is written through private sealed
/// storage only after signature, temporal and exact job/source checks succeed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifiedHostedJob {
    pub schema_version: u32,
    pub expectation: HostedProofExpectation,
    pub token_digest: String,
    pub key_digest: String,
    pub jti_digest: String,
    pub verified_at: DateTime<Utc>,
    pub token_expires_at: DateTime<Utc>,
}
impl HostedProofVerifier {
    /// The production trust anchor is fixed to GitHub's documented TLS JWKS
    /// location. JWT jku/x5u and user-supplied discovery URLs are not supported.
    pub async fn fetch(timeout: Duration) -> Result<Self> {
        let transport = ReqwestTransport::new(timeout, 262_144)?;
        Self::fetch_with_transport(timeout, &transport).await
    }
    pub async fn fetch_with_transport(
        timeout: Duration,
        transport: &dyn HttpTransport,
    ) -> Result<Self> {
        if timeout.is_zero() {
            return Err(Error::invalid("JWKS timeout must be finite and positive"));
        }
        let response = tokio::time::timeout(
            timeout,
            transport.send(HttpRequest {
                method: Method::Get,
                url: Url::parse(GITHUB_OIDC_JWKS)
                    .map_err(|_| Error::invalid("invalid built-in issuer URL"))?,
                headers: HeaderMap::new(),
                body: Vec::new(),
            }),
        )
        .await
        .map_err(|_| Error::new("provider_transport", "GitHub signing key fetch timed out"))??;
        if response.status != 200 || response.body.len() > 262_144 {
            return Err(Error::new(
                "oidc_key_unavailable",
                "GitHub signing keys unavailable within bounds",
            ));
        }
        let value: Value = serde_json::from_slice(&response.body)
            .map_err(|_| Error::new("oidc_key_unavailable", "GitHub signing keys malformed"))?;
        Self::from_trusted_jwks(&value)
    }
    /// For offline verification/testing, the caller must establish trust in these
    /// keys independently. Production uses fetch; token contents never supply keys.
    pub fn from_trusted_jwks(value: &Value) -> Result<Self> {
        use base64::Engine;
        let entries = value
            .get("keys")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("oidc_key_unavailable", "JWKS keys missing"))?;
        if entries.is_empty() || entries.len() > 32 {
            return Err(Error::new(
                "oidc_key_unavailable",
                "JWKS key count outside bounds",
            ));
        }
        let mut keys = BTreeMap::new();
        for entry in entries {
            if text_at(entry, "/kty")? != "RSA"
                || text_at(entry, "/alg")? != "RS256"
                || text_at(entry, "/use")? != "sig"
            {
                return Err(Error::new(
                    "oidc_key_unavailable",
                    "JWKS contains unsupported key use or algorithm",
                ));
            }
            let kid = text_at(entry, "/kid")?;
            if kid.len() > 256 {
                return Err(Error::new("oidc_key_unavailable", "key ID exceeds bound"));
            }
            let decode = |path| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(text_at(entry, path)?)
                    .map_err(|_| {
                        Error::new("oidc_key_unavailable", "RSA key component encoding invalid")
                    })
            };
            let n = decode("/n")?;
            let e = decode("/e")?;
            if !(256..=1024).contains(&n.len()) || e.is_empty() || e.len() > 8 {
                return Err(Error::new(
                    "oidc_key_unavailable",
                    "RSA key size outside supported bounds",
                ));
            }
            let digest = canonical_digest(
                1,
                &json!({"kty":"RSA","n":text_at(entry,"/n")?,"e":text_at(entry,"/e")?}),
            )?;
            if keys
                .insert(kid.into(), RsaVerificationKey { n, e, digest })
                .is_some()
            {
                return Err(Error::new(
                    "oidc_key_unavailable",
                    "JWKS key IDs are ambiguous",
                ));
            }
        }
        Ok(Self { keys })
    }
    /// Verification and JTI ownership are committed under an OS lock. A token
    /// may be observed again only for the identical operation and expectation.
    /// Previously verified historical records can be read without reaccepting an
    /// expired token; their integrity remains the private storage contract.
    pub fn verify_and_record(
        &self,
        token: &str,
        expected: &HostedProofExpectation,
        now: DateTime<Utc>,
        directory: &kegari_storage::PrivateDir,
    ) -> Result<VerifiedHostedJob> {
        let candidate = self.verify(token, expected, now)?;
        let _lock = directory.section_lock("hosted-proof.lock")?;
        let leaf = format!(
            "hosted-proof-{}.json",
            candidate
                .jti_digest
                .strip_prefix("sha256:")
                .ok_or_else(|| Error::invalid("JTI digest format invalid"))?
        );
        match directory.read::<VerifiedHostedJob>(&leaf) {
            Ok(previous) => {
                if previous.expectation != candidate.expectation
                    || previous.token_digest != candidate.token_digest
                {
                    return Err(Error::new(
                        "proof_replay",
                        "OIDC token is already bound to another operation or evidence set",
                    ));
                }
                Ok(previous)
            }
            Err(error) if error.code == "record_missing" => {
                directory.write(&leaf, &candidate)?;
                Ok(candidate)
            }
            Err(error) => Err(error),
        }
    }
    fn verify(
        &self,
        token: &str,
        expected: &HostedProofExpectation,
        now: DateTime<Utc>,
    ) -> Result<VerifiedHostedJob> {
        use base64::Engine;
        let reject = || {
            Error::new(
                "invalid_hosted_proof",
                "signed hosted-job proof does not match the admitted evidence contract",
            )
        };
        if token.len() > 32_768 {
            return Err(reject());
        }
        let pieces = token.split('.').collect::<Vec<_>>();
        if pieces.len() != 3 {
            return Err(reject());
        }
        let decode = |s: &str| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(s)
                .map_err(|_| reject())
        };
        let header: JwtHeader =
            serde_json::from_slice(&decode(pieces[0])?).map_err(|_| reject())?;
        if header.alg != "RS256" || header.typ != "JWT" {
            return Err(reject());
        }
        let key = self.keys.get(&header.kid).ok_or_else(|| {
            Error::new(
                "oidc_key_unavailable",
                "token signing key is not in trusted GitHub key snapshot",
            )
        })?;
        let signed = format!("{}.{}", pieces[0], pieces[1]);
        ring::signature::RsaPublicKeyComponents {
            n: &key.n,
            e: &key.e,
        }
        .verify(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            signed.as_bytes(),
            &decode(pieces[2])?,
        )
        .map_err(|_| reject())?;
        let claims: GithubOidcClaims =
            serde_json::from_slice(&decode(pieces[1])?).map_err(|_| reject())?;
        let timestamp = now.timestamp();
        if claims.iss != GITHUB_OIDC_ISSUER
            || claims.aud != expected.audience()?
            || claims.sub.is_empty()
            || claims.sub.len() > 2048
            || claims.jti.is_empty()
            || claims.jti.len() > 256
            || claims.iat > timestamp.saturating_add(30)
            || claims.iat < timestamp.saturating_sub(600)
            || claims.nbf > timestamp.saturating_add(30)
            || claims.nbf > claims.iat
            || claims.exp <= timestamp.saturating_sub(30)
            || claims.exp <= claims.iat
            || claims.exp.saturating_sub(claims.iat) > 600
            || numeric_id(&claims.repository_id)? != expected.repository_id
            || numeric_id(&claims.repository_owner_id)? != expected.repository_owner_id
            || numeric_id(&claims.run_id)? != expected.run_id
            || numeric_id(&claims.run_attempt)? != expected.run_attempt
            || numeric_id(&claims.check_run_id)? != expected.check_run_id
            || numeric_id(&claims.actor_id)? != expected.actor_id
            || claims.sha != expected.trigger_sha
            || claims.workflow_sha != expected.workflow_sha
            || claims.workflow_ref != expected.workflow_ref
            || claims.job_workflow_sha != expected.job_workflow_sha
            || claims.job_workflow_ref != expected.job_workflow_ref
            || claims.runner_environment != "github-hosted"
            || claims.event_name != expected.event_name
        {
            return Err(reject());
        }
        Ok(VerifiedHostedJob {
            schema_version: 1,
            expectation: expected.clone(),
            token_digest: bytes_digest(token.as_bytes()),
            key_digest: key.digest.clone(),
            jti_digest: bytes_digest(claims.jti.as_bytes()),
            verified_at: now,
            token_expires_at: DateTime::from_timestamp(claims.exp, 0).ok_or_else(reject)?,
        })
    }
}
