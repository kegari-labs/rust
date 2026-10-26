//! Real worker/Control/executor composition with injected deterministic command
//! outcomes. No repository command, native service or qualification is issued.
#![allow(dead_code)]
use super::*;

fn validation_receipt(fixture: &Fixture) -> Receipt {
    let mut receipts: Vec<_> = fixture
        .control
        .operations()
        .unwrap()
        .into_iter()
        .filter(|receipt| receipt.kind == "worker_validation")
        .collect();
    assert_eq!(receipts.len(), 1);
    receipts.pop().unwrap()
}

async fn run_bundle(fixture: &mut Fixture) -> Result<Value> {
    let call = fixture.call("validation_run", json!({"name":"suite","kind":"local"}));
    tokio::time::timeout(StdDuration::from_secs(10), fixture.worker.call(&call))
        .await
        .expect("real worker bundle path exceeded component bound")
}

fn assert_known_closed(fixture: &Fixture) {
    assert_eq!(
        validation_receipt(fixture).phase,
        ReceiptPhase::Acknowledged
    );
    assert!(
        fixture
            .control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .is_empty()
    );
    assert!(fixture.executions.active_operations().unwrap().is_empty());
}

#[tokio::test]
async fn worker_bundle_stops_writers_before_actual_artifact_capture_and_acknowledgment() {
    let mut fixture =
        Fixture::with_bundle(worker_bundle::Mode::Success, WorkerContract::new_configured);
    let result = run_bundle(&mut fixture).await.unwrap();
    assert_eq!(result["schema_version"], 3);
    assert_eq!(result["successful"], true);
    assert_eq!(result["bundle"]["services"].as_array().unwrap().len(), 1);
    assert_eq!(
        fixture.boundary.bundle.as_ref().unwrap().events(),
        vec![
            "service:start",
            "readiness:start",
            "readiness:joined",
            "check:start",
            "check:joined",
            "service:stop",
            "service:joined",
        ]
    );
    let actual = std::fs::read(fixture.input.isolation.plan().scratch.join("check.json")).unwrap();
    assert_eq!(actual, b"written-during-service-stop\n");
    assert_eq!(result["artifacts"][0]["digest"], bytes_digest(&actual));
    assert_known_closed(&fixture);
    let receipt = validation_receipt(&fixture);
    assert!(receipt.expected.contains_key("check_bundle"));
    assert_eq!(receipt.observed_result.as_ref(), Some(&result));
    let checks = fixture.worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(
        checks[0].production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    assert!(fixture.worker.take_failed_checks().is_empty());
    assert_eq!(
        fixture.worker.observe().unwrap().checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Passed
    );
}

#[tokio::test]
async fn worker_bundle_readiness_failure_prevents_check_and_repair_capability() {
    let mut fixture = Fixture::with_bundle(
        worker_bundle::Mode::ReadinessFailure,
        WorkerContract::new_configured,
    );
    assert_eq!(
        run_bundle(&mut fixture).await.unwrap_err().code,
        "check_bundle_failed"
    );
    assert_eq!(
        fixture.boundary.bundle.as_ref().unwrap().events(),
        vec![
            "service:start",
            "readiness:start",
            "readiness:joined",
            "service:stop",
            "service:joined",
        ]
    );
    assert_known_closed(&fixture);
    assert!(
        validation_receipt(&fixture)
            .observed_result
            .unwrap()
            .get("check_bundle_failed")
            .is_some()
    );
    assert!(fixture.worker.take_verified_checks().is_empty());
    assert!(fixture.worker.take_failed_checks().is_empty());
}

#[tokio::test]
async fn worker_bundle_unexpected_service_exit_blocks_main_after_readiness() {
    let mut fixture = Fixture::with_bundle(
        worker_bundle::Mode::UnexpectedExit,
        WorkerContract::new_configured,
    );
    assert_eq!(
        run_bundle(&mut fixture).await.unwrap_err().code,
        "service_not_ready"
    );
    let events = fixture.boundary.bundle.as_ref().unwrap().events();
    assert!(!events.contains(&"check:start"));
    assert!(events.contains(&"readiness:joined"));
    assert!(events.contains(&"service:joined"));
    assert_known_closed(&fixture);
    assert!(fixture.worker.take_verified_checks().is_empty());
    assert!(fixture.worker.take_failed_checks().is_empty());
}

#[tokio::test]
async fn worker_bundle_unknown_stop_retains_sent_effect_and_original_registry_owner() {
    let mut fixture = Fixture::with_bundle(
        worker_bundle::Mode::UnknownStop,
        WorkerContract::new_configured,
    );
    let error = run_bundle(&mut fixture).await.unwrap_err();
    assert_eq!(error.code, "fixture_stop_unknown");
    let receipt = validation_receipt(&fixture);
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    assert!(
        fixture
            .control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .contains_key(&receipt.operation_id)
    );
    assert_eq!(
        fixture.executions.active_operations().unwrap(),
        BTreeSet::from([fixture.permit.operation_id])
    );
    assert!(fixture.worker.take_verified_checks().is_empty());
    assert!(fixture.worker.take_failed_checks().is_empty());
    assert!(
        fixture
            .boundary
            .bundle
            .as_ref()
            .unwrap()
            .events()
            .contains(&"service:unknown")
    );
    // The injected task has returned, but its simulated exit remains unknown.
    // Preserve that explicit unresolved evidence; this is not an OS join claim.
    let retained = fixture._temp.keep();
    eprintln!(
        "retained injected bundle uncertainty: {}",
        retained.display()
    );
}

#[tokio::test]
async fn worker_without_services_keeps_original_single_effect_record_shape() {
    let mut fixture = Fixture::new(AgentRole::Implementation);
    let result = run_bundle(&mut fixture).await.unwrap();
    assert_eq!(result["schema_version"], 1);
    assert!(result.get("bundle").is_none());
    assert!(fixture.boundary.bundle.is_none());
    assert!(
        !validation_receipt(&fixture)
            .expected
            .contains_key("check_bundle")
    );
    assert_known_closed(&fixture);
}
