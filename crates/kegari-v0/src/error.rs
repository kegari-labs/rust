//! Shared application error presentation policy.
use kegari_types::error::Error;

pub fn exit_code(error: &Error) -> i32 {
    match error.code.as_str() {
        "invalid_input" | "invalid_command" => 2,
        "authentication_required" | "access_denied" | "missing_secret" => 5,
        "stale_result"
        | "stale_config"
        | "stale_control"
        | "request_conflict"
        | "ownership_conflict"
        | "identity_mismatch"
        | "unsupported_codex_version"
        | "incompatible_version"
        | "snapshot_expired" => 6,
        "provider_transport" | "target_unavailable" | "timeout" => 4,
        "operation_uncertain" | "operation_failed" | "partial_release" => 7,
        _ => 3,
    }
}
