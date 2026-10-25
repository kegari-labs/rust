//! Exercise the ordinary owned-baseline collector using the terminal fixture's
//! actual twice-resumed Control admissions and OIDC/native target proof. This
//! does not run another no-release publication or qualify synthetic upstream CI.
use super::*;

pub(super) async fn assert_unretired(workspace: &WorkspaceDelivery) {
    let contract = &workspace.contracts[0];
    let baseline = workspace.observer.observe(contract).await.unwrap();
    let error = workspace
        .observer
        .owned_baseline_references(contract, &baseline)
        .unwrap_err();
    assert_eq!(error.code, "reconciliation_required");
    assert_eq!(
        error.message,
        "release history lacks an exact closed release admission"
    );
}

pub(super) async fn assert_closed(
    f: &TerminalFixture,
    delivery: &RuntimeDelivery,
    workspace: &WorkspaceDelivery,
    permit: &Permit,
) {
    let contract = &workspace.contracts[0];
    let baseline = workspace.observer.observe(contract).await.unwrap();
    let references = workspace
        .observer
        .owned_baseline_references(contract, &baseline)
        .unwrap();
    let [reference] = references.as_slice() else {
        panic!("one current target required")
    };
    assert_eq!(reference.target().source, f.merged);
    let closed = delivery
        .runtime
        .control
        .closed_release_resume_receipts(permit.operation_id)
        .unwrap();
    assert_eq!(
        closed.len(),
        9,
        "origin plus two actual resume admission/command/authorization/completion chains"
    );
    let encoded = serde_json::to_value(reference).unwrap();
    let ids: BTreeSet<Uuid> = encoded["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| serde_json::from_value(item["operation"].clone()).unwrap())
        .collect();
    assert_eq!(
        ids.len(),
        closed.len() + 1,
        "full closed ancestry plus the actual target dispatch"
    );
    for receipt in &closed {
        assert!(ids.contains(&receipt.operation_id));
    }
    let dispatch: Vec<_> = ids
        .iter()
        .filter(|id| !closed.iter().any(|receipt| &receipt.operation_id == *id))
        .copied()
        .collect();
    let [dispatch] = dispatch.as_slice() else {
        panic!("exact dispatch required")
    };
    assert_eq!(
        delivery.runtime.control.operation(*dispatch).unwrap().kind,
        "github_dispatch"
    );
    let deployment =
        crate::storage::PrivateDir::open(&workspace.records.path().join("deployment")).unwrap();
    reference
        .validate_retained(
            &deployment,
            &delivery.runtime.control,
            &delivery.runtime.services.key,
        )
        .unwrap();
    let writes = || {
        f.data
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(_, body)| {
                body["query"]
                    .as_str()
                    .is_some_and(|query| query.contains("mutation KegariComment"))
            })
            .count()
    };
    let initial_writes = writes();
    let effects = f.github.release_effect_count();
    let receipts = canonical_digest(1, &delivery.runtime.control.operations().unwrap()).unwrap();
    let directory = crate::storage::PrivateDir::open(
        &delivery.runtime.registration.root().path().join("receipts"),
    )
    .unwrap();
    let _poll = f.observation_barrier.lock().await;
    // No current pointer or new provider read is needed to validate a retained
    // reference; every original/intermediate/last recovery receipt stays required.
    for receipt in &closed {
        let leaf = format!("{}.json", receipt.operation_id);
        let bytes = directory.read_bytes(&leaf, 1_048_576).unwrap();
        std::fs::remove_file(directory.path().join(&leaf)).unwrap();
        let missing = reference
            .validate_retained(
                &deployment,
                &delivery.runtime.control,
                &delivery.runtime.services.key,
            )
            .unwrap_err();
        assert_eq!(
            missing.code, "record_missing",
            "required closed receipt {leaf}"
        );
        directory.write_bytes(&leaf, &bytes).unwrap();
        let mut changed = receipt.clone();
        changed.actor = "substituted upstream recovery actor".into();
        directory.write(&leaf, &changed).unwrap();
        let replaced = reference
            .validate_retained(
                &deployment,
                &delivery.runtime.control,
                &delivery.runtime.services.key,
            )
            .unwrap_err();
        assert_eq!(replaced.code, "production_baseline");
        assert_eq!(replaced.message, "owned baseline immutable receipt changed");
        directory.write_bytes(&leaf, &bytes).unwrap();
        reference
            .validate_retained(
                &deployment,
                &delivery.runtime.control,
                &delivery.runtime.services.key,
            )
            .unwrap();
    }
    assert_eq!(writes(), initial_writes);
    assert_eq!(f.github.release_effect_count(), effects);
    assert_eq!(
        canonical_digest(1, &delivery.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}
