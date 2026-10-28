//! Read-only discovery proves retained bytes and original effect lineage. It
//! does not prove current provider refs, admit work, fetch, or create directories.
use super::*;

pub(crate) fn retained_sources(
    records: &PrivateDir,
    control: &ControlHost,
    key: &AttestationKey,
    github: &GithubClient,
    contracts: &[RepositoryContract],
) -> Result<Vec<VerifiedObjectSource>> {
    records.revalidate()?;
    let base = records.path().join("object-sources");
    if !base
        .try_exists()
        .map_err(|e| Error::io("inspect retained object store", e))?
    {
        return Ok(vec![]);
    }
    let store = PrivateDir::open(&base)?;
    let mut leaves = Vec::new();
    for entry in
        fs::read_dir(store.path()).map_err(|e| Error::io("enumerate retained object sources", e))?
    {
        let entry = entry.map_err(|e| Error::io("read retained object source entry", e))?;
        if leaves.len() >= 1024 {
            return Err(Error::new(
                "record_limit",
                "retained object source inventory exceeds 1024 entries",
            ));
        }
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| Error::new("record_corrupt", "object source name is invalid"))?;
        let effect = leaf
            .strip_prefix("objects-")
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "unexpected entry in private object source store",
                )
            })?;
        leaves.push((leaf, effect));
    }
    leaves.sort();
    let runtime = control.snapshot()?.intent.runtime_id;
    let provider = github.object_origin_identity();
    let mut result = Vec::new();
    for (leaf, effect) in leaves {
        let directory = Arc::new(PrivateDir::open(&store.path().join(leaf))?);
        let bytes = match directory.read_bytes("source.json", 256 * 1024) {
            Ok(bytes) => bytes,
            Err(error) if error.code == "record_missing" => continue,
            Err(error) => return Err(error),
        };
        let signed: Signed<SourceRecord> = serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("record_corrupt", "retained source record malformed"))?;
        key.verify("verified_object_source", &signed)?;
        let record = signed.payload;
        let Some(contract) = contracts.iter().find(|c| c.repository == record.repository) else {
            continue;
        };
        // Old account/configuration artifacts remain preserved, without granting
        // a read root under newly selected authority.
        if record.contract_digest != contract.digest()?
            || (
                record.provider_endpoint.clone(),
                record.provider_account.clone(),
            ) != provider
        {
            continue;
        }
        record.revision.validate()?;
        let receipt = control.operation(effect)?;
        if record.operator_checkout != Path::new(&contract.configuration.source)
            || record.required_revisions.get(&record.revision.commit) != Some(&record.revision.tree)
            || record.git_version != kegari_git::SUPPORTED_GIT_VERSION
            || record.schema_version != 1
            || record.effect != effect
            || receipt.runtime_id != runtime
            || receipt.kind != "git/object-import"
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.expected.get("object_request") != Some(&record.request_digest)
            || receipt.expected.get("pack_digest") != Some(&record.pack_digest)
            || receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("source_digest"))
                .and_then(serde_json::Value::as_str)
                != Some(canonical_digest(1, &record)?.as_str())
        {
            return Err(Error::new(
                "reconciliation_required",
                "retained source lacks its exact acknowledged import lineage",
            ));
        }
        let objects: BTreeSet<String> =
            serde_json::from_slice(&directory.read_bytes("objects.json", 8 * 1024 * 1024)?)
                .map_err(|_| Error::new("record_corrupt", "retained object inventory malformed"))?;
        if canonical_digest(1, &objects)? != record.objects_digest
            || record.required_revisions.is_empty()
            || record.required_revisions.len() > 64
        {
            return Err(Error::new(
                "provenance_mismatch",
                "retained source closure inventory changed",
            ));
        }
        for (commit, tree) in &record.required_revisions {
            Revision {
                commit: commit.clone(),
                tree: tree.clone(),
            }
            .validate()?;
            if !objects.contains(commit) || !objects.contains(tree) {
                return Err(Error::new(
                    "provenance_mismatch",
                    "required source root is absent from original complete object inventory",
                ));
            }
        }
        let root = directory.path().join("quarantine/repository.git");
        let source = VerifiedObjectSource {
            directory,
            root,
            record,
        };
        source.revalidate_filesystem()?;
        result.push(source);
    }
    Ok(result)
}
impl VerifiedObjectSource {
    pub(crate) fn host_read_root(
        &self,
        configuration: &kegari_config::Config,
        workspace: Uuid,
        contract: &RepositoryContract,
    ) -> Result<crate::git::VerifiedHostReadRoot> {
        let selected = configuration
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "retained object workspace is not configured",
                )
            })?;
        let repository = selected
            .repositories
            .get(&contract.configuration_name)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "retained object repository is no longer selected",
                )
            })?;
        let connection = configuration
            .connections
            .get(&selected.github_connection)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "retained object provider is no longer selected",
                )
            })?;
        let endpoint = connection
            .endpoint
            .as_deref()
            .unwrap_or("https://api.github.com");
        let endpoint =
            url::Url::parse(endpoint).map_err(|_| Error::invalid("provider endpoint invalid"))?;
        if canonical_digest(1, repository)? != canonical_digest(1, &contract.configuration)?
            || self.record.contract_digest != contract.digest()?
            || self.record.repository != contract.repository
            || self.record.provider_account != connection.account_id
            || self.record.provider_endpoint != endpoint.as_str()
        {
            return Err(Error::new(
                "scope_violation",
                "retained object source differs from current exact repository/account contract",
            ));
        }
        self.revalidate_filesystem()?;
        crate::git::retained_object_read_root(self, crate::config::config_revision(configuration)?)
    }
}

/// Choose a retained source by all exact delta roots, including replacement
/// history, then recheck its actual complete Git object closure.
pub(crate) fn select_delta_source(
    sources: Vec<VerifiedObjectSource>,
    git: &Git,
    delta: &kegari_git::DeltaManifest,
) -> Result<Option<VerifiedObjectSource>> {
    for source in sources {
        if source.contains_delta(delta)? {
            source.verify_for_clone(git)?;
            return Ok(Some(source));
        }
    }
    Ok(None)
}
