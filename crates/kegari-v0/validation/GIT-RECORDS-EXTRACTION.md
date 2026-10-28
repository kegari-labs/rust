# Git records extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `b07f93798fb0e1f55684af258d37c206295e6165` on `main`.
All sixteen packages retain `0.2.0-alpha.10`; this pass adds only `kegari-git`.

## Ownership and compatibility

The new crate owns fourteen Git data types, the pinned Git/schema constants,
Unicode title conversion, repository/OID/path validation and existing digest and
replacement-contribution calculations. Production code is relocated unchanged
except for the approved public visibility of `validate_repo_path`,
`CommitIdentity::validate` and `SourceCommitMetadata::validate`.

Backend `git` retains executable verification, subprocesses, limits, gates,
cancellation, journals, actual Git-object inspection and clone/candidate/recovery
operations. Its private construction of observed and owned products remains
unchanged. Operation request structs stay beside their backend APIs. Consumers
import moved data directly from `kegari_git`; there are no compatibility reexports,
new unchecked constructors or new process callbacks. This is a first pass, not a
completed Git adapter extraction or an execution qualification.

Production dependencies are only `kegari-types`, `kegari-digest` and pinned
serde/chrono/UUID data support. Chrono and UUID use `std`/`serde` without defaults;
tests add only serde_json and fixed inputs. There is no backend dependency,
including in tests, or process, platform, storage, provider, configuration,
networking or asynchronous-runtime dependency. All 239 external lock entries
remain unchanged. Backend build hashing and rebuild tracking include the new
manifest and source tree, retaining the existing hash algorithm and one identity.

Before relocation, a temporary backend test captured eighteen representative
record variants, exact serialized bytes, canonical digests and method results
from the original implementation. It passed on its first attempt. The corpus is
retained in `kegari-git/tests/fixtures/serialization.json` and matches
`/private/tmp/kegari-git-baseline.json` byte for byte. The original harness and log
are `/private/tmp/kegari-git-baseline-harness.rs` and
`/private/tmp/kegari-git-baseline.log`; the temporary backend test was removed.

All eleven new-crate release tests passed on their first attempt. They cover all
thirteen serialized records, missing/null/unknown fields, exact original bytes
and digests, no implicit validation, pure bounds/error ordering, Unicode/path
rules, human signoff, replacement accounting and review-versus-final identities.
`CommitIdentity` remains permissive about unknown fields, including when nested
inside strict source metadata. `DeltaArtifact` remains non-serializable.
Existing backend test bodies and assertions remain intact. The focused crate log
is `/private/tmp/kegari-git-crate-tests.log`.

## Review and verification

One bounded independent review found no material issue. It confirmed exact moved
implementations apart from the three approved visibility changes, unchanged
retained backend logic/private construction, namespace-only migrations in the
other Rust files, preserved assertions, dependencies and build tracking.
Review metadata is `/private/tmp/kegari-git-review.json`; the reviewed source,
test and build snapshot is `/private/tmp/kegari-git-source.json`.

Verification uses Rust/Cargo 1.99.1 through the verified `+stable` alias,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache` and `--offline --locked` after the
offline workspace lock update. Normal concurrency and exactly three native
qualification ignores were preserved.

Initial setup diagnostics are retained. The first formatter attempt found imports
inserted before module documentation in two fixtures; the imports were moved
below the unchanged documentation (`kegari-git-initial-format-diagnostic.txt`).
An initial locked compile refused because `metadata --no-deps` had not updated
the workspace lock (`kegari-git-check-initial.log`). The subsequent offline
all-target compile updated only workspace entries and passed; its two unused
import warnings were removed (`kegari-git-check.log`). These diagnostics are in
`/private/tmp/`; no test assertion or production behavior was weakened.

All **215 focused release tests passed**, with zero failures or ignores: eleven
new-crate cases, 22 retained Git cases, twenty remote-object cases, 39 recovery
cases and 123 backend integration cases across Git/delivery, execution,
housekeeping and configuration. Actual disposable Git, partial-staging,
process/journal and recovery assertions remain in their existing harnesses.
Commands/results are indexed in `/private/tmp/kegari-git-focused-results.json`
and `/private/tmp/kegari-git-focused-counts.json`.

Final workspace verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks, 140 named requirements and eleven passing tests.
- Complete workspace release suite: **1,344 passed, zero failed, exactly three
  native ignores**, across 93 result groups with normal concurrency.

The complete suite ran once on the reviewed candidate. No test failed and no
source changes or broad verification reruns were needed after review. Final
commands/results are indexed at `/private/tmp/kegari-git-final-candidate-results.json`;
counts and exact ignore names are in `/private/tmp/kegari-git-test-results.json`.

## Executable and stopping point

The optimized `aarch64-apple-darwin` build from `kegari-cli` passed.
`target/release/kegari-v0` is an arm64 Mach-O executable. All five
minimal-environment inspections (`--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`) passed with consistent identity, unchanged
schemas/assets and no runtime-state creation. Handover remains schema 5,
configuration/API schema 1 and instruction assets 7. The pinned Git and Codex
versions are unchanged.

Independent reconstruction matched the application hash over 393 inputs,
including the new manifest/source tree, and verified Cargo rebuild tracking.
All 823 reviewed source/test/build inputs match the inspected candidate. The
refactor legitimately changes the application build identity without changing
the workspace version or introducing a competing identity.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build snapshot | `9f6c65b13afe705d6425fd299f0470c4fa7aa1eade72d13755dff284f753f006` |
| Application build | `f447c69f26ea0220cb6f2e6b8f8a305456702ee495b63fae25d650329da1b0dd` |
| Optimized executable | `f40214bf0638ab3449adde337e6db320ad3aae4b47502331def72f28184b8fd5` |

Inspection evidence is `/private/tmp/kegari-git-inspection.json`.
Dependency/corpus checks are in `/private/tmp/kegari-git-boundary.json`.
The archived alpha.10 executable package retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

The alpha.10 archive is preserved. No live providers, installed Codex, real
Keychain items, native qualification, service activation, goal continuation,
release packaging, commits or remote operations were involved. Changes are
uncommitted and ready for review. Further Git adapter work requires a separate
plan; operational qualification and milestone requirements remain open.
