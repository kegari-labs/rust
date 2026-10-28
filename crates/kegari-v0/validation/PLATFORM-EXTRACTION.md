# Platform extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `15c5ad3d118bd6f03f55b3fe913e2d7202af4085` on `main`.
Version remains `0.2.0-alpha.10`. No release is packaged.

## Boundary and compatibility

`kegari-platform` owns OS account/host identity, executable inspection, process
identity/probes, `OwnedChild`, free-space queries and the OS browser-launch helper.
CLI and backend import these directly. Its workspace dependencies are
process/digest/types; it has no direct configuration/storage/backend dependency.
All 239 external lockfile entries are unchanged. Native helpers, executable checks
and child ownership fields remain private, with no new arbitrary-PID probe.

Selected implementation bodies and retained backend policy bodies match the
baseline apart from blank lines. Registration, runtime locks, services, handover
and application build metadata remain in the backend. Account lookup remains
separate from host discovery. CLI URL validation, browser timeout/cancellation
and the shared spawn interlock are unchanged. Serialized identity fields and all
persisted/API schemas and instruction assets are unchanged.

## Review and verification

One bounded independent source review found no material findings in ownership,
private native interfaces, child semantics, serialization, caller/fixture imports,
dependency direction or build hashing.

The initial workspace all-target compile passed with four unused-import warnings
in the reduced backend platform module. Removed those obsolete imports; no
production behaviour changed. Initial output remains at
`/private/tmp/kegari-platform-extraction-check.log`.

Standalone platform checks: five passed, zero failed, two native tests ignored.
The original owned-child cleanup fixture moved unchanged. Four new compatibility
tests cover exact identity serialization/unknown fields, executable digest and
same-size changes, modes, symlinks and invalid paths using disposable files.
The files are not executed and the tests do not query native host/process identity.

The two native tests moved with exact names, bodies, macOS guard and ignore
reasons. The third qualification test remains ignored in backend isolation.
Focused backend platform/process-record/host-process/protocol checks:
57 passed. Executable-binding observation checks: 17 passed.
CLI checks: 44 passed. No focused test failed. Formatting, strict workspace
all-target Clippy, contract inventory (519 blocks, 140 named requirements) and
its 11 Python tests passed.

The final workspace release-profile suite passed: **1,279 passed, zero failed,
exactly three native tests ignored**, across 63 result groups. Normal concurrency
and existing fixture synchronization were preserved. One final broad suite was
run; no failed test was retried and no additional test was disabled.

Verification used Rust/Cargo 1.99.1 through the verified `+stable` toolchain and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`. Cargo checks after the offline
lockfile update used `--offline --locked`. Final commands and logs are indexed in
`/private/tmp/kegari-platform-extraction-final-checks.json`; focused results and
the review are recorded in the corresponding `focused-results.json` and
`review.json` files with the same prefix.

## Executable and source identity

Built `target/release/kegari-v0` from `kegari-cli`: an optimized Mach-O arm64
executable. Minimal-environment inspections of `--help`, `--version`,
`setup --help`, `__build-info` and `__handover-info` all exited successfully,
agreed on application identity and created no runtime state. Persisted/API
schemas, instruction assets and asset version 7 are unchanged.

Independent reconstruction matched the application build hash over 367 inputs,
including the new platform manifest and source. Cargo rebuild tracking includes
both. The 774 source/test/build inputs matched the snapshot used for verification.

| Identity | SHA-256 |
|---|---|
| Source/test/build snapshot | `c5dde540cd179e6b58d77767c592315bca9dd298c1744cd43ef63b3e174e6b6d` |
| Application build | `64815b98f0fa6d0ca6c42e28c0f4d87ed61443e1fe00ec270d9124243673fd99` |
| Optimized executable | `75b25ae3417397df00ae583965793e0968c18abb73b1410e0aee8e9666904a88` |

Detailed inspection evidence is at
`/private/tmp/kegari-platform-extraction-inspection.json`, with source inputs at
`/private/tmp/kegari-platform-extraction-source.json`. The existing alpha.10
archive still has SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.
The executable is a local verification build; no release was packaged.

## Retained limitations

This is an ownership refactor, not additional OS support or operational
qualification. Native/live-operation restrictions, managed-workload limitations
and the CLI's temporary backend dependency remain. Goal mode remains cancelled.
The existing alpha.10 archive is preserved. No installation, service activation,
release packaging, commit or remote operation is included.
