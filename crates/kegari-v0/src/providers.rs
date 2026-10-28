//! Host authorization, receipt ownership and admission for provider effects.
use crate::control::{ControlHost, ControlSnapshot, MutationRequest, Permit, Receipt};
use chrono::{DateTime, Utc};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_providers::validation::*;
use kegari_providers::*;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

#[path = "providers_hosted.rs"]
mod hosted;
pub use hosted::{
    GITHUB_OIDC_ISSUER, GITHUB_OIDC_JWKS, HostedProofExpectation, HostedProofVerifier,
    VerifiedHostedJob,
};
#[path = "linear_assets.rs"]
mod linear_assets;
pub use linear_assets::{LinearAssetReference, VerifiedLinearAsset, download_uploaded_asset};

/// Canonical issue identity remains an authority-domain conversion.
pub fn linear_issue_key(client: &LinearClient, issue: Uuid) -> Result<crate::authority::IssueKey> {
    crate::authority::IssueKey::new(client.endpoint().as_str(), client.workspace_id(), issue)
}

/// Closed host command vocabulary. There is no raw GraphQL, arbitrary URL,
/// repository-settings mutation, force update, review approval or rerun command.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderMutation {
    LinearComment {
        issue: Uuid,
        body: String,
    },
    LinearState {
        issue: Uuid,
        expected_state: Uuid,
        state: Uuid,
    },
    LinearUploadRequest {
        issue: Uuid,
        filename: String,
        content_type: String,
        size: u32,
    },
    LinearUploadBytes {
        issue: Uuid,
        grant_operation: Uuid,
        asset_url: String,
        content_digest: String,
        bytes: Vec<u8>,
    },
    GithubComment {
        repository: RepositoryIdentity,
        pr: u64,
        expected_head: String,
        body: String,
    },
    GithubStatus {
        repository: RepositoryIdentity,
        head: String,
        state: StatusState,
        description: String,
        target_url: String,
    },
    GithubPullRequest {
        repository: RepositoryIdentity,
        head_ref: String,
        base_ref: String,
        expected_head: String,
        expected_base: String,
        title: String,
        body: String,
    },
    GithubMerge {
        repository: RepositoryIdentity,
        pr: u64,
        expected_head: String,
        expected_base: String,
    },
    GithubDispatch {
        repository: RepositoryIdentity,
        workflow_id: u64,
        workflow_path: String,
        reference: String,
        workflow_sha: String,
        source_sha: String,
        expected_production: Option<kegari_git::Revision>,
        source_input: String,
        operation_input: String,
        inputs: BTreeMap<String, String>,
    },
    GithubCancel {
        repository: RepositoryIdentity,
        run_id: u64,
        attempt: u64,
        workflow_id: u64,
        head: String,
        dispatch_operation: Uuid,
    },
    GithubCreateRef {
        repository: RepositoryIdentity,
        reference: String,
        head: String,
    },
    GithubUpdateRef {
        repository: RepositoryIdentity,
        reference: String,
        expected_head: String,
        head: String,
    },
    GithubBlob {
        repository: RepositoryIdentity,
        content_base64: String,
    },
    GithubTree {
        repository: RepositoryIdentity,
        base_tree: String,
        entries: Vec<TreeEntry>,
    },
    GithubCommit {
        repository: RepositoryIdentity,
        message: String,
        tree: String,
        parent: String,
        author: CommitIdentity,
        committer: CommitIdentity,
        expected_sha: String,
    },
}
impl std::fmt::Debug for ProviderMutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderMutation([PAYLOAD REDACTED])")
    }
}
#[derive(Clone, Default)]
pub struct ProviderBroker {
    pub linear: Option<LinearClient>,
    pub github: Option<GithubClient>,
}
impl ProviderBroker {
    /// Host-authorized mutations pass through this broker. A durable reservation precedes
    /// asynchronous preflight; final authorization and owned dispatch serialize
    /// with control changes. Network waits never retain the control mutex.
    pub async fn mutate<F>(
        &self,
        host: Arc<ControlHost>,
        request: MutationRequest,
        permit: Option<Permit>,
        operator_role: Option<String>,
        command: ProviderMutation,
        fresh_gate: F,
    ) -> Result<Receipt>
    where
        F: Fn() -> Result<()> + Send + 'static,
    {
        self.mutate_observed(host, request, permit, operator_role, command, move |_| {
            fresh_gate()
        })
        .await
    }
    /// Supplies the already locked host snapshot to final authorization. Callers
    /// must not reenter ControlHost while evaluating this gate.
    pub async fn mutate_observed<F>(
        &self,
        host: Arc<ControlHost>,
        mut request: MutationRequest,
        permit: Option<Permit>,
        operator_role: Option<String>,
        command: ProviderMutation,
        fresh_gate: F,
    ) -> Result<Receipt>
    where
        F: Fn(&ControlSnapshot) -> Result<()> + Send + 'static,
    {
        let (kind, targets, payload) = self.binding(&command)?;
        if let ProviderMutation::LinearComment { body, .. } = &command {
            request
                .expected
                .insert("provider_body_digest".into(), bytes_digest(body.as_bytes()));
            request.expected.insert(
                "provider_writer".into(),
                self.linear()?.account_id().to_owned(),
            );
            request.expected.insert(
                "provider_principal".into(),
                canonical_digest(1, &request.actor.principal)?,
            );
        }
        match &command {
            ProviderMutation::LinearUploadRequest {
                filename,
                content_type,
                size,
                ..
            } => {
                request.expected.extend(BTreeMap::from([
                    (
                        "provider_writer".into(),
                        self.linear()?.account_id().to_owned(),
                    ),
                    ("provider_upload_size".into(), size.to_string()),
                    ("provider_upload_filename".into(), filename.clone()),
                    ("provider_upload_content_type".into(), content_type.clone()),
                ]));
            }
            ProviderMutation::LinearUploadBytes {
                grant_operation,
                asset_url,
                content_digest,
                bytes,
                ..
            } => {
                request.expected.extend(BTreeMap::from([
                    (
                        "provider_writer".into(),
                        self.linear()?.account_id().to_owned(),
                    ),
                    ("provider_upload_grant".into(), grant_operation.to_string()),
                    ("provider_asset_url".into(), asset_url.clone()),
                    ("provider_content_digest".into(), content_digest.clone()),
                    ("provider_upload_size".into(), bytes.len().to_string()),
                ]));
            }
            _ => {}
        }
        request.kind = kind;
        request.canonical_targets = targets;
        request.payload = payload;
        let broker = self.clone();
        // This owned task survives HTTP/worker caller cancellation. A second
        // invocation observes the same Prepared/Sent receipt without resending.
        tokio::spawn(async move {
            let reservation = host.reserve_remote(request, permit, operator_role, &fresh_gate)?;
            let reservation = match reservation {
                crate::control::RemoteReservation::Existing(receipt) => return Ok(receipt),
                crate::control::RemoteReservation::Prepared(reservation) => reservation,
            };
            let operation = reservation.operation_id();
            let owned_host = host.clone();
            let task = tokio::spawn(async move {
                let preflight = async {
                    if let ProviderMutation::GithubCancel {
                        repository,
                        run_id,
                        workflow_id,
                        head,
                        dispatch_operation,
                        ..
                    } = &command
                    {
                        let dispatch = owned_host.operation(*dispatch_operation)?;
                        let observation = dispatch.observed_result.as_ref().ok_or_else(|| {
                            Error::new(
                                "reconciliation_required",
                                "dispatch has no verified acknowledgement",
                            )
                        })?;
                        if dispatch.kind != "github_dispatch"
                            || dispatch.remote_ids.get("workflow_run_id")
                                != Some(&run_id.to_string())
                            || observation.get("repository_id").and_then(Value::as_u64)
                                != Some(repository.id)
                            || observation.get("workflow_id").and_then(Value::as_u64)
                                != Some(*workflow_id)
                            || observation.get("workflow_sha").and_then(Value::as_str)
                                != Some(head.as_str())
                        {
                            return Err(Error::new(
                                "scope_conflict",
                                "cancellation is not bound to the durable owned dispatch",
                            ));
                        }
                    }
                    broker.preflight(&command).await
                }
                .await;
                let launch_host = owned_host.clone();
                let handle = tokio::runtime::Handle::current();
                let (effect, transport) = tokio::task::spawn_blocking(move || {
                    launch_host.launch_remote(reservation, preflight, fresh_gate, move |receipt| {
                        let operation = receipt.operation_id;
                        handle.spawn(async move { broker.transmit(&command, operation).await })
                    })
                })
                .await
                .map_err(|_| {
                    Error::new(
                        "provider_broker",
                        "provider dispatch task failed; consult receipt",
                    )
                })??;
                let observed = transport
                    .await
                    .map_err(|_| Error::new("provider_broker", "owned transport task failed"))
                    .and_then(|result| result);
                tokio::task::spawn_blocking(move || owned_host.complete_remote(effect, observed))
                    .await
                    .map_err(|_| {
                        Error::new(
                            "provider_broker",
                            "provider completion task failed; consult receipt",
                        )
                    })?
            });
            let result = task
                .await
                .map_err(|_| {
                    Error::new(
                        "provider_broker",
                        "owned provider task failed; consult receipt",
                    )
                })
                .and_then(|result| result);
            host.remote_task_exited(operation)?;
            result
        })
        .await
        .map_err(|_| {
            Error::new(
                "provider_broker",
                "provider owner task failed; consult receipt",
            )
        })?
    }

    pub(crate) fn independent_feedback_request(
        &self,
        mut request: MutationRequest,
        command: &ProviderMutation,
    ) -> Result<MutationRequest> {
        if !matches!(command, ProviderMutation::GithubComment { .. }) {
            return Err(Error::new(
                "observation_conflict",
                "independent diagnostic requires actual feedback command",
            ));
        }
        let (kind, targets, payload) = self.binding(command)?;
        request.kind = kind;
        request.canonical_targets = targets;
        request.payload = payload;
        Ok(request)
    }

    fn linear(&self) -> Result<&LinearClient> {
        self.linear
            .as_ref()
            .ok_or_else(|| Error::invalid("Linear provider is not configured"))
    }
    fn github(&self) -> Result<&GithubClient> {
        self.github
            .as_ref()
            .ok_or_else(|| Error::invalid("GitHub provider is not configured"))
    }
    fn binding(&self, command: &ProviderMutation) -> Result<(String, Vec<String>, Value)> {
        let mut payload = match command {
            ProviderMutation::LinearUploadBytes {
                issue,
                grant_operation,
                asset_url,
                content_digest,
                bytes,
            } => {
                json!({"kind":"linear_upload_bytes","issue":issue,"grant_operation":grant_operation,"asset_url":asset_url,"content_digest":content_digest,"size":bytes.len()})
            }
            _ => serde_json::to_value(command)
                .map_err(|_| Error::invalid("cannot encode provider command"))?,
        };
        let kind = text_at(&payload, "/kind")?.to_owned();
        let target = if let Some(issue) = payload.get("issue") {
            let linear = self.linear()?;
            let id = Uuid::parse_str(
                issue
                    .as_str()
                    .ok_or_else(|| Error::invalid("issue UUID missing"))?,
            )
            .map_err(|_| Error::invalid("issue UUID invalid"))?;
            if id.is_nil() {
                return Err(Error::invalid("issue UUID must be nonnil"));
            }
            format!(
                "linear:{}:{}:{}",
                linear.endpoint(),
                linear.workspace_id(),
                id
            )
        } else {
            let github = self.github()?;
            let repo: RepositoryIdentity = serde_json::from_value(payload["repository"].clone())
                .map_err(|_| Error::invalid("repository identity missing"))?;
            github.check_scope(&repo)?;
            format!("github:{}:{}", github.endpoint(), repo.id)
        };
        if let ProviderMutation::LinearUploadBytes {
            bytes,
            content_digest,
            ..
        } = command
        {
            if bytes_digest(bytes) != *content_digest {
                return Err(Error::invalid("upload bytes do not match immutable digest"));
            }
            let object = payload.as_object_mut().unwrap();
            object.remove("bytes");
            object.insert("size".into(), json!(bytes.len()));
        }
        let account = if payload.get("issue").is_some() {
            self.linear()?.account_id()
        } else {
            self.github()?.account_id()
        };
        Ok((
            kind,
            vec![target],
            json!({"account_id":account,"command":payload}),
        ))
    }
    async fn preflight(&self, command: &ProviderMutation) -> Result<()> {
        use ProviderMutation::*;
        match command {
            LinearComment { issue, body } => {
                // A maximum-size explicit operator correction carries a small
                // authenticated provenance suffix in addition to its 1MiB text.
                if body.trim().is_empty() || body.len() > 2 * 1024 * 1024 || body.contains('\0') {
                    return Err(Error::invalid(
                        "Linear comment exceeds its bounded UTF-8 envelope",
                    ));
                }
                self.linear()?.issue(*issue).await?;
            }
            LinearState {
                issue,
                expected_state,
                state,
            } => {
                self.linear()?
                    .preflight_state_change(*issue, *expected_state, *state)
                    .await?;
            }
            LinearUploadRequest {
                issue,
                filename,
                content_type,
                size,
            } => {
                self.linear()?.issue(*issue).await?;
                if filename.is_empty()
                    || filename.contains(['\0', '/', '\\', '\r', '\n'])
                    || content_type != "video/mp4"
                    || *size == 0
                    || *size > 256 * 1024 * 1024
                {
                    return Err(Error::invalid(
                        "recording upload requires a bounded named MP4 file",
                    ));
                }
            }
            LinearUploadBytes {
                issue,
                grant_operation,
                asset_url,
                content_digest,
                bytes,
            } => {
                self.linear()?
                    .preflight_upload_bytes(
                        *issue,
                        *grant_operation,
                        asset_url,
                        content_digest,
                        bytes,
                    )
                    .await?;
            }
            GithubComment {
                repository,
                pr,
                expected_head,
                body,
            } => {
                bounded_text(body)?;
                let current = self.github()?.pull_request(repository, *pr).await?;
                if current.head_sha != *expected_head {
                    return Err(Error::new("stale_generation", "PR head changed"));
                }
            }
            GithubStatus {
                repository,
                head,
                description,
                target_url,
                ..
            } => {
                self.github()?.commit(repository, sha(head)?).await?;
                if description.chars().count() > 140 {
                    return Err(Error::invalid("status description exceeds 140 characters"));
                }
                validated_url(target_url)?;
            }
            GithubPullRequest {
                repository,
                head_ref,
                base_ref,
                expected_head,
                expected_base,
                title,
                body,
            } => {
                bounded_text(title)?;
                bounded_text(body)?;
                let client = self.github()?;
                if client
                    .reference(repository, &format!("heads/{head_ref}"))
                    .await?
                    != *expected_head
                    || client
                        .reference(repository, &format!("heads/{base_ref}"))
                        .await?
                        != *expected_base
                {
                    return Err(Error::new(
                        "stale_generation",
                        "PR head or base reference changed",
                    ));
                }
            }
            GithubMerge {
                repository,
                pr,
                expected_head,
                expected_base,
            } => {
                let current = self.github()?.pull_request(repository, *pr).await?;
                if current.head_sha != *expected_head
                    || current.base_sha != *expected_base
                    || current.state != "open"
                    || current.draft
                {
                    return Err(Error::new(
                        "stale_generation",
                        "PR changed before squash merge",
                    ));
                }
            }
            GithubDispatch {
                repository,
                workflow_id,
                workflow_path,
                reference,
                workflow_sha,
                source_sha,
                expected_production,
                source_input,
                operation_input,
                inputs,
            } => {
                let client = self.github()?;
                if !reference.starts_with("heads/") && !reference.starts_with("tags/") {
                    return Err(Error::invalid(
                        "workflow dispatch requires an exact branch or tag ref",
                    ));
                }
                if client.reference(repository, reference).await? != *workflow_sha {
                    return Err(Error::new(
                        "stale_generation",
                        "dispatch ref no longer points at pinned workflow revision",
                    ));
                }
                sha(source_sha)?;
                if let Some(production) = expected_production {
                    production.validate()?;
                    if production.commit != *source_sha
                        || client.reference(repository, "heads/main").await? != production.commit
                        || text_at(
                            &client.commit(repository, &production.commit).await?,
                            "/tree/sha",
                        )? != production.tree
                    {
                        return Err(Error::new(
                            "stale_generation",
                            "production main/source tree changed before exact deployment dispatch",
                        ));
                    }
                }
                if source_input.is_empty()
                    || operation_input.is_empty()
                    || source_input == operation_input
                    || inputs.contains_key(source_input)
                    || inputs.contains_key(operation_input)
                {
                    return Err(Error::invalid(
                        "dispatch reserved inputs are invalid or overridden",
                    ));
                }
                let workflows = client.workflows(repository).await?;
                let workflow =
                    workflows
                        .iter()
                        .find(|w| w.id == *workflow_id)
                        .ok_or_else(|| {
                            Error::new(
                                "incompatible_repository",
                                "configured workflow ID is absent",
                            )
                        })?;
                if text_at(&workflow.data, "/path")? != workflow_path
                    || text_at(&workflow.data, "/state")? != "active"
                {
                    return Err(Error::new(
                        "incompatible_repository",
                        "configured workflow path/state differs",
                    ));
                }
            }
            GithubCancel {
                repository,
                run_id,
                attempt,
                workflow_id,
                head,
                dispatch_operation: _,
            } => {
                let client = self.github()?;
                let run = client.workflow_run(repository, *run_id, None).await?;
                if run.attempt != *attempt
                    || run.workflow_id != *workflow_id
                    || run.head_sha != *head
                    || run.actor_id.to_string() != client.account_id()
                    || run.triggering_actor_id != run.actor_id
                    || run.event != "workflow_dispatch"
                {
                    return Err(Error::new(
                        "scope_conflict",
                        "run attempt is not the verified owned dispatch",
                    ));
                }
                // The host must also check its signed operation/run association.
            }
            GithubCreateRef {
                repository,
                reference,
                head,
            } => {
                reference_components(reference)?;
                self.github()?.commit(repository, sha(head)?).await?;
            }
            GithubUpdateRef {
                repository,
                reference,
                expected_head,
                head,
            } => {
                let client = self.github()?;
                if client.reference(repository, reference).await? != *expected_head {
                    return Err(Error::new(
                        "stale_generation",
                        "owned branch changed before update",
                    ));
                }
                client.commit(repository, sha(head)?).await?;
            }
            GithubBlob {
                repository,
                content_base64,
            } => {
                self.github()?.repository(repository).await?;
                if content_base64.len() > MAX_BODY / 2
                    || !content_base64
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                {
                    return Err(Error::invalid(
                        "blob content exceeds bound or is not base64",
                    ));
                }
            }
            GithubTree {
                repository,
                base_tree,
                entries,
            } => {
                self.github()?.tree(repository, sha(base_tree)?).await?;
                if entries.is_empty() {
                    return Err(Error::invalid("tree delta must contain entries"));
                }
                let mut paths = BTreeSet::new();
                for entry in entries {
                    if entry.path.is_empty()
                        || entry.path.starts_with('/')
                        || entry.path.contains(['\0', '\\'])
                        || entry.path.split('/').any(|p| {
                            p.is_empty() || p == "." || p == ".." || p.eq_ignore_ascii_case(".git")
                        })
                        || !paths.insert(&entry.path)
                    {
                        return Err(Error::invalid("tree entry path is unsafe or duplicated"));
                    }
                    if let Some(s) = &entry.sha {
                        sha(s)?;
                    }
                }
            }
            GithubCommit {
                repository,
                message,
                tree,
                parent,
                author,
                committer,
                expected_sha,
            } => {
                bounded_text(message)?;
                author.validate()?;
                committer.validate()?;
                sha(expected_sha)?;
                self.github()?.tree(repository, sha(tree)?).await?;
                self.github()?.commit(repository, sha(parent)?).await?;
            }
        }
        Ok(())
    }
    async fn transmit(
        &self,
        command: &ProviderMutation,
        operation: Uuid,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        use ProviderMutation::*;
        match command {
            LinearComment { issue, body } => self.linear()?.write_comment(*issue, body).await,
            LinearState { issue, state, .. } => self.linear()?.write_state(*issue, *state).await,
            LinearUploadRequest {
                issue,
                filename,
                content_type,
                size,
            } => {
                self.linear()?
                    .write_upload_request(*issue, filename, content_type, *size, operation)
                    .await
            }
            LinearUploadBytes {
                grant_operation,
                content_digest,
                bytes,
                ..
            } => {
                self.linear()?
                    .write_upload_bytes(*grant_operation, content_digest, bytes)
                    .await
            }
            GithubComment {
                repository,
                pr,
                body,
                ..
            } => self.github()?.write_comment(repository, *pr, body).await,
            GithubStatus {
                repository,
                head,
                state,
                description,
                target_url,
            } => {
                self.github()?
                    .write_status(repository, head, state, description, target_url)
                    .await
            }
            GithubPullRequest {
                repository,
                head_ref,
                base_ref,
                expected_head,
                expected_base,
                title,
                body,
            } => {
                self.github()?
                    .write_pull_request(GithubPullRequestWrite {
                        repository,
                        head_ref,
                        base_ref,
                        expected_head,
                        expected_base,
                        title,
                        body,
                    })
                    .await
            }
            GithubMerge {
                repository,
                pr,
                expected_head,
                ..
            } => {
                self.github()?
                    .write_merge(repository, *pr, expected_head)
                    .await
            }
            GithubDispatch {
                repository,
                workflow_id,
                reference,
                workflow_sha,
                source_sha,
                source_input,
                operation_input,
                inputs,
                ..
            } => {
                self.github()?
                    .write_dispatch(
                        GithubDispatchWrite {
                            repository,
                            workflow_id: *workflow_id,
                            reference,
                            workflow_sha,
                            source_sha,
                            source_input,
                            operation_input,
                            inputs,
                        },
                        operation,
                    )
                    .await
            }
            GithubCancel {
                repository,
                run_id,
                attempt,
                ..
            } => {
                self.github()?
                    .write_cancel(repository, *run_id, *attempt)
                    .await
            }
            GithubCreateRef {
                repository,
                reference,
                head,
            } => {
                self.github()?
                    .write_create_ref(repository, reference, head)
                    .await
            }
            GithubUpdateRef {
                repository,
                reference,
                head,
                ..
            } => {
                self.github()?
                    .write_update_ref(repository, reference, head)
                    .await
            }
            GithubBlob {
                repository,
                content_base64,
            } => self.github()?.write_blob(repository, content_base64).await,
            GithubTree {
                repository,
                base_tree,
                entries,
            } => {
                self.github()?
                    .write_tree(repository, base_tree, entries)
                    .await
            }
            GithubCommit {
                repository,
                message,
                tree,
                parent,
                author,
                committer,
                expected_sha,
            } => {
                self.github()?
                    .write_commit(GithubCommitWrite {
                        repository,
                        message,
                        tree,
                        parent,
                        author,
                        committer,
                        expected_sha,
                    })
                    .await
            }
        }
    }
}
