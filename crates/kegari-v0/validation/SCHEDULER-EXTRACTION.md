# Scheduler extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `24290686a96f0c72c76c36e585b4a9ba1aa16f89` on `main`.
All fifteen packages retain `0.2.0-alpha.10`; this pass adds only `kegari-scheduler`.

## Ownership and compatibility

`kegari_scheduler` exposes the existing synchronous scheduler, capacity/work/quota
models, attempt-accounting helpers and retry function directly at its crate root.
The complete 450-line production implementation is byte-identical to the original
backend module. Signatures, field visibility, serialization and error behavior are
unchanged. Callers import the owning crate directly; no compatibility reexport or
reverse dependency is introduced. `PhaseKey` remains in `kegari-types`.

Configuration-derived limits, durable admission callbacks, permits, receipts,
recovery and process ownership remain backend-owned. The runtime driver's lock
and scheduler replacement/restoration sequence is unchanged. The existing
`Attempts` helper gains no production callers; the separate private delivery-release
attempt record and receipt-based attempt accounting remain untouched.

Production dependencies are `kegari-types` and pinned serde/chrono/UUID data and
serialization support. Clock access, UUID generation and the existing digest fixture
helper are test-only features/dependencies. The crate has no backend dependency,
including in tests, and no configuration, async runtime, storage, process or network
dependency. CLI and controller need no direct scheduler dependency. The backend
retains the single application identity and now hashes/tracks the new manifest and
source tree. All 239 external lock entries remain unchanged after the offline update.

Before relocation, the original backend generated fixed exact serialized bytes for
all trigger/work-kind/quota variants and representative ready work, allocations and
attempt records, including optional/null values. The portable corpus is retained in
`kegari-scheduler/tests/fixtures/serialization.json`. Compatibility tests verify those
bytes and round trips, missing/null handling and unchanged unknown-field policies.
Only the scheduler's `AttemptRecord` refuses unknown top-level fields; nested
`PhaseKey` retains its existing acceptance.

Capture evidence is `/private/tmp/kegari-scheduler-baseline.json`,
`/private/tmp/kegari-scheduler-baseline.log` and the retained original harness
`/private/tmp/kegari-scheduler-baseline-harness.rs`. The temporary backend capture
test was removed after use. Its first compile found an array-method expression
that needed parentheses inside `json!`; that capture-only syntax error was fixed
before the successful run. The original diagnostic is
`/private/tmp/kegari-scheduler-baseline-initial-compile.log`. No production change
or assertion weakening was needed.

## Verification

All ten original tests retain their bodies/assertions: three scheduler/attempt
cases move from the mixed authority/scheduler harness, its six authority cases stay
backend-owned, and the pure reload/fairness test moves from the runtime driver.
The new crate adds three serialization compatibility tests and eight deterministic
boundary tests. These cover priority/capacity, quota handling, gated callbacks,
failed/duplicate admissions, pending refresh, attempt bounds and retry overflow/
provider precedence. Actual admission, configuration and receipt tests remain in
the backend.

The workspace all-target integration compile passed on its first run. Its log is
`/private/tmp/kegari-scheduler-check.log`. Verification uses Rust/Cargo 1.99.1 through
the verified `+stable` alias, `CARGO_HOME=/private/tmp/kegari-v0-cargo-cache` and
`--offline --locked` after the authorized offline lock update. Normal concurrency
and exactly three native qualification ignores remain.

All **76 focused release tests passed**, with zero failures: 15 scheduler tests,
five runtime-driver tests, the recovery-planning test and 55 backend authority,
runtime-schedule and worker tests. The first recovery filter used the file-family
name rather than the actual private module name and selected no tests. It was
corrected to `runtime::session_recovery::tests`; the intended case passed. Both
outputs remain recorded. Focused commands/results and counts are indexed at
`/private/tmp/kegari-scheduler-focused-results.json` and
`/private/tmp/kegari-scheduler-focused-counts.json`.

One bounded independent review found **no material issue**. It independently
confirmed exact production bytes, all ten preserved test bodies/assertions,
namespace-only caller migrations, retained backend authority, dependency features,
serialization fixtures and build tracking. Review metadata is
`/private/tmp/kegari-scheduler-review.json`; the reviewed candidate covers 817
source/test/build inputs in `/private/tmp/kegari-scheduler-source.json`.

Final workspace verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks, 140 named requirements and 11 passing tests.
- Complete workspace release suite: **1,333 passed, zero failed, exactly three
  native ignores**, across 89 result groups with normal concurrency.

The complete suite ran once on the final reviewed candidate. No test failed and
no additional broad run was needed. Final commands/results are indexed at
`/private/tmp/kegari-scheduler-final-candidate-results.json`; counts and exact ignore
names are in `/private/tmp/kegari-scheduler-test-results.json`.

## Executable and source identity

The optimized `aarch64-apple-darwin` application build from `kegari-cli` passed.
`target/release/kegari-v0` is an arm64 Mach-O executable. All five minimal-environment
inspections (`--help`, `--version`, `setup --help`, `__build-info`,
`__handover-info`) passed with consistent identity, unchanged schemas/assets and
no runtime-state creation. Handover remains schema 5, configuration/API schema 1
and instruction assets 7. The pinned Codex version is unchanged.

Independent reconstruction matched the application hash over 391 inputs,
including the scheduler manifest and source tree; Cargo rebuild tracking covers
both. All 817 reviewed source/test/build inputs match the inspected candidate.
The extraction legitimately changes the single backend-owned application build
identity without changing the workspace version or introducing a competing ID.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build snapshot | `268e46e5543cbe5139589ca99f96b7c8265bbc04d4a117e412510238aaf56641` |
| Application build | `49fbe0916b39acbfdce29521c9e00da25751851cc63b64af6c0e3b5794225847` |
| Optimized executable | `b99922dde981092331cfc23e6790f51a4cedcbda30f3cb721a60f69f647e1b4e` |

Inspection evidence is `/private/tmp/kegari-scheduler-inspection.json`.
Dependency/source-boundary checks are `/private/tmp/kegari-scheduler-boundary.json`.
The unchanged alpha.10 archive retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

## Retained limitations and stopping point

This extraction introduces no scheduling policy, authority/runtime/isolation
extraction, new use of the attempt-accounting helper or qualification claim.
Native/live restrictions, schemas, instruction/protocol assets, milestone
requirements and the alpha.10 archive remain intact. Installed Codex, live
accounts and real Keychain items were not accessed. No native qualification,
service activation, goal resumption, release packaging, installation, commit,
tag or remote operation was performed. This pass stops with the reviewed
working-tree diff and verification summary.
