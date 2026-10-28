# Control API and controller extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `abfb49d5cfa9378e0af944a1b1828228c8abb33d` on `main`.
All twelve packages remain at `0.2.0-alpha.10`. No release is packaged.

## Ownership and compatibility

`kegari-control-api` owns `api`, `identity`, `transport` and `snapshot`.
`kegari-controller` owns `app`, `host`, `supervisor` and `orchestration`.
Consumers import moved interfaces directly; original backend definitions and
modules are removed without compatibility reexports. Shared serialized command,
response, error and pagination data remain in `kegari-types`.

Backend `runtime_host` collects the unchanged shared runtime interfaces and
provides the evaluation-aware configuration-input factory. `control_command`
collects existing command identity and current-operator helpers. Durable control,
registration, configuration administration, execution/delivery and the native
runtime factory remain backend-owned. Only the approved read-only inspection
interfaces and deterministic trial-identity helper gain public visibility;
process-ownership and verified-evidence constructors remain private.

Controller depends on backend and control API. Backend depends on control API;
neither backend nor control API depends on controller, including in tests.
Control API has no backend dependency. All 239 external lockfile entries remain
unchanged. Existing application identity, persisted/API schemas and instruction
assets retain their semantics and versions.

The control host still executes the runtime driver. Its child still holds the
runtime lock and exchanges bounded lease/heartbeat/poll frames. Authorization,
credential/error ordering, uncertain mutation ownership, recovery/admission,
shutdown, stale-socket checks and the shared spawn interlock are preserved.

## Tests and independent review

The existing API, transport and snapshot harnesses moved to control API;
dispatcher, supervisor and orchestration harnesses moved to controller. The two
private host tests moved with their implementation. The stale control-socket
recovery case moved separately with minimal disposable fixtures and its macOS
guard; remaining platform tests stay backend-owned. Assertions are retained.

Four added compatibility tests cover exact identity serialization and unknown
fields, role errors, fixed trial identity and API/backend digest-validation
parity. The trial vector uses an independent sorted-JSON/SHA-256 calculation.
The private syntax helper retains the original accepted formats and errors.

One bounded independent review found no material issues. It verified dependency
direction, identity/authentication and authorization ordering, moved bodies,
shared contracts, visibility, test preservation and build hashing. All 71 other
changed Rust caller files contain only import/path/format migrations. Review
scope and source identity are recorded at
`/private/tmp/kegari-control-extraction-review.json`.

Initial integration checks exposed an import preceding a module doc comment and
one test relying on an implicitly inherited `Principal`. Both were corrected
with import-only changes. Original failed outputs are preserved at
`/private/tmp/kegari-control-extraction-check.log` and
`/private/tmp/kegari-control-extraction-check-2.log`.
The third all-target workspace check passed without warnings; its output is at
`/private/tmp/kegari-control-extraction-check-3.log`. No test failed.

All **173 focused release checks passed**: 58 API/controller, 71 backend
control/configuration/platform/process checks and 44 CLI checks. Commands and
counts are retained at
`/private/tmp/kegari-control-extraction-focused-results.json` and
`/private/tmp/kegari-control-extraction-focused-counts.json`.

Final verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks and 140 named requirements; its 11 tests pass.
- Complete workspace release suite: **1,290 passed, zero failed,
  exactly three native ignores**, across 74 result groups.
- Optimized application build from `kegari-cli`.

Checks use verified Rust/Cargo 1.99.1 through `+stable`,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` after the
offline lockfile update. Normal concurrency is preserved. The complete suite
ran once on the final reviewed candidate; no further broad run was needed.
Commands/results and full output logs are indexed at
`/private/tmp/kegari-control-extraction-final-results.json`; suite totals are at
`/private/tmp/kegari-control-extraction-test-results.json`.

## Executable and source identity

Built `target/release/kegari-v0`, an optimized arm64 Mach-O executable.
Minimal-environment `--help`, `--version`, `setup --help`, `__build-info` and
`__handover-info` all succeeded with consistent metadata and no runtime state.
Schemas/assets are unchanged, including instruction asset version 7.

Independent reconstruction matched the application hash over
376 build inputs, including both new manifests/source
trees and Cargo rebuild tracking. All 789 reviewed
source/test/build inputs match the inspected candidate.

| Identity | SHA-256 |
|---|---|
| Reviewed source/test/build snapshot | `9a6b519571a6a8ef35f5561e08b6ea2ea06b60a10a3c019bda10a57d8572063d` |
| Application build | `1cb44dd2bd33d23081f24d82f3e9454f5a88156c474946c8092dffbdce6a4458` |
| Optimized executable | `bcbdf9fd7838392f6e760e6ec2de1b011b29083890b452e45ad8bb7f95f020ec` |

Evidence is retained at `/private/tmp/kegari-control-extraction-source.json` and
`/private/tmp/kegari-control-extraction-inspection.json`.
The archived alpha.10 retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

## Retained limitations and stopping point

This is the initial controller extraction. Durable admission, registration,
configuration administration and the execution/delivery engine remain backend
responsibilities; the temporary CLI/controller-to-backend dependencies remain.
Crate separation does not relocate work between OS processes or qualify native
execution, live providers or untrusted-context handling.

Tests use mocks, synthetic inputs and disposable fixtures. No real Keychain
items, installed Tailscale, live providers, service activation or native
qualification were invoked. The two platform and one isolation native ignores
remain unchanged. Goal mode remains cancelled. No new release, commit, tag,
installation or remote operation was performed.
