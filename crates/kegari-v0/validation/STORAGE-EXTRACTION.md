# Storage extraction verification

Status: complete. Final candidate verified; no unresolved material review findings.

Baseline: `47967ccc52825c4d6baa9205f7f0f52202447843` on `main`.
Version remains `0.2.0-alpha.10`. No release is packaged.

## Boundary and compatibility

`kegari-storage` owns `PrivateDir`, durable record encoding, atomic writes, lock
primitives and retained-file identity. CLI and backend import it directly.
Types/digest are its only workspace dependencies. All 239 external lockfile
entries are unchanged. Configuration/API/persisted schemas and instruction assets
are unchanged. The backend retains domain records, recovery/authority policy and
the single private spawn/retirement interlock; its production spawn code changes
only its storage import.

Existing storage-only atomic-write/path-safety tests, retained-file unit tests and
inherited-lock fixtures moved with their assertions. Mixed control/recovery tests
remain in the backend. Two additional tests cover exact protected record bytes,
decode refusals and descriptor identity/CLOEXEC/last-owner lock lifetime.

## Review and verification

One bounded independent source review found no material issues in dependency
direction, public API opacity, implementation/test preservation, descriptor
ownership, lockfile compatibility or application build hashing.

The initial all-target compile exposed a mechanical import-migration error:
`deployment_wire` lost `pub(crate)` on its remaining backend imports. Original
visibility was restored; no production behaviour or test assertion changed.
The original failing output is retained at
`/private/tmp/kegari-storage-extraction-check.log`.

Focused checks: storage 13 passed; CLI 44 passed; backend library plus
control/storage, evaluation reports and host-process integration 715 passed,
with its one native test ignored. Formatting and strict workspace all-target
Clippy passed. Contract inventory passed (519 blocks, 140 named requirements),
as did its 11 Python tests.

Final workspace release suite: **1,275 passed, zero failed, three native tests
ignored**, across 57 result groups. All existing coverage was retained; the total
increased by the two storage compatibility tests. Tests ran with normal concurrency
and existing fixture-local synchronization. No additional ignores or retries were
introduced. The optimized `aarch64-apple-darwin` executable built successfully.

The five minimal-environment inspections (`--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`) passed. Application/build identities agree;
all schemas and embedded asset records match their existing values. No runtime
state was created. Independent hash reconstruction covers all seven packages,
including storage's manifest and full source tree; Cargo rebuild tracking matches.
The existing alpha.10 archive retains its original SHA-256.

Verification used Rust/Cargo 1.99.1 via the verified stable alias, the existing
`/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` after the initial
offline lockfile update. One final broad verification pass was sufficient.

Evidence:

- Final check commands/results: `/private/tmp/kegari-storage-extraction-final-checks.json`.
- Focused outputs: `/private/tmp/kegari-storage-extraction-focused-{storage,backend,cli}.log`.
- Independent review: `/private/tmp/kegari-storage-extraction-review.json`.
- Source snapshot: `/private/tmp/kegari-storage-extraction-source.json`.
- Executable/asset inspections: `/private/tmp/kegari-storage-extraction-inspection.json`.

Candidate identity:

- Source inputs: 768; digest `sha256:196da081d289c318942ec10aac1de3053271a30c884c0573b3cb7670da48754d`.
- Application build inputs: 363; identity `sha256:b90ebe557a118bc671b5d1131261cc6d60669f324581dfa7771d862999e946da`.
- Development executable: `target/release/kegari-v0`; SHA-256 `6292cd43e0b0a3b13313af9cfde10de3d9ce9a6d37bfb2e3a7addc3739ee4a6b`.

The executable is a local verification build, not a packaged alpha. The contract-inventory tool inventories specification content,
not test locations; relocated test discovery is verified through Cargo's suite.

## Retained limitations

Goal mode remains cancelled. This refactor does not enable execution, qualify
native/live operations, install services or advance operational milestones.
The CLI's temporary backend dependency remains. Low-level lock descriptor methods
are trusted in-process integration interfaces; storage does not itself acquire
the backend spawn interlock or authorize execution. No commit or remote operation
is included.
