# Kegari execution

[SPEC 1.5](../kegari-v0/SPEC.md) assigns repository inspection, editing, command
execution, sandbox enforcement and supported terminal management to Codex
app-server. This crate now retains filesystem scope validation, descriptive
output records and Kegari-owned host process journals. It contains no repository
command runner, sandbox renderer, executable catalog or environment inventory.

## Current source and retained host machinery

The custom command lifecycle, resource fingerprinting, managed execution domain
and sandbox generation have been removed with their architecture-specific tests.
Role, path-grant and owned-directory checks remain for Codex permission selection
and deterministic host Git operations. `IsolationPlan` refuses the removed
`managed_execution` field; old records are preserved without reinterpreting them.
`CommandOutcome` and `ServiceObservation` remain descriptive data, not execution
capabilities. Integrated Codex behavior and qualification are tracked separately
in [PROGRESS.md](../kegari-v0/PROGRESS.md).

The `journal` module also supplies protected process records and registered guards
used by Kegari's own Codex, deterministic Git and control operations. Preserve
mechanisms with those retained callers: actual child ownership, the shared
spawn/retirement interlock, bounded cancellation, joined cleanup and conservative
recovery observations. Persisted identities alone do not grant process ownership.
Kegari retains application admission and interpretation of workflow results.

The migration must not add Kegari admission, resource inventories or journal
records for every Codex-internal command. Supported Codex events provide available
execution observations; unavailable facts and uncertain outcomes stay explicit.
Old process or execution proofs retain their original meaning and cannot silently
become Codex evidence. Change affected persisted identities deliberately and
preserve unsupported old data.

## Development and evidence

Version remains `0.2.0-alpha.10`. This deletion does not establish native or
live qualification. Historical journal, outcome and service record formats remain
unchanged; handover metadata identifies Codex execution records separately.
Follow the [feature-first development policy](../../docs/development.md#feature-first-development-policy):
compile changed code and use the smallest relevant check. Comprehensive testing
and authorized native/provider qualification belong to later stabilization.

The [final-pass extraction report](../kegari-v0/validation/EXECUTION-LIFECYCLE-EXTRACTION.md)
and [environment extraction report](../kegari-v0/validation/EXECUTION-ENVIRONMENT-EXTRACTION.md)
record historical source verification, not the new architecture or its readiness.
