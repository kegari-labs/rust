//! Exact immutable request identity for an owned ordinary admission return.
//! These digests are comparison data; they never mint admission or exit authority.
use super::*;

pub(super) fn request(
    permit: &Permit,
    reference: Option<AdmissionReference>,
) -> Result<MutationRequest> {
    request_versioned(permit, reference, true)
}

// Legacy byte identity is retained for explicit historical comparison only.
// New admission and exact owned-return identity always use the current encoder.
fn request_versioned(
    permit: &Permit,
    reference: Option<AdmissionReference>,
    indexed: bool,
) -> Result<MutationRequest> {
    // A durable reference allows conservative recovery without persisting a shadow issue store.
    let mut expected = BTreeMap::from([
        ("scope".into(), permit.scope_digest.clone()),
        ("control".into(), permit.control_generation.to_string()),
    ]);
    if indexed && effect_index::eligible(reference.as_ref()) {
        expected.insert(effect_index::KEY.into(), "1".into());
    }
    if let Some(
        AdmissionReference::Run(reference)
        | AdmissionReference::ParentRun(reference, _, _)
        | AdmissionReference::RoundRun(reference, _)
        | AdmissionReference::ContinuedRun(reference, _)
        | AdmissionReference::SourceAdvancedRun(reference, _)
        | AdmissionReference::ResumedRun(reference, _)
        | AdmissionReference::RoundResumedRun(reference, _, _),
    ) = &reference
    {
        expected.insert(
            "run".into(),
            serde_json::to_string(&reference)
                .map_err(|_| Error::invalid("run reference encoding"))?,
        );
    }
    if let Some(AdmissionReference::ContinuedRun(_, continuation)) = &reference {
        expected.insert(
            "continuation".into(),
            serde_json::to_string(continuation)
                .map_err(|_| Error::invalid("continuation reference encoding"))?,
        );
    }
    if let Some(AdmissionReference::SourceAdvancedRun(_, source)) = &reference {
        expected.insert(
            "source_advanced".into(),
            serde_json::to_string(source)
                .map_err(|_| Error::invalid("source-advanced reference encoding"))?,
        );
    }
    if let Some(
        AdmissionReference::ResumedRun(_, resume)
        | AdmissionReference::RoundResumedRun(_, resume, _),
    ) = &reference
    {
        expected.insert(
            "session_resume".into(),
            serde_json::to_string(resume)
                .map_err(|_| Error::invalid("session resume reference encoding"))?,
        );
    }
    if let Some(
        AdmissionReference::Preparation(reference)
        | AdmissionReference::ParentPreparation(reference, _)
        | AdmissionReference::RoundPreparation(reference, _),
    ) = &reference
    {
        expected.insert(
            "worker_preparation".into(),
            serde_json::to_string(reference)
                .map_err(|_| Error::invalid("worker preparation reference encoding"))?,
        );
    }
    if let Some(
        AdmissionReference::SessionRecovery(reference)
        | AdmissionReference::RoundSessionRecovery(reference, _),
    ) = &reference
    {
        expected.insert(
            "session_recovery_preparation".into(),
            serde_json::to_string(reference)
                .map_err(|_| Error::invalid("recovery preparation encoding"))?,
        );
    }
    match &reference {
        Some(AdmissionReference::ParentPreparation(_, proof)) => {
            expected.insert(
                "parent_reassembly".into(),
                serde_json::to_string(proof.as_ref())
                    .map_err(|_| Error::invalid("parent reassembly encoding"))?,
            );
        }
        Some(AdmissionReference::ParentRun(_, proof, preparation)) => {
            expected.insert(
                "parent_reassembly".into(),
                serde_json::to_string(proof.as_ref())
                    .map_err(|_| Error::invalid("parent reassembly encoding"))?,
            );
            expected.insert(
                "parent_reassembly_preparation".into(),
                serde_json::to_string(preparation)
                    .map_err(|_| Error::invalid("parent reassembly preparation encoding"))?,
            );
        }
        Some(AdmissionReference::RoundPreparation(_, round)) => {
            expected.insert(
                "review_round".into(),
                serde_json::to_string(round.as_ref())
                    .map_err(|_| Error::invalid("round preparation encoding"))?,
            );
        }
        Some(AdmissionReference::RoundRun(_, provenance)) => provenance.persist(&mut expected)?,
        Some(AdmissionReference::RoundSessionRecovery(_, provenance)) => {
            provenance.persist(&mut expected)?
        }
        Some(AdmissionReference::RoundResumedRun(_, _, provenance)) => {
            provenance.persist(&mut expected)?
        }
        _ => (),
    }
    if let Some(AdmissionReference::Metadata(reference)) = reference {
        expected.insert(
            "metadata".into(),
            serde_json::to_string(&reference)
                .map_err(|_| Error::invalid("metadata reference encoding"))?,
        );
    } else if let Some(AdmissionReference::ReleaseResume(reference)) = reference {
        expected.insert(
            "release_resume".into(),
            serde_json::to_string(&reference)
                .map_err(|_| Error::invalid("release resume reference encoding"))?,
        );
    }
    Ok(MutationRequest {
        request_id: permit.operation_id,
        runtime_id: permit.lease.runtime_id,
        actor: Actor {
            identity: format!("owner:{}", permit.lease.owner_id),
            roles: BTreeSet::new(),
            principal: None,
        },
        kind: "admission".into(),
        canonical_targets: vec![permit.work_key.clone()],
        expected,
        payload: serde_json::to_value(permit).map_err(|_| Error::invalid("permit encoding"))?,
    })
}

pub(super) fn payload_digest(request: &MutationRequest) -> Result<String> {
    canonical_digest(
        1,
        &serde_json::json!({"kind":request.kind,"targets":request.canonical_targets,"expected":request.expected,"payload":request.payload,"principal":request.actor.principal}),
    )
}

#[derive(Serialize)]
struct Identity<'a> {
    operation: Uuid,
    request: Uuid,
    runtime: Uuid,
    owner: u64,
    actor: &'a str,
    kind: &'a str,
    targets: &'a [String],
    payload: &'a str,
    expected: &'a BTreeMap<String, String>,
}

pub(crate) fn receipt_request_digest(receipt: &Receipt) -> Result<String> {
    canonical_digest(
        1,
        &Identity {
            operation: receipt.operation_id,
            request: receipt.request_id,
            runtime: receipt.runtime_id,
            owner: receipt.owner_generation,
            actor: &receipt.actor,
            kind: &receipt.kind,
            targets: &receipt.canonical_targets,
            payload: &receipt.payload_digest,
            expected: &receipt.expected,
        },
    )
}

fn expected_digest(permit: &Permit, reference: AdmissionReference) -> Result<String> {
    let request = request(permit, Some(reference))?;
    canonical_digest(
        1,
        &Identity {
            operation: request.request_id,
            request: request.request_id,
            runtime: request.runtime_id,
            owner: permit.lease.owner_generation,
            actor: &request.actor.identity,
            kind: &request.kind,
            targets: &request.canonical_targets,
            payload: &payload_digest(&request)?,
            expected: &request.expected,
        },
    )
}

pub(crate) fn ordinary_preparation_request_digest(
    permit: &Permit,
    reference: &PreparationReference,
) -> Result<String> {
    reference.validate()?;
    if reference.operation_id()? != permit.operation_id
        || reference.phase.work != permit.work_key
        || reference.phase.retry_grant.is_some()
    {
        return Err(Error::new(
            "worker_binding",
            "ordinary preparation identity differs from actual permit",
        ));
    }
    expected_digest(permit, AdmissionReference::Preparation(reference.clone()))
}

pub(crate) fn ordinary_run_request_digest(
    permit: &Permit,
    reference: &RunReference,
) -> Result<String> {
    reference.validate()?;
    if reference.phase.work != permit.work_key || reference.phase.retry_grant.is_some() {
        return Err(Error::new(
            "worker_binding",
            "ordinary model identity differs from actual permit",
        ));
    }
    expected_digest(permit, AdmissionReference::Run(reference.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_parent_request_is_additive_and_legacy_encoding_stays_exact() {
        let digest = |v: &str| canonical_digest(1, &v).unwrap();
        let permit = Permit {
            operation_id: Uuid::from_u128(1),
            lease: Lease {
                runtime_id: Uuid::from_u128(2),
                owner_generation: 3,
                owner_id: Uuid::from_u128(4),
            },
            control_generation: 5,
            work_key: "parent".into(),
            scope_digest: digest("scope"),
            registered_at: "2026-10-26T00:00:00Z".parse().unwrap(),
        };
        let mut run = RunReference {
            workspace: Uuid::from_u128(6),
            account: "component".into(),
            normalized_state: None,
            phase: crate::scheduler::PhaseKey {
                work: "parent".into(),
                generation: digest("generation"),
                phase: "parent_evidence_review".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instruction"),
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 4,
            attempt: 1,
        };
        let current = request(&permit, Some(AdmissionReference::Run(run.clone()))).unwrap();
        let legacy =
            request_versioned(&permit, Some(AdmissionReference::Run(run.clone())), false).unwrap();
        let exact_old = MutationRequest {
            request_id: permit.operation_id,
            runtime_id: permit.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", permit.lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "admission".into(),
            canonical_targets: vec![permit.work_key.clone()],
            expected: BTreeMap::from([
                ("scope".into(), permit.scope_digest.clone()),
                ("control".into(), permit.control_generation.to_string()),
                ("run".into(), serde_json::to_string(&run).unwrap()),
            ]),
            payload: serde_json::to_value(&permit).unwrap(),
        };
        assert_eq!(
            payload_digest(&legacy).unwrap(),
            payload_digest(&exact_old).unwrap()
        );
        assert_eq!(legacy.expected, exact_old.expected);
        assert_eq!(legacy.payload, exact_old.payload);
        let mut only_new = current.expected.clone();
        assert_eq!(only_new.remove(effect_index::KEY), Some("1".into()));
        assert_eq!(only_new, legacy.expected);
        assert_ne!(
            payload_digest(&current).unwrap(),
            payload_digest(&legacy).unwrap()
        );
        run.phase.phase = "implementation".into();
        assert_eq!(
            payload_digest(&request(&permit, Some(AdmissionReference::Run(run.clone()))).unwrap())
                .unwrap(),
            payload_digest(
                &request_versioned(&permit, Some(AdmissionReference::Run(run)), false).unwrap()
            )
            .unwrap()
        );
    }
}
