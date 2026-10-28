//! Fixed provider writes used by the trusted backend after authorization.
//! These methods perform no host admission, reservation, retry or reconciliation.
use super::*;

/// Borrowed arguments for an exact pull-request write and acknowledgement.
pub struct GithubPullRequestWrite<'a> {
    pub repository: &'a RepositoryIdentity,
    pub head_ref: &'a str,
    pub base_ref: &'a str,
    pub expected_head: &'a str,
    pub expected_base: &'a str,
    pub title: &'a str,
    pub body: &'a str,
}
/// Borrowed arguments for a fixed workflow dispatch; host preconditions remain backend-owned.
pub struct GithubDispatchWrite<'a> {
    pub repository: &'a RepositoryIdentity,
    pub workflow_id: u64,
    pub reference: &'a str,
    pub workflow_sha: &'a str,
    pub source_sha: &'a str,
    pub source_input: &'a str,
    pub operation_input: &'a str,
    pub inputs: &'a BTreeMap<String, String>,
}
/// Borrowed arguments for a commit whose exact expected object is already host-validated.
pub struct GithubCommitWrite<'a> {
    pub repository: &'a RepositoryIdentity,
    pub message: &'a str,
    pub tree: &'a str,
    pub parent: &'a str,
    pub author: &'a CommitIdentity,
    pub committer: &'a CommitIdentity,
    pub expected_sha: &'a str,
}

/// Fixed wire writes for trusted backend use after host preflight and authorization.
impl LinearClient {
    pub async fn write_comment(
        &self,
        issue: Uuid,
        body: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let issue = &issue;
        let mut ids = BTreeMap::new();
        let observed = {
            let client = self;
            let v=client.graph("mutation KegariComment($input:CommentCreateInput!) { commentCreate(input:$input) { success comment { id issue { id } user { id } } } }",json!({"input":{"issueId":issue,"body":body}})).await?;
            mutation_success(&v, "/commentCreate/success")?;
            if uuid_at(&v, "/commentCreate/comment/issue/id")? != *issue
                || uuid_at(&v, "/commentCreate/comment/user/id")?.to_string()
                    != client.http.policy.account_id
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "created comment identity differs",
                ));
            }
            let id = uuid_at(&v, "/commentCreate/comment/id")?;
            ids.insert("comment_id".into(), id.to_string());
            json!({"comment_id":id,"body_digest":bytes_digest(body.as_bytes())})
        };
        Ok((ids, observed))
    }
    pub async fn write_state(
        &self,
        issue: Uuid,
        state: Uuid,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let issue = &issue;
        let state = &state;
        let ids = BTreeMap::new();
        let observed = {
            let v=self.graph("mutation KegariState($id:String!,$input:IssueUpdateInput!) { issueUpdate(id:$id,input:$input) { success issue { id state { id } } } }",json!({"id":issue,"input":{"stateId":state}})).await?;
            mutation_success(&v, "/issueUpdate/success")?;
            if uuid_at(&v, "/issueUpdate/issue/id")? != *issue
                || uuid_at(&v, "/issueUpdate/issue/state/id")? != *state
            {
                return Err(Error::new(
                    "provider_response",
                    "state update did not confirm requested state",
                ));
            }
            json!({"issue_id":issue,"state_id":state})
        };
        Ok((ids, observed))
    }
    pub async fn write_upload_request(
        &self,
        issue: Uuid,
        filename: &str,
        content_type: &str,
        size: u32,
        operation: Uuid,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let issue = &issue;
        let size = &size;
        let mut ids = BTreeMap::new();
        let observed = {
            let client = self;
            let v=client.graph("mutation KegariUpload($type:String!,$filename:String!,$size:Int!) { fileUpload(contentType:$type,filename:$filename,size:$size,makePublic:false) { success uploadFile { uploadUrl assetUrl headers { key value } contentType filename size } } }",json!({"type":content_type,"filename":filename,"size":size})).await?;
            mutation_success(&v, "/fileUpload/success")?;
            let grant = v
                .pointer("/fileUpload/uploadFile")
                .ok_or_else(|| Error::new("provider_response", "upload grant missing"))?;
            let upload_url = Url::parse(text_at(grant, "/uploadUrl")?)
                .map_err(|_| Error::new("provider_response", "upload URL invalid"))?;
            safe_upload_url(&upload_url)?;
            let asset_url = validated_url(text_at(grant, "/assetUrl")?)?;
            validate_https(
                &Url::parse(&asset_url)
                    .map_err(|_| Error::new("provider_response", "upload asset URL invalid"))?,
            )?;
            if text_at(grant, "/contentType")? != content_type
                || text_at(grant, "/filename")? != filename
                || grant.get("size").and_then(Value::as_u64) != Some(*size as u64)
            {
                return Err(Error::new(
                    "provider_response",
                    "upload grant content identity differs",
                ));
            }
            let mut headers = HeaderMap::new();
            headers.insert(
                "content-type",
                HeaderValue::from_str(content_type)
                    .map_err(|_| Error::invalid("invalid upload type"))?,
            );
            for header in grant
                .get("headers")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::new("provider_response", "upload grant headers missing"))?
            {
                let name = HeaderName::from_bytes(text_at(header, "/key")?.as_bytes())
                    .map_err(|_| Error::new("provider_response", "invalid upload header name"))?;
                if matches!(
                    name.as_str(),
                    "authorization"
                        | "cookie"
                        | "host"
                        | "proxy-authorization"
                        | "content-length"
                        | "transfer-encoding"
                ) {
                    return Err(Error::new(
                        "provider_response",
                        "upload grant requests forbidden header",
                    ));
                }
                let mut value = HeaderValue::from_str(text_at(header, "/value")?)
                    .map_err(|_| Error::new("provider_response", "invalid upload header value"))?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
            client
                .uploads
                .lock()
                .map_err(|_| Error::new("provider_broker", "upload grant state unavailable"))?
                .insert(
                    operation,
                    UploadGrant {
                        issue: *issue,
                        asset_url: asset_url.clone(),
                        upload_url,
                        headers,
                        size: *size as usize,
                        content_type: content_type.to_owned(),
                    },
                );
            ids.insert("upload_grant_operation".into(), operation.to_string());
            json!({"asset_url":asset_url,"grant_operation":operation,"uploaded":false,"associated":false})
        };
        Ok((ids, observed))
    }
    pub async fn write_upload_bytes(
        &self,
        grant_operation: Uuid,
        content_digest: &str,
        bytes: &[u8],
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let grant_operation = &grant_operation;
        let ids = BTreeMap::new();
        let observed = {
            let client = self;
            let (url, headers, asset_url, content_type) = {
                let grants = client
                    .uploads
                    .lock()
                    .map_err(|_| Error::new("provider_broker", "upload grant state unavailable"))?;
                let grant = grants.get(grant_operation).ok_or_else(|| {
                    Error::new("reconciliation_required", "upload grant unavailable")
                })?;
                (
                    grant.upload_url.clone(),
                    grant.headers.clone(),
                    grant.asset_url.clone(),
                    grant.content_type.clone(),
                )
            };
            let response = tokio::time::timeout(
                client.http.policy.timeout,
                client.http.transport.send(HttpRequest {
                    method: Method::Put,
                    url,
                    headers,
                    body: bytes.to_vec(),
                }),
            )
            .await
            .map_err(|_| {
                Error::new(
                    "provider_transport",
                    "recording upload timed out; reconcile before retry",
                )
            })??;
            if !(200..300).contains(&response.status) {
                return Err(status_error(&response, false));
            }
            json!({"asset_url":asset_url,"content_digest":content_digest,"content_type":content_type,"uploaded":true,"associated":false})
        };
        Ok((ids, observed))
    }
    /// Observe the issue before checking the configured transition, preserving preflight order.
    pub async fn preflight_state_change(
        &self,
        issue: Uuid,
        expected_state: Uuid,
        state: Uuid,
    ) -> Result<()> {
        let issue = &issue;
        let expected_state = &expected_state;
        let state = &state;
        let client = self;
        let current = client.issue(*issue).await?;
        let states = client
            .selection
            .states
            .get(&current.team_id.to_string())
            .ok_or_else(|| {
                Error::new(
                    "scope_conflict",
                    "issue team has no configured state mapping",
                )
            })?;
        if current.state_id != *expected_state
            || ![
                states.prepared,
                states.running,
                states.blocked,
                states.child_done,
                states.parent_accepted,
                states.cancelled,
            ]
            .contains(state)
        {
            return Err(Error::new(
                "stale_generation",
                "state or configured transition target changed",
            ));
        }
        Ok(())
    }
    /// Observe the issue before checking the exact retained upload grant and bytes.
    pub async fn preflight_upload_bytes(
        &self,
        issue: Uuid,
        grant_operation: Uuid,
        asset_url: &str,
        content_digest: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let issue = &issue;
        let grant_operation = &grant_operation;
        let client = self;
        client.issue(*issue).await?;
        if bytes_digest(bytes) != *content_digest {
            return Err(Error::invalid("upload content digest mismatch"));
        }
        let grants = client
            .uploads
            .lock()
            .map_err(|_| Error::new("provider_broker", "upload grant state unavailable"))?;
        let grant = grants.get(grant_operation).ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "upload grant is unavailable; do not recreate an uncertain upload",
            )
        })?;
        if grant.issue != *issue || grant.asset_url != *asset_url || grant.size != bytes.len() {
            return Err(Error::new(
                "request_conflict",
                "upload grant does not match recording",
            ));
        }
        Ok(())
    }
}

/// Fixed wire writes for trusted backend use after host preflight and authorization.
impl GithubClient {
    pub async fn write_comment(
        &self,
        repository: &RepositoryIdentity,
        pr: u64,
        body: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["issues", &nonzero(pr)?, "comments"])?;
        let (value, _) = self
            .http
            .json(Method::Post, url, Some(json!({"body":body})))
            .await?;
        record_acknowledgement(value)
    }
    pub async fn write_status(
        &self,
        repository: &RepositoryIdentity,
        head: &str,
        state: &StatusState,
        description: &str,
        target_url: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["statuses", sha(head)?])?;
        let body = json!({"state":state,"context":"kegari/codex-review","description":description,"target_url":target_url});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        record_acknowledgement(value)
    }
    pub async fn write_pull_request(
        &self,
        request: GithubPullRequestWrite<'_>,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let GithubPullRequestWrite {
            repository,
            head_ref,
            base_ref,
            expected_head,
            expected_base,
            title,
            body,
        } = request;
        let url = self.repo_url(repository, &["pulls"])?;
        let body = json!({"head":head_ref,"base":base_ref,"title":title,"body":body,"draft":false,"maintainer_can_modify":false});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        let pr = normalize_pr(value, repository.id)?;
        if pr.author_id.to_string() != self.http.policy.account_id
            || pr.head_sha != expected_head
            || pr.base_sha != expected_base
        {
            return Err(Error::new(
                "identity_mismatch",
                "created PR identity or revision differs",
            ));
        }
        let mut ids = BTreeMap::new();
        ids.insert("pull_request".into(), pr.number.to_string());
        let observed =
            serde_json::to_value(pr).map_err(|_| Error::invalid("cannot encode PR result"))?;
        Ok((ids, observed))
    }
    pub async fn write_merge(
        &self,
        repository: &RepositoryIdentity,
        pr: u64,
        expected_head: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["pulls", &nonzero(pr)?, "merge"])?;
        let body = json!({"sha":expected_head,"merge_method":"squash"});
        let (value, _) = self.http.json(Method::Put, url, Some(body)).await?;
        let merged = bool_at(&value, "/merged")?;
        let commit = sha(text_at(&value, "/sha")?)?;
        let mut ids = BTreeMap::new();
        ids.insert("acknowledged_merge_sha".into(), commit.into());
        Ok((
            ids,
            json!({"acknowledged":merged,"sha":commit,"provenance_verified":false}),
        ))
    }
    pub async fn write_dispatch(
        &self,
        request: GithubDispatchWrite<'_>,
        operation: Uuid,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let GithubDispatchWrite {
            repository,
            workflow_id,
            reference,
            workflow_sha,
            source_sha,
            source_input,
            operation_input,
            inputs,
        } = request;
        let mut inputs = inputs.clone();
        inputs.insert(source_input.to_owned(), source_sha.to_owned());
        inputs.insert(operation_input.to_owned(), operation.to_string());
        let url = self.repo_url(
            repository,
            &["actions", "workflows", &nonzero(workflow_id)?, "dispatches"],
        )?;
        let body = json!({"ref":reference.strip_prefix("heads/").or_else(||reference.strip_prefix("tags/")).ok_or_else(||Error::invalid("dispatch ref invalid"))?,"inputs":inputs,"return_run_details":true});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        let mut ids = BTreeMap::new();
        let observed = if value.is_null() {
            json!({"accepted":true,"run_identity":"unavailable","requires_correlation":true})
        } else {
            let run = positive_id(&value, "/workflow_run_id")?;
            ids.insert("workflow_run_id".into(), run.to_string());
            json!({"accepted":true,"workflow_run_id":run,"operation_id":operation,"repository_id":repository.id,"workflow_id":workflow_id,"workflow_sha":workflow_sha,"source_sha":source_sha,"requires_observation":true})
        };
        Ok((ids, observed))
    }
    pub async fn write_cancel(
        &self,
        repository: &RepositoryIdentity,
        run_id: u64,
        attempt: u64,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(
            repository,
            &["actions", "runs", &nonzero(run_id)?, "cancel"],
        )?;
        self.http.json(Method::Post, url, Some(json!({}))).await?;
        Ok((
            BTreeMap::new(),
            json!({"cancellation_requested":true,"run_id":run_id,"attempt":attempt,"requires_observation":true}),
        ))
    }
    pub async fn write_create_ref(
        &self,
        repository: &RepositoryIdentity,
        reference: &str,
        head: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["git", "refs"])?;
        let body = json!({"ref":format!("refs/{reference}"),"sha":head});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        reference_acknowledgement(value, reference, head)
    }
    pub async fn write_update_ref(
        &self,
        repository: &RepositoryIdentity,
        reference: &str,
        head: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let mut parts = vec!["git", "refs"];
        parts.extend(reference_components(reference)?);
        let url = self.repo_url(repository, &parts)?;
        let body = json!({"sha":head,"force":false});
        let (value, _) = self.http.json(Method::Patch, url, Some(body)).await?;
        reference_acknowledgement(value, reference, head)
    }
    pub async fn write_blob(
        &self,
        repository: &RepositoryIdentity,
        content_base64: &str,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["git", "blobs"])?;
        let body = json!({"content":content_base64,"encoding":"base64"});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        object_acknowledgement(value, None)
    }
    pub async fn write_tree(
        &self,
        repository: &RepositoryIdentity,
        base_tree: &str,
        entries: &[TreeEntry],
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let url = self.repo_url(repository, &["git", "trees"])?;
        let body = json!({"base_tree":base_tree,"tree":entries.iter().map(|e|json!({"path":e.path,"mode":e.mode,"type":e.mode.kind(),"sha":e.sha})).collect::<Vec<_>>()});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        object_acknowledgement(value, None)
    }
    pub async fn write_commit(
        &self,
        request: GithubCommitWrite<'_>,
    ) -> Result<(BTreeMap<String, String>, Value)> {
        let GithubCommitWrite {
            repository,
            message,
            tree,
            parent,
            author,
            committer,
            expected_sha,
        } = request;
        let url = self.repo_url(repository, &["git", "commits"])?;
        let body = json!({"message":message,"tree":tree,"parents":[parent],"author":author,"committer":committer});
        let (value, _) = self.http.json(Method::Post, url, Some(body)).await?;
        object_acknowledgement(value, Some(expected_sha))
    }
}
fn record_acknowledgement(value: Value) -> Result<(BTreeMap<String, String>, Value)> {
    let id = positive_id(&value, "/id")?;
    let mut ids = BTreeMap::new();
    ids.insert("record_id".into(), id.to_string());
    Ok((ids, json!({"id":id,"requires_observation":true})))
}
fn reference_acknowledgement(
    value: Value,
    reference: &str,
    head: &str,
) -> Result<(BTreeMap<String, String>, Value)> {
    if text_at(&value, "/ref")? != format!("refs/{reference}")
        || text_at(&value, "/object/sha")? != head
    {
        return Err(Error::new(
            "provider_response",
            "ref mutation acknowledgement differs",
        ));
    }
    Ok((
        BTreeMap::new(),
        json!({"reference":reference,"sha":head,"requires_observation":true}),
    ))
}
fn object_acknowledgement(
    value: Value,
    expected_sha: Option<&str>,
) -> Result<(BTreeMap<String, String>, Value)> {
    let object = sha(text_at(&value, "/sha")?)?;
    if expected_sha.is_some_and(|expected| object != expected) {
        return Err(Error::new(
            "identity_mismatch",
            "GitHub-created commit SHA differs from exact validated local commit",
        ));
    }
    let mut ids = BTreeMap::new();
    ids.insert("object_sha".into(), object.into());
    Ok((ids, json!({"sha":object,"requires_observation":true})))
}
