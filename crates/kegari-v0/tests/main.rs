//! Offline checks of the installed entry grammar; no service/model is started.
use serde_json::Value;
use std::process::{Command, Output};

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kegari-v0"))
        .args(arguments)
        .env_clear()
        .output()
        .unwrap()
}
#[test]
fn public_help_and_version_work_without_runtime_account_or_provider() {
    for arguments in [
        vec!["--help"],
        vec!["--version"],
        vec!["release", "--help"],
        vec!["config", "--help"],
    ] {
        let output = run(&arguments);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.is_empty());
    }
    let output = run(&["--json", "--help"]);
    assert!(output.status.success());
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["ok"], true);
}
#[test]
fn exact_identity_entrypoints_report_the_bound_build_and_assets() {
    let output = run(&["__build-info"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::to_value(kegari_v0::platform::build_info()).unwrap()
    );
    let output = run(&["__handover-info"]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(actual["schema_version"], 3);
    assert_eq!(actual["process_schema"], 2);
    assert_eq!(actual["managed_execution_schema"], 1);
    assert_eq!(
        actual,
        serde_json::to_value(kegari_v0::housekeeping::handover_info().unwrap()).unwrap()
    );
}
#[test]
fn malformed_hidden_entrypoints_and_secret_bearing_errors_never_activate() {
    let sensitive = "private-marker-never-echo";
    for hidden in [
        "__build-info",
        "__handover-info",
        "__orchestrate",
        "__control-host",
    ] {
        let output = run(&["--json", hidden, sensitive]);
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains(sensitive));
        let result: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "invalid_command");
    }
}
