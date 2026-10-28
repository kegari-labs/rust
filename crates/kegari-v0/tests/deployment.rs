//! Synthetic provider transport and real private journal/proof validation.
use kegari_v0 as fixture_api;
#[path = "support/deployment_wire.rs"]
mod wire;
use wire::*;

#[tokio::test]
async fn prepared_proof_owner_requires_actual_live_registration_after_restart() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    assert_eq!(
        f.host.operation(p.operation_id).unwrap().phase,
        fixture_api::control::ReceiptPhase::Prepared
    );
    f.observer.recover_target(&c, "deploy").await.unwrap();
    let Fixture {
        _temp,
        observer,
        host,
        github,
        records,
        ..
    } = f;
    drop(observer);
    drop(host);
    let recovered = Arc::new(
        ControlHost::recover(
            PrivateDir::open(&records.parent().unwrap().join("control")).unwrap(),
            id(1),
        )
        .unwrap(),
    );
    assert!(recovered.snapshot().unwrap().active_operations.is_empty());
    let reader = ProductionObserver::new(
        github,
        recovered,
        Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
        PrivateDir::open(&records).unwrap(),
        Arc::new(|_| panic!("read-only proof recovery must not request execution authority")),
    )
    .unwrap();
    assert_eq!(
        reader.recover_target(&c, "deploy").await.unwrap_err().code,
        "production_baseline"
    );
    assert_eq!(
        reader
            .recover_historical_target(&c, &proof.historical_reference().unwrap())
            .await
            .unwrap_err()
            .code,
        "production_baseline"
    );
}

#[tokio::test]
async fn retired_current_proof_recovers_without_indexes_tokens_or_new_writes() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    f.host
        .retire_local(p.operation_id, json!({"fixture":"owned task joined"}))
        .unwrap();
    let records = PrivateDir::open(&f.records).unwrap();
    for leaf in records.records().unwrap() {
        if leaf.starts_with("target-index-")
            || leaf.starts_with("target-dispatch-")
            || leaf.starts_with("target-digest-")
        {
            std::fs::remove_file(f.records.join(leaf)).unwrap();
        }
    }
    let inventory = records.records().unwrap();
    let receipts = f.host.operations().unwrap();
    {
        let mut state = f.state.lock().unwrap();
        state.archive.clear();
        state.requests.clear();
    }
    let reader = ProductionObserver::new(
        f.github.clone(),
        f.host.clone(),
        Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
        PrivateDir::open(&f.records).unwrap(),
        Arc::new(|_| panic!("retired proof lookup must not regain execution authority")),
    )
    .unwrap();
    assert_eq!(
        reader
            .recover_target(&c, "deploy")
            .await
            .unwrap()
            .digest()
            .unwrap(),
        proof.digest().unwrap()
    );
    assert!(reader.observe_recovery(&c).await.unwrap().coherent());
    f.state.lock().unwrap().main = "c".repeat(40);
    assert!(!reader.observe_recovery(&c).await.unwrap().coherent());
    assert!(reader.recover_target(&c, "deploy").await.is_err());
    assert_eq!(records.records().unwrap(), inventory);
    assert_eq!(
        serde_json::to_value(f.host.operations().unwrap()).unwrap(),
        serde_json::to_value(receipts).unwrap()
    );
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(url, body)| body.is_null() && !url.contains("/artifacts"))
    );
}

#[tokio::test]
async fn retired_replacement_recovers_from_authenticated_stale_pointer_after_restart() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let original = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let records = PrivateDir::open(&f.records).unwrap();
    let pointer = records
        .records()
        .unwrap()
        .into_iter()
        .find(|name| name.starts_with("target-index-"))
        .unwrap();
    let old_pointer: Value = records.read(&pointer).unwrap();
    let replacement = f.replace_target(&c, &a, &p).await;
    for permit in f.host.snapshot().unwrap().active_operations {
        f.host
            .retire_local(
                permit.operation_id,
                json!({"fixture":"replacement task joined"}),
            )
            .unwrap();
    }
    // Reproduce a crash after the new proof/index writes but before replacement
    // of the old current pointer. Both actual admissions are now retired.
    records.write(&pointer, &old_pointer).unwrap();
    let inventory = records.records().unwrap();
    {
        let mut state = f.state.lock().unwrap();
        state.archive.clear();
        state.requests.clear();
    }
    let Fixture {
        _temp,
        observer,
        host,
        github,
        state,
        records: path,
    } = f;
    drop(observer);
    drop(host);
    let control = Arc::new(
        ControlHost::recover(
            PrivateDir::open(&path.parent().unwrap().join("control")).unwrap(),
            id(1),
        )
        .unwrap(),
    );
    assert!(control.snapshot().unwrap().active_operations.is_empty());
    let reader = ProductionObserver::new(
        github,
        control,
        Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
        PrivateDir::open(&path).unwrap(),
        Arc::new(|_| panic!("read-only replacement recovery has no permit")),
    )
    .unwrap();
    assert_eq!(
        reader
            .recover_target(&c, "deploy")
            .await
            .unwrap()
            .digest()
            .unwrap(),
        replacement.digest().unwrap()
    );
    assert!(reader.observe_recovery(&c).await.unwrap().coherent());
    assert_eq!(
        reader
            .recover_historical_target(&c, &original.historical_reference().unwrap())
            .await
            .unwrap()
            .digest()
            .unwrap(),
        original.digest().unwrap()
    );
    assert_eq!(records.records().unwrap(), inventory);
    assert_eq!(records.read::<Value>(&pointer).unwrap(), old_pointer);
    assert!(
        state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(url, body)| body.is_null() && !url.contains("/artifacts"))
    );
}

#[tokio::test]
async fn missing_current_pointer_rejects_ambiguous_mismatched_or_malformed_proofs() {
    for case in [
        "ambiguous",
        "mismatch",
        "malformed_pointer",
        "missing_referenced_proof",
    ] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        f.observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap();
        f.host
            .retire_local(p.operation_id, json!({"fixture":"owned task joined"}))
            .unwrap();
        let records = PrivateDir::open(&f.records).unwrap();
        let inventory = records.records().unwrap();
        let pointer = inventory
            .iter()
            .find(|name| name.starts_with("target-index-"))
            .unwrap();
        let proof = inventory
            .iter()
            .find(|name| name.starts_with("target-proof-"))
            .unwrap();
        if case == "malformed_pointer" {
            records
                .write(
                    pointer,
                    &json!({"invalid":"present pointer must not trigger fallback"}),
                )
                .unwrap();
        } else if case == "missing_referenced_proof" {
            std::fs::remove_file(f.records.join(proof)).unwrap();
        } else {
            std::fs::remove_file(f.records.join(pointer)).unwrap();
            if case == "mismatch" {
                f.state.lock().unwrap().deployment_id = 92;
            } else {
                // A second genuinely signed record with the same current native
                // identity is ambiguous, even if only its observation time differs.
                let signed: fixture_api::evidence::Signed<Value> = records.read(proof).unwrap();
                let mut payload = signed.payload;
                payload["target"]["observed_at"] = json!("2026-01-01T00:00:00Z");
                let duplicate = AttestationKey::new("fixture".into(), vec![7; 32])
                    .unwrap()
                    .sign("target_deployment", &payload)
                    .unwrap();
                let leaf = format!(
                    "target-proof-{}.json",
                    duplicate.digest().unwrap().rsplit(':').next().unwrap()
                );
                assert_ne!(&leaf, proof);
                records.write(&leaf, &duplicate).unwrap();
            }
        }
        assert!(
            f.observer.recover_target(&c, "deploy").await.is_err(),
            "{case}"
        );
        assert!(f.observer.observe_recovery(&c).await.is_err(), "{case}");
    }
}

#[tokio::test]
async fn owned_proof_reobservation_repairs_interrupted_indexes_without_new_token_or_dispatch() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let records = PrivateDir::open(&f.records).unwrap();
    for leaf in records.records().unwrap() {
        if leaf.starts_with("target-index-")
            || leaf.starts_with("target-dispatch-")
            || leaf.starts_with("target-digest-")
        {
            std::fs::remove_file(f.records.join(leaf)).unwrap();
        }
    }
    {
        let mut state = f.state.lock().unwrap();
        state.archive.clear();
        state.requests.clear();
    }
    assert_eq!(
        f.observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap()
            .digest()
            .unwrap(),
        proof.digest().unwrap()
    );
    assert_eq!(
        f.observer
            .recover_target(&c, "deploy")
            .await
            .unwrap()
            .digest()
            .unwrap(),
        proof.digest().unwrap()
    );
    assert!(
        records
            .records()
            .unwrap()
            .iter()
            .any(|name| name.starts_with("target-digest-"))
    );
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(url, body)| body.is_null() && !url.contains("/artifacts"))
    );
}

#[tokio::test]
async fn immutable_history_survives_replacement_and_exact_attempt_human_rerun() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let original = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let reference = original.historical_reference().unwrap();
    // Repeated observation preserves the original digest without consuming a JWT twice.
    assert_eq!(
        f.observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap()
            .digest()
            .unwrap(),
        original.digest().unwrap()
    );
    let replacement = f.replace_target(&c, &a, &p).await;
    assert_ne!(replacement.digest().unwrap(), original.digest().unwrap());
    assert_eq!(
        f.observer
            .recover_target(&c, "deploy")
            .await
            .unwrap()
            .deployment_id(),
        92
    );
    assert_eq!(
        f.observer
            .observe(&c)
            .await
            .unwrap()
            .baseline()
            .production
            .commit,
        "c".repeat(40)
    );
    {
        let mut state = f.state.lock().unwrap();
        state.responses.insert(
            "/repos/fixture/repo/actions/runs/50".into(),
            json!({"id":50,"run_attempt":2}),
        );
        state.requests.clear();
    }
    let restarted = ProductionObserver::new(
        f.github.clone(),
        f.host.clone(),
        Arc::new(AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
        PrivateDir::open(&f.records).unwrap(),
        Arc::new(|_| {
            Err(Error::new(
                "fixture_no_execution",
                "history has no execution authority",
            ))
        }),
    )
    .unwrap();
    let historical = restarted
        .recover_historical_target(&c, &reference)
        .await
        .unwrap();
    assert_eq!(historical.digest().unwrap(), original.digest().unwrap());
    assert_eq!(historical.target().source, source());
    let requests = &f.state.lock().unwrap().requests;
    assert!(requests.iter().all(|(_, body)| body.is_null()));
    assert!(
        requests
            .iter()
            .any(|(url, _)| url.ends_with("/actions/runs/50/attempts/1"))
    );
    assert!(
        !requests
            .iter()
            .any(|(url, _)| url.ends_with("/actions/runs/50")
                || url.contains("heads/main")
                || url.contains("/artifacts"))
    );
}

#[tokio::test]
async fn historical_target_rejects_native_revision_changes_and_incomplete_status_history() {
    for change in [
        "missing_status",
        "status_actor",
        "status_time",
        "status_state",
        "source",
        "creator",
        "attempt",
        "job",
        "environment",
        "last_page",
        "proof",
    ] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        let original = f
            .observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap();
        f.preserve_original();
        {
            let mut s = f.state.lock().unwrap();
            s.main = "c".repeat(40);
            match change {
                "missing_status" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91/statuses")
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .pop();
                }
                "status_actor" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91/statuses")
                        .unwrap()[1]["creator"]["id"] = json!(100)
                }
                "status_time" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91/statuses")
                        .unwrap()[1]["updated_at"] = json!("2026-02-01T00:00:00Z")
                }
                "status_state" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91/statuses")
                        .unwrap()[1]["state"] = json!("failure")
                }
                "source" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91")
                        .unwrap()["sha"] = json!("c".repeat(40))
                }
                "creator" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/deployments/91")
                        .unwrap()["creator"]["id"] = json!(100)
                }
                "attempt" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/actions/runs/50/attempts/1")
                        .unwrap()["run_attempt"] = json!(2)
                }
                "job" => {
                    s.responses
                        .get_mut("/repos/fixture/repo/actions/runs/50/attempts/1/jobs")
                        .unwrap()["jobs"][0]["conclusion"] = json!("failure")
                }
                "environment" => s.environment = 89,
                "last_page" => s.status_page_failure = true,
                "proof" => {}
                _ => unreachable!(),
            }
        }
        if change == "proof" {
            let records = PrivateDir::open(&f.records).unwrap();
            let leaf = records
                .records()
                .unwrap()
                .into_iter()
                .find(|n| n.starts_with("target-proof-"))
                .unwrap();
            let mut signed: Value = records.read(&leaf).unwrap();
            signed["payload"]["facts"]["status_id"] = json!(94);
            records.write(&leaf, &signed).unwrap();
        }
        assert!(
            f.observer
                .recover_historical_target(&c, &original.historical_reference().unwrap())
                .await
                .is_err(),
            "{change}"
        );
    }
}

#[tokio::test]
async fn historical_legacy_lookup_is_exact_and_does_not_rewrite_indexes() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    for name in PrivateDir::open(&f.records).unwrap().records().unwrap() {
        if name.starts_with("target-digest-") || name.starts_with("target-dispatch-") {
            std::fs::remove_file(f.records.join(name)).unwrap();
        }
    }
    let before = PrivateDir::open(&f.records).unwrap().records().unwrap();
    f.preserve_original();
    f.state.lock().unwrap().main = "c".repeat(40);
    assert_eq!(
        f.observer
            .recover_historical_target(&c, &proof.historical_reference().unwrap())
            .await
            .unwrap()
            .digest()
            .unwrap(),
        proof.digest().unwrap()
    );
    assert_eq!(
        PrivateDir::open(&f.records).unwrap().records().unwrap(),
        before
    );
    assert!(f.observer.recover_target(&c, "deploy").await.is_err());
}

#[tokio::test]
async fn deployment_free_reads_actual_main_tree_and_rejects_moving_baseline() {
    let f = Fixture::new();
    let c = contract(false);
    let baseline = f.observer.observe(&c).await.unwrap();
    assert_eq!(baseline.baseline().production, source());
    assert!(baseline.targets().is_empty());
    assert!(baseline.baseline().target_artifacts.is_empty());
    f.state.lock().unwrap().main = "c".repeat(40);
    assert!(f.observer.reobserve(&c, &baseline).await.is_err());
    let f = Fixture::new();
    f.state.lock().unwrap().move_main = true;
    assert!(f.observer.observe(&c).await.is_err());
}
#[tokio::test]
async fn historical_unattested_target_is_explicit_preparation_blocker() {
    let f = Fixture::new();
    let e = f.observer.observe(&contract(true)).await.unwrap_err();
    assert_eq!(e.code, "production_baseline");
    assert!(e.next_action.unwrap().contains("trusted"));
    assert!(
        f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .all(|(_, b)| b.is_null())
    );
}
#[tokio::test]
async fn actual_artifact_oidc_and_owned_receipt_produce_complete_recoverable_baseline() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    let repeated = f
        .observer
        .prepare_target(&c, &a, &p, "deploy", source())
        .await
        .unwrap();
    assert_eq!(repeated.dispatch_request(), ch.dispatch_request());
    assert_eq!(repeated.challenge(), ch.challenge());
    f.artifact(&c, &ch, |_| {});
    let proof = f
        .observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    assert_eq!(proof.deployment_id(), 91);
    let baseline = f.observer.observe(&c).await.unwrap();
    assert_eq!(baseline.targets().len(), 1);
    assert_eq!(
        baseline.baseline().target_artifacts["service-production"],
        bytes_digest(b"actual target artifact bytes")
    );
    assert_eq!(
        baseline.baseline().deployed_artifact.as_ref(),
        Some(&bytes_digest(b"actual target artifact bytes"))
    );
    f.observer.reobserve(&c, &baseline).await.unwrap();
    let existing = f
        .observer
        .verify_existing_target(
            &c,
            "service-production",
            &source(),
            &bytes_digest(b"actual target artifact bytes"),
        )
        .await
        .unwrap();
    assert_eq!(existing.deployment_id(), proof.deployment_id());
    assert_eq!(existing.digest().unwrap(), proof.digest().unwrap());
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "service-production",
                &source(),
                &bytes_digest(b"wrong artifact")
            )
            .await
            .is_err()
    );
    let mut stale_source = source();
    stale_source.commit = "c".repeat(40);
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "service-production",
                &stale_source,
                &bytes_digest(b"actual target artifact bytes")
            )
            .await
            .is_err()
    );
    assert!(
        f.observer
            .verify_existing_target(
                &c,
                "unconfigured",
                &source(),
                &bytes_digest(b"actual target artifact bytes")
            )
            .await
            .is_err()
    );

    assert!(
        std::fs::read_dir(&f.records)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .all(
                |e| !String::from_utf8_lossy(&std::fs::read(e.path()).unwrap())
                    .contains("eyJhbGci")
            )
    );
}
#[tokio::test]
async fn mutated_target_artifact_cannot_pair_with_source_only_or_other_artifact_token() {
    for change in ["artifact", "environment", "source", "dispatch", "job"] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |v| match change {
            "artifact" => v["artifact_digest"] = json!(bytes_digest(b"other artifact")),
            "environment" => v["environment_id"] = json!(89),
            "source" => v["source"]["tree"] = json!("c".repeat(40)),
            "dispatch" => v["dispatch_request"] = json!(id(999)),
            "job" => v["proofs"][0]["check_run_id"] = json!(61),
            _ => unreachable!(),
        });
        assert!(
            f.observer
                .observe_target(&c, &a, &p, &ch, &r, &verifier())
                .await
                .is_err(),
            "{change}"
        );
        assert!(f.observer.recover_target(&c, "deploy").await.is_err());
    }
}
#[tokio::test]
async fn recovery_rechecks_full_unfiltered_target_status_and_run_attempt() {
    for change in [
        "pending",
        "failure",
        "deployment",
        "attempt",
        "job",
        "environment",
        "ambiguous",
        "last_page",
        "main",
    ] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        f.observer
            .observe_target(&c, &a, &p, &ch, &r, &verifier())
            .await
            .unwrap();
        {
            let mut s = f.state.lock().unwrap();
            match change {
                "pending" => s.status = "pending".into(),
                "failure" => s.status = "failure".into(),
                "deployment" => s.deployment_id = 92,
                "attempt" => s.attempt = 2,
                "job" => s.extra_job = true,
                "environment" => s.environment = 89,
                "ambiguous" => s.extra_target = true,
                "last_page" => s.fail_last_page = true,
                "main" => s.main = "c".repeat(40),
                _ => unreachable!(),
            }
        }
        assert!(
            f.observer.recover_target(&c, "deploy").await.is_err(),
            "{change}"
        );
    }
}

#[tokio::test]
async fn every_configured_target_needs_its_own_authenticated_current_proof() {
    let f = Fixture::new();
    let mut c = contract(true);
    if let Some(kegari_v0::config::Deployment::Steps { steps }) = &mut c.configuration.deployment {
        let mut second = steps[0].clone();
        second.id = "deploy-two".into();
        second.target_id = "second-production-target".into();
        second.environment_id = "89".into();
        steps.push(second);
    }
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    {
        // Both environments have valid native success. Only the second lacks
        // an authenticated host proof, which is the precise blocker under test.
        let environment = |id, name: &str| json!({"id":id,"name":name,"url":format!("https://github.invalid/repos/fixture/repo/environments/{name}"),"protection_rules":[],"deployment_branch_policy":null});
        let mut state = f.state.lock().unwrap();
        state.responses.insert("/repos/fixture/repo/environments".into(), json!({"total_count":2,"environments":[environment(88,"production"),environment(89,"production-two")]}));
        state.responses.insert(
            "/repos/fixture/repo/environments/production-two".into(),
            environment(89, "production-two"),
        );
        state.responses.insert("https://github.invalid/repos/fixture/repo/deployments?environment=production-two&per_page=100&page=1".into(), json!([{"id":92,"sha":source().commit,"environment":"production-two","production_environment":true,"transient_environment":false,"creator":{"id":99},"created_at":"2026-01-03T00:00:00Z","updated_at":"2026-01-03T00:00:00Z"}]));
        state.responses.insert("/repos/fixture/repo/deployments/92/statuses".into(), json!([{"id":94,"state":"success","environment":"production-two","creator":{"id":99},"created_at":"2026-01-04T00:00:00Z","updated_at":"2026-01-04T00:00:00Z"}]));
    }
    let error = f.observer.observe(&c).await.unwrap_err();
    assert_eq!(error.code, "production_baseline");
    assert_eq!(
        error.message,
        "current target has no retained authenticated deployment proof"
    );
}

#[tokio::test]
async fn target_proof_requires_actual_receipt_and_private_authenticated_recovery_record() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    let mut wrong = r.clone();
    wrong.request_id = id(999);
    assert!(
        f.observer
            .observe_target(&c, &a, &p, &ch, &wrong, &verifier())
            .await
            .is_err()
    );
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let records = PrivateDir::open(&f.records).unwrap();
    let leaf = records
        .records()
        .unwrap()
        .into_iter()
        .find(|n| n.starts_with("target-proof-"))
        .unwrap();
    let mut record: Value = records.read(&leaf).unwrap();
    record["payload"]["target"]["artifact_digest"] = json!(bytes_digest(b"tampered"));
    records.write(&leaf, &record).unwrap();
    assert_eq!(
        f.observer
            .recover_target(&c, "deploy")
            .await
            .unwrap_err()
            .code,
        "invalid_attestation"
    );
}

#[tokio::test]
async fn archive_member_path_count_and_expansion_are_bounded_before_acceptance() {
    for variant in ["traversal", "multiple", "expanded"] {
        let f = Fixture::new();
        let c = contract(true);
        let (a, p, ch, r) = f.prepare(&c).await;
        f.artifact(&c, &ch, |_| {});
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            if variant == "traversal" {
                "../kegari-deployment.json"
            } else {
                "kegari-deployment.json"
            },
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        zip.write_all(
            if variant == "expanded" {
                vec![b' '; 1_048_577]
            } else {
                b"{}".to_vec()
            }
            .as_slice(),
        )
        .unwrap();
        if variant == "multiple" {
            zip.start_file("extra.json", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"{}").unwrap();
        }
        f.state.lock().unwrap().archive = zip.finish().unwrap().into_inner();
        assert!(
            f.observer
                .observe_target(&c, &a, &p, &ch, &r, &verifier())
                .await
                .is_err(),
            "{variant}"
        );
        assert!(
            !f.records
                .parent()
                .unwrap()
                .join("kegari-deployment.json")
                .exists()
        );
    }
}

#[tokio::test]
async fn deployment_source_matches_candidate_tree_and_is_rechecked_at_dispatch() {
    let f = Fixture::new();
    let c = contract(true);
    let (admission, permit) = f.admission(&c);
    let mut wrong = source();
    wrong.tree = "c".repeat(40);
    assert!(
        f.observer
            .prepare_target(&c, &admission, &permit, "deploy", wrong)
            .await
            .is_err()
    );
    // A provider squash SHA may differ while preserving the exact accepted tree.
    f.state.lock().unwrap().main = "c".repeat(40);
    let squash = Revision {
        commit: "c".repeat(40),
        tree: source().tree,
    };
    let challenge = f
        .observer
        .prepare_target(&c, &admission, &permit, "deploy", squash.clone())
        .await
        .unwrap();
    assert_eq!(challenge.source(), &squash);
    let command = f.observer.dispatch_command(&c, &challenge).unwrap();
    let ProviderMutation::GithubDispatch {
        ref expected_production,
        ..
    } = command
    else {
        panic!()
    };
    assert_eq!(expected_production.as_ref(), Some(&squash));
    f.state.lock().unwrap().main = "d".repeat(40);
    let broker = ProviderBroker {
        linear: None,
        github: Some(f.github.clone()),
    };
    let failure = broker
        .mutate(
            f.host.clone(),
            request(challenge.dispatch_request()),
            Some(permit),
            None,
            command,
            || Ok(()),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, "stale_generation");
    let receipt = f.host.operation(challenge.dispatch_request()).unwrap();
    assert_eq!(
        receipt.observed_result.as_ref().unwrap()["effect"],
        "not_sent"
    );
    assert!(
        !f.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|(url, _)| url.ends_with("/dispatches"))
    );
}

#[tokio::test]
async fn late_failure_status_on_older_deployment_blocks_current_target() {
    let f = Fixture::new();
    let c = contract(true);
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    f.state.lock().unwrap().older_failure = Some("2026-01-02T00:00:00Z");
    f.observer.recover_target(&c, "deploy").await.unwrap();
    // Old deployment T1, successful deployment/status T2, old failed status T3.
    f.state.lock().unwrap().older_failure = Some("2026-01-05T00:00:00Z");
    assert!(f.observer.recover_target(&c, "deploy").await.is_err());
    f.state.lock().unwrap().older_failure = Some("2026-01-04T00:00:00Z");
    assert!(f.observer.recover_target(&c, "deploy").await.is_err());
}

#[tokio::test]
async fn recovery_inventory_retains_verified_main_target_divergence_without_assembly_authority() {
    let f = Fixture::new();
    let c = contract(true);
    assert!(f.observer.observe_recovery(&c).await.is_err());
    let (a, p, ch, r) = f.prepare(&c).await;
    f.artifact(&c, &ch, |_| {});
    f.observer
        .observe_target(&c, &a, &p, &ch, &r, &verifier())
        .await
        .unwrap();
    let coherent = f.observer.observe_recovery(&c).await.unwrap();
    assert!(coherent.coherent());
    assert_eq!(coherent.observations().len(), 2);
    f.state.lock().unwrap().main = "c".repeat(40);
    assert!(f.observer.observe(&c).await.is_err());
    let recovery = f.observer.observe_recovery(&c).await.unwrap();
    assert!(!recovery.coherent());
    assert_eq!(recovery.production().commit, "c".repeat(40));
    assert_eq!(recovery.targets()[0].source, source());
    assert_eq!(recovery.contract_digest(), c.digest().unwrap());
    let observations = recovery.observations();
    assert!(
        observations
            .iter()
            .any(|observation| observation.deployed_commit.as_deref()
                == Some(source().commit.as_str())
                && observation.production_commit == "c".repeat(40))
    );
    f.state.lock().unwrap().status = "failure".into();
    assert!(f.observer.observe_recovery(&c).await.is_err());
}
