//! Data-only identity checks on the same predicate used after opaque evidence
//! validation. These observations mint no parent, carry or execution capability.
use super::*;

#[test]
fn no_release_carry_keeps_original_acceptance_and_current_result_distinct() {
    let old = bytes_digest(b"original human reviewed result");
    let current = bytes_digest(b"new machine validated result");
    let accepted = bytes_digest(b"original signed human acceptance");
    let carry = CarriedAcceptance {
        schema_version: 1,
        original_reviewed_token: old.clone(),
        original_acceptance_digest: accepted.clone(),
        current_evidence_token: current.clone(),
        unchanged_patch_proof: bytes_digest(b"exact ordered empty selected slices"),
        production_validation_digest: bytes_digest(b"fresh complete production checks"),
        created_at: Utc::now(),
    };
    assert!(no_release_acceptance_matches(&old, &old, &accepted, None));
    assert!(!no_release_acceptance_matches(
        &current, &old, &accepted, None
    ));
    assert!(no_release_acceptance_matches(
        &current,
        &old,
        &accepted,
        Some(&carry)
    ));
    // A direct acceptance is not silently reinterpreted as a carry, and an
    // original reviewed token is never rewritten to the newer recording.
    assert!(!no_release_acceptance_matches(
        &old,
        &old,
        &accepted,
        Some(&carry)
    ));
    assert!(!no_release_acceptance_matches(
        &current,
        &current,
        &accepted,
        Some(&carry)
    ));
    for field in 0..4 {
        let mut changed = carry.clone();
        match field {
            0 => changed.schema_version = 2,
            1 => changed.original_reviewed_token = bytes_digest(b"other human result"),
            2 => changed.original_acceptance_digest = bytes_digest(b"other acceptance"),
            _ => changed.current_evidence_token = bytes_digest(b"later corrected outcome"),
        }
        assert!(!no_release_acceptance_matches(
            &current,
            &old,
            &accepted,
            Some(&changed)
        ));
    }
}
