//! Report classification only. Actual terminal authority remains with the owned
//! worker and runtime; collected rubric evidence is preserved on cleanup failure.
use crate::evaluation::{TrialOutcome, TrialReport};
use kegari_types::error::Error;

pub(super) fn classify(
    report: &mut TrialReport,
    deadline: Option<&Error>,
    terminal: Option<&Error>,
) {
    let Some(primary) = deadline.or(terminal) else {
        return;
    };
    report.omissions.push(format!(
        "Collected outcome before lifecycle classification: {:?}; reason: {}",
        report.outcome, report.reason
    ));
    for (stage, error) in [
        ("native finalization", terminal),
        ("total deadline", deadline),
    ] {
        if let Some(error) = error {
            report
                .omissions
                .push(format!("{stage} failed: {}: {}", error.code, error.message));
        }
    }
    report.outcome = TrialOutcome::Interrupted;
    report.reason = primary.message.clone();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::*;
    use kegari_digest::canonical_digest;
    use serde_json::json;
    use std::path::Path;

    fn collected() -> TrialReport {
        let suite = SuiteManifest::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"),
        )
        .unwrap();
        let trial = suite.plan().unwrap().remove(0);
        let variant = suite
            .variants
            .iter()
            .find(|v| v.id == trial.variant)
            .unwrap();
        let mut report = TrialReport::unrun(
            &suite,
            variant,
            trial,
            TrialOutcome::Blocked,
            "synthetic collected report data only",
        )
        .unwrap();
        // Plain report data never constructs a worker or terminal capability.
        report.evidence_class = EvidenceClass::DeterministicReplay;
        report.outcome = TrialOutcome::DeterministicPassHumanPending;
        report.grade = Some(Grade {
            assertions: vec![AssertionResult {
                id: "observed-fixture-assertion".into(),
                passed: true,
                detail: "synthetic component data".into(),
            }],
            deterministic_pass: true,
            human_review: "pending".into(),
        });
        report.turns_started = 2;
        report.usage_tokens = Some(17);
        report
            .evidence_references
            .push("fixture:retained-observation".into());
        report
    }

    #[test]
    fn finalization_failure_interrupts_without_erasing_collected_evidence() {
        let mut report = collected();
        let before = serde_json::to_value(&report).unwrap();
        let error = Error::new("control_failed", "actual retirement could not be persisted");
        classify(&mut report, None, Some(&error));
        let after = serde_json::to_value(&report).unwrap();
        assert_eq!(report.outcome, TrialOutcome::Interrupted);
        assert_eq!(report.reason, error.message);
        assert!(
            report
                .omissions
                .iter()
                .any(|s| s.contains("DeterministicPassHumanPending"))
        );
        assert!(
            report
                .omissions
                .iter()
                .any(|s| s.contains("native finalization failed: control_failed"))
        );
        for (key, value) in before.as_object().unwrap() {
            if !["outcome", "reason", "omissions"].contains(&key.as_str()) {
                assert_eq!(after.get(key), Some(value), "{key}");
            }
        }
        assert_eq!(after["grade"]["deterministic_pass"], json!(true));
    }

    #[test]
    fn deadline_and_terminal_errors_both_survive_and_success_is_unchanged() {
        let mut report = collected();
        let original = canonical_digest(1, &report).unwrap();
        classify(&mut report, None, None);
        assert_eq!(canonical_digest(1, &report).unwrap(), original);
        let terminal = Error::new("worker_binding", "actual finalization was unavailable");
        let deadline = Error::new("evaluation_deadline", "declared total deadline expired");
        classify(&mut report, Some(&deadline), Some(&terminal));
        assert_eq!(report.outcome, TrialOutcome::Interrupted);
        assert_eq!(report.reason, deadline.message);
        assert!(
            report
                .omissions
                .iter()
                .any(|s| s.contains("native finalization failed: worker_binding"))
        );
        assert!(
            report
                .omissions
                .iter()
                .any(|s| s.contains("total deadline failed: evaluation_deadline"))
        );
        assert!(report.grade.unwrap().deterministic_pass);
    }
}
