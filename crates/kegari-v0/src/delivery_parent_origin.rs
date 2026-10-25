//! Exact accepted native origin references, independent of a release plan.
//! This module produces historical references only, never parent readiness.
use super::*;
use crate::control::{ControlHost, Receipt, ReceiptPhase};
use crate::evidence::{VerifiedAcceptance, VerifiedParent};
use crate::storage::PrivateDir;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationReference {
    comment: Uuid,
    writer: Uuid,
    body_digest: String,
    receipt: Uuid,
    receipt_digest: String,
}

/// Serializable reference data with no public field constructor. Only the
/// private native reader below can produce it for the completion writer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptedOriginRef {
    runtime: Uuid,
    workspace: Uuid,
    work: WorkIdentity,
    token: String,
    acceptance_digest: String,
    parent: PublicationReference,
    acceptance: PublicationReference,
    vector_leaf: String,
    vector_signed_digest: String,
    vector_digest: String,
    contract_digest: String,
    baselines_digest: String,
    candidates_digest: String,
    slices_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    targets_digest: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedAcceptedOriginRefs {
    reference: AcceptedOriginRef,
    observed_at: DateTime<Utc>,
}
pub(crate) struct AcceptedOriginIdentity<'a> {
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub work: &'a WorkIdentity,
    pub token: &'a str,
    pub acceptance_digest: &'a str,
}
impl VerifiedAcceptedOriginRefs {
    pub(crate) fn reference(&self) -> &AcceptedOriginRef {
        &self.reference
    }
    pub(crate) fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}

impl AcceptedOriginRef {
    pub(crate) fn validate_retained(
        &self,
        root: &PrivateDir,
        control: &ControlHost,
        key: &AttestationKey,
        identity: AcceptedOriginIdentity<'_>,
    ) -> Result<()> {
        let AcceptedOriginIdentity {
            runtime,
            workspace,
            work,
            token,
            acceptance_digest,
        } = identity;
        work.validate()?;
        if self.runtime != runtime
            || self.workspace != workspace
            || &self.work != work
            || self.token != token
            || self.acceptance_digest != acceptance_digest
            || self.parent.comment == self.acceptance.comment
            || self.parent.receipt == self.acceptance.receipt
            || self.parent.writer != self.acceptance.writer
            || work.child.is_some()
        {
            return Err(origin_error(
                "accepted origin reference identifies another exact outcome",
            ));
        }
        let target = format!(
            "linear:{}:{}:{}",
            work.linear_endpoint, work.linear_workspace, work.parent
        );
        for reference in [&self.parent, &self.acceptance] {
            let receipt = control.operation(reference.receipt)?;
            if reference.comment.is_nil()
                || reference.writer.is_nil()
                || reference.receipt.is_nil()
                || receipt.operation_id != reference.receipt
                || receipt.request_id != reference.receipt
                || receipt.runtime_id != runtime
                || receipt.kind != "linear_comment"
                || !matches!(
                    receipt.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                )
                || canonical_digest(1, &receipt)? != reference.receipt_digest
                || receipt.canonical_targets != vec![target.clone()]
                || receipt.remote_ids.get("comment_id") != Some(&reference.comment.to_string())
                || receipt.expected.get("provider_body_digest") != Some(&reference.body_digest)
                || receipt.expected.get("provider_writer") != Some(&reference.writer.to_string())
            {
                return Err(origin_error(
                    "accepted origin publication receipt disappeared or changed",
                ));
            }
        }
        evidence::validate_digest(&self.vector_digest)?;
        if self.vector_leaf != format!("parent-vector-{}.json", &self.vector_digest[7..]) {
            return Err(origin_error(
                "accepted origin vector filename is not canonical",
            ));
        }
        // Read existing namespace only. A root shadow cannot replace this file.
        let records = PrivateDir::open(&root.path().join(workspace.to_string()))?;
        let vector: Signed<VectorRecord> = records.read(&self.vector_leaf)?;
        key.verify("parent_candidate_vector", &vector)?;
        let value = &vector.payload;
        if vector.digest()? != self.vector_signed_digest
            || value.schema_version != 1
            || value.runtime_id != runtime
            || value.work != *work
            || value.contract_digest != self.contract_digest
            || vector_digest(value)? != self.vector_digest
            || canonical_digest(1, &value.baselines)? != self.baselines_digest
            || canonical_digest(1, &value.candidates)? != self.candidates_digest
            || canonical_digest(1, &value.slices)? != self.slices_digest
            || match &self.targets_digest {
                Some(digest) => {
                    value.targets.is_empty() || canonical_digest(1, &value.targets)? != *digest
                }
                None => !value.targets.is_empty(),
            }
        {
            return Err(origin_error(
                "accepted origin vector is not the original complete candidate",
            ));
        }
        Ok(())
    }
}

fn origin_error(message: &str) -> Error {
    Error::new("accepted_origin", message)
}

impl DeliveryCoordinator {
    /// The current opaque parent/acceptance prove gates; this reader records
    /// their actual native/local reference identities without reissuing them.
    pub(crate) async fn accepted_origin_references(
        &self,
        gate: &FreshGate,
        parent: &VerifiedParent,
        acceptance: &VerifiedAcceptance,
        parent_comment: Uuid,
        acceptance_comment: Uuid,
    ) -> Result<VerifiedAcceptedOriginRefs> {
        (self.current_gate)(gate)?;
        let manifest = parent.manifest();
        let accepted = acceptance.acceptance();
        if gate.work() != &manifest.work
            || manifest.work.child.is_some()
            || accepted.work != manifest.work
            || accepted.reviewed_token != parent.token()
            || manifest.baselines.is_empty()
            || manifest
                .baselines
                .iter()
                .any(|b| b.successor_recovery_operation.is_some())
            || manifest.candidates.iter().any(|c| {
                !c.ordered_delta_digests.is_empty() || c.result.as_ref() != Some(&c.production)
            })
        {
            return Err(origin_error(
                "accepted origin is not exact unchanged ordinary production work",
            ));
        }
        let linear = self.broker.linear.as_ref().ok_or_else(|| {
            Error::new("prerequisite", "accepted origin Linear reader unavailable")
        })?;
        let actual = linear.identity().await?;
        let writer: Uuid = actual
            .account_id
            .parse()
            .map_err(|_| origin_error("accepted origin writer is not canonical"))?;
        let issue = linear.issue_key(manifest.work.parent)?;
        if issue.endpoint != manifest.work.linear_endpoint
            || issue.workspace != manifest.work.linear_workspace
            || accepted.provider_writer != writer.to_string()
        {
            return Err(origin_error(
                "accepted origin belongs to another native account/scope",
            ));
        }
        let comments = linear.comments(manifest.work.parent).await?;
        let select = |id| -> Result<&crate::providers::Comment> {
            let matching: Vec<_> = comments.iter().filter(|c| c.id == id).collect();
            let [comment] = matching.as_slice() else {
                return Err(origin_error(
                    "accepted native publication is missing or duplicated",
                ));
            };
            if comment.issue_id != manifest.work.parent
                || comment.user_id != Some(writer)
                || comment.bot
                || comment.delegated
                || bytes_digest(comment.body.as_bytes()) != comment.body_digest
            {
                return Err(origin_error(
                    "accepted native publication writer or body differs",
                ));
            }
            Ok(*comment)
        };
        let parent_comment = select(parent_comment)?;
        let acceptance_comment = select(acceptance_comment)?;
        if parent_comment.id == acceptance_comment.id {
            return Err(origin_error(
                "parent and acceptance publications are not distinct",
            ));
        }
        let signed: Signed<ParentResultRecord> =
            evidence::decode_signed_publication(&parent_comment.body, "parent_result", &self.key)?;
        let signed_acceptance: Signed<evidence::Acceptance> = evidence::decode_signed_publication(
            &acceptance_comment.body,
            "human_acceptance",
            &self.key,
        )?;
        if signed.payload.schema_version != 1
            || signed.payload.runtime_id != accepted.runtime_id
            || signed.payload.evidence != *manifest
            || signed_acceptance.payload != *accepted
            || signed_acceptance.digest()? != acceptance.digest()
            || signed.payload.evidence.token()? != parent.token()
        {
            return Err(origin_error(
                "native parent/acceptance differs from its verified evidence",
            ));
        }
        let receipts = self.host.operations()?;
        let retain =
            |comment: &crate::providers::Comment, parent: bool| -> Result<PublicationReference> {
                let matching: Vec<&Receipt> = receipts
                    .iter()
                    .filter(|receipt| {
                        receipt.kind == "linear_comment"
                            && receipt.runtime_id == accepted.runtime_id
                            && matches!(
                                receipt.phase,
                                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                            )
                            && receipt.operation_id == receipt.request_id
                            && receipt.remote_ids.get("comment_id") == Some(&comment.id.to_string())
                            && receipt.expected.get("provider_body_digest")
                                == Some(&comment.body_digest)
                            && receipt.expected.get("provider_writer") == Some(&writer.to_string())
                            && receipt.canonical_targets
                                == vec![format!(
                                    "linear:{}:{}:{}",
                                    issue.endpoint, issue.workspace, issue.issue
                                )]
                            && if parent {
                                receipt.expected.get("admission_operation")
                                    == Some(&manifest.publication_operation.to_string())
                            } else {
                                receipt.operation_id == accepted.operation_id
                                    && receipt.actor == accepted.control_actor
                            }
                    })
                    .collect();
                let [receipt] = matching.as_slice() else {
                    return Err(origin_error(
                        "accepted publication lacks one exact actual host receipt",
                    ));
                };
                Ok(PublicationReference {
                    comment: comment.id,
                    writer,
                    body_digest: comment.body_digest.clone(),
                    receipt: receipt.operation_id,
                    receipt_digest: canonical_digest(1, receipt)?,
                })
            };
        let parent_reference = retain(parent_comment, true)?;
        let acceptance_reference = retain(acceptance_comment, false)?;
        evidence::validate_digest(&signed.payload.vector_digest)?;
        let leaf = format!("parent-vector-{}.json", &signed.payload.vector_digest[7..]);
        let vector: Signed<VectorRecord> = self.records.read(&leaf)?;
        self.key.verify("parent_candidate_vector", &vector)?;
        let v = &vector.payload;
        let repositories: BTreeSet<_> = manifest
            .baselines
            .iter()
            .map(|b| b.repository.clone())
            .collect();
        if v.schema_version != 1
            || v.runtime_id != accepted.runtime_id
            || v.work != manifest.work
            || v.contract_digest != signed.payload.contract_digest
            || vector_digest(v)? != signed.payload.vector_digest
            || v.baselines != manifest.baselines
            || v.candidates != manifest.candidates
            || v.baselines.len() != repositories.len()
            || v.candidates.len() != repositories.len()
            || v.slices.len() != repositories.len()
            || v.targets
                .iter()
                .map(|target| target.target.clone())
                .collect::<BTreeSet<_>>()
                != manifest.production_targets
            || v.targets.len() != manifest.production_targets.len()
            || v.baselines
                .iter()
                .map(|b| b.repository.clone())
                .collect::<BTreeSet<_>>()
                != repositories
            || v.candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect::<BTreeSet<_>>()
                != repositories
            || v.slices
                .iter()
                .map(|(repo, _)| repo.clone())
                .collect::<BTreeSet<_>>()
                != repositories
            || v.slices.iter().any(|(_, operation)| operation.is_nil())
        {
            return Err(origin_error(
                "accepted vector repository/slice inventory is incomplete",
            ));
        }
        for baseline in &v.baselines {
            baseline.validate()?;
            let targets = v
                .targets
                .iter()
                .filter(|target| target.repository == baseline.repository)
                .collect::<Vec<_>>();
            let ids = targets
                .iter()
                .map(|target| target.target.clone())
                .collect::<BTreeSet<_>>();
            if baseline.successor_recovery_operation.is_some()
                || (baseline.deployment_free && !targets.is_empty())
                || (!baseline.deployment_free
                    && (targets.is_empty()
                        || ids != baseline.target_artifacts.keys().cloned().collect()
                        || ids != baseline.target_revisions.keys().cloned().collect()))
                || targets.iter().any(|target| {
                    target.source != baseline.production
                        || baseline.artifact_for_target(&target.target)
                            != Some(target.artifact_digest.as_str())
                        || baseline.source_for_target(&target.target)
                            != Some(target.source.commit.as_str())
                })
            {
                return Err(origin_error(
                    "accepted vector has incomplete exact production target inventory",
                ));
            }
        }
        (self.current_gate)(gate)?;
        Ok(VerifiedAcceptedOriginRefs {
            reference: AcceptedOriginRef {
                runtime: accepted.runtime_id,
                workspace: gate.workspace_id(),
                work: manifest.work.clone(),
                token: parent.token().into(),
                acceptance_digest: acceptance.digest().into(),
                parent: parent_reference,
                acceptance: acceptance_reference,
                vector_leaf: leaf,
                vector_signed_digest: vector.digest()?,
                vector_digest: signed.payload.vector_digest,
                contract_digest: signed.payload.contract_digest,
                baselines_digest: canonical_digest(1, &v.baselines)?,
                candidates_digest: canonical_digest(1, &v.candidates)?,
                slices_digest: canonical_digest(1, &v.slices)?,
                targets_digest: (!v.targets.is_empty())
                    .then(|| canonical_digest(1, &v.targets))
                    .transpose()?,
            },
            observed_at: Utc::now(),
        })
    }
}
