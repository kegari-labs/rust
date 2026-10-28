# Process extraction verification

Status: complete. Final candidate verified; no unresolved material review findings.

Baseline: `ff471893c50c10a67238f53ca59345ad859d9579` on `main`.
Version remains `0.2.0-alpha.10`. No release is packaged.

## Boundary and compatibility

`kegari-process` owns the existing spawn interlock/registry and owned process-group
implementation under `spawn` and `group`. The backend imports these directly;
there are no compatibility reexports. Storage/types are the only workspace
dependencies, and the CLI has no direct dependency. All 239 external lockfile
entries are unchanged. Schemas, instruction assets and managed-workload scope
are unchanged.

The two moved implementations and their tests preserve existing behaviour and
assertions. Only module paths, approved visibility and API documentation change.
There is exactly one spawn/retirement mutex and registry. The backend process
journal's critical section, including descriptor release and existing-lock
observation, is unchanged apart from module paths. Fields, identity constructors
and owned exit construction remain private; exit evidence is not deserializable.

## Review and verification

One bounded independent source review found no material issue in ownership,
dependency direction, interlock/retirement ordering, test retention or build
hashing. Its minor dependency-documentation note was resolved by documenting
`serde_json` in the new crate README.

The initial standalone compile identified an omitted existing dependency:
process-group errors construct three `serde_json` diagnostic payloads. Added the
existing pinned `serde_json = 1.0.151` dependency without changing their details.
The original compile output is retained at
`/private/tmp/kegari-process-extraction-check.log`.

Standalone process tests: 13 passed, zero failed or ignored, including the named
self-executing fixture demonstrating that escaped descendants are not contained.
Focused backend integration passed: process-record/host-process tests
17, Git startup/exit tests 13, and isolation command tests
13. No focused test failed or was ignored. Formatting, strict workspace
all-target Clippy, contract inventory (519 blocks, 140 named requirements) and
its 11 Python tests passed.

Final workspace release suite: **1,275 passed, zero failed, three native tests
ignored**, across 59 result groups. The pre-extraction count is unchanged; all
assertions and the named self-executing fixture are retained. Tests ran with
normal concurrency and existing fixture synchronization. No additional ignores,
retries or changes to native qualification were introduced.

The optimized `aarch64-apple-darwin` executable built successfully. All five
minimal-environment inspections (`--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`) passed with consistent application/build
identity, unchanged schemas and matching embedded asset records. No runtime
state was created. Independent hash reconstruction covers all eight packages,
including the process manifest and source tree, and confirms Cargo rebuild
tracking. The existing alpha.10 archive retains its original SHA-256.

Verification used Rust/Cargo 1.99.1 through the verified stable alias, the existing
`/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` after the initial
offline lockfile update. One final broad verification pass was sufficient.

Evidence:

- Final check commands/results: `/private/tmp/kegari-process-extraction-final-checks.json`.
- Focused commands/results: `/private/tmp/kegari-process-extraction-focused-checks.json` and `/private/tmp/kegari-process-extraction-focused-results.json`.
- Independent review: `/private/tmp/kegari-process-extraction-review.json`.
- Source snapshot: `/private/tmp/kegari-process-extraction-source.json`.
- Executable/asset inspections: `/private/tmp/kegari-process-extraction-inspection.json`.

Candidate identity:

- Source inputs: 770; digest `sha256:f50f50a8b7b39ca1e62320e22da921cd0dc2a3dcecdab76f9fa42e40c8eb39c3`.
- Application build inputs: 365; identity `sha256:b7839f77687cda5a3d8b926cfc25febb007832a8ac73624b41b608a132a44c12`.
- Development executable: `target/release/kegari-v0`; SHA-256 `642145382a3f43552aedec27c585dc16e87d38b9f44009ea2cdf947d94522ed2`.

The executable is a local verification build, not a packaged alpha. Contract
inventory tracks specification content; relocated tests are discovered by Cargo.

## Retained limitations

Process records, control permits, recovery policy, isolation, host registration,
service management and application supervision remain in the backend. Public
process mechanics are trusted integration APIs, not application admission or
sandbox policy. Native/live qualification remains restricted; the three existing
native tests remain ignored. Goal mode remains cancelled. The existing alpha.10
archive is preserved; no installation, service activation, packaging, commit or
remote operation is included in this refactor.
