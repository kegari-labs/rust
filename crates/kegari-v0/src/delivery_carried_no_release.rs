//! Immutable no-release carry references. This reader cannot recover live carry
//! authority; only an already retained PublishedCarry can mint these references.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarriedNoReleaseOrigin {
    workspace: Uuid,
    saved_digest: String,
    carry: CarriedAcceptance,
    publication: (Uuid, String, Uuid),
    references: CarryReferences,
}
impl CarriedNoReleaseOrigin {
    pub(crate) fn carry(&self) -> &CarriedAcceptance {
        &self.carry
    }
    pub(crate) fn publication(&self) -> &(Uuid, String, Uuid) {
        &self.publication
    }
    pub(crate) fn validate_retained(
        &self,
        root: &PrivateDir,
        host: &ControlHost,
        key: &AttestationKey,
        outcome: &evidence::NoReleaseRequired,
        workspace: Uuid,
        original: &AcceptedOriginRef,
    ) -> Result<()> {
        if self.workspace != workspace || workspace.is_nil() {
            return Err(invalid("no-release carry belongs to another workspace"));
        }
        let records = PrivateDir::open(&root.path().join(workspace.to_string()))?;
        let saved: Signed<CarryPublicationRecord> =
            records.read(&leaf(&outcome.evidence_token)?)?;
        validate_saved(&saved, root, &records, host, key)?;
        let value = &saved.payload.signed.payload;
        let current = &value.current_parent.payload.evidence;
        let publication = (
            saved
                .payload
                .publication
                .record_id
                .parse::<Uuid>()
                .map_err(|_| invalid("carry comment identity is not canonical"))?,
            bytes_digest(
                evidence::encode_signed_publication(PURPOSE, &saved.payload.signed)?.as_bytes(),
            ),
            saved
                .payload
                .publication
                .writer_account_id
                .parse::<Uuid>()
                .map_err(|_| invalid("carry writer identity is not canonical"))?,
        );
        if saved.digest()? != self.saved_digest
            || value.workspace != workspace
            || value.runtime != outcome.runtime_id
            || current.work != outcome.work
            || current.token()? != outcome.evidence_token
            || value.carry != self.carry
            || publication != self.publication
            || value.carry.original_acceptance_digest != outcome.acceptance_digest
            || value.carry.created_at > outcome.recorded_at
            || canonical_digest(1, original)? != canonical_digest(1, &value.original)?
            || current.baselines != outcome.baselines
            || current.candidates.is_empty()
            || current.candidates.iter().any(|candidate| {
                !candidate.ordered_delta_digests.is_empty()
                    || !candidate.ordered_patch_digests.is_empty()
                    || candidate.result.as_ref() != Some(&candidate.production)
            })
        {
            return Err(invalid(
                "no-release carry differs from the original/current immutable outcome",
            ));
        }
        closed(&host.operation(value.publication_operation)?)?;
        self.references.validate(root, host, key)
    }
}
impl PublishedCarry {
    /// The current capability is verified before capturing immutable history.
    /// No deserialized origin can call this constructor in the reverse direction.
    pub(crate) fn no_release_origin(
        &self,
        config: &Config,
        outcome: &evidence::NoReleaseRequired,
    ) -> Result<(AcceptedOriginRef, CarriedNoReleaseOrigin)> {
        self.validate(config)?;
        let value = &self.saved.payload.signed.payload;
        let mut references = value.references.clone();
        references.retain_file(
            CarryNamespace::Workspace {
                workspace: value.workspace,
            },
            &self.records,
            &leaf(&outcome.evidence_token)?,
            SAVED_PURPOSE,
            &self.key,
        )?;
        references.retain_receipt(&self.host.operation(self.saved.payload.receipt.operation)?)?;
        references.retain_receipt(&self.host.operation(value.publication_operation)?)?;
        let result = CarriedNoReleaseOrigin {
            workspace: value.workspace,
            saved_digest: self.saved.digest()?,
            carry: value.carry.clone(),
            publication: self.publication_reference()?,
            references,
        };
        result.validate_retained(
            &self.root,
            &self.host,
            &self.key,
            outcome,
            value.workspace,
            &value.original,
        )?;
        Ok((value.original.clone(), result))
    }
}
