# Execution environment extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `1b0a15c98ac6ed5468ca29aef498a32e9da6b525` on `main`. All seventeen packages
retain `0.2.0-alpha.10`; all 239 external dependency lock entries are unchanged.

## Ownership and compatibility

`kegari-execution` owns managed environments, filesystem isolation, immutable
resource binding, sandbox rendering and descriptive observations. Native commands,
output budgets, cancellation and joined cleanup now live in backend
`execution_host`; journals, permits and application execution/evidence remain
with their existing backend owners. The old backend `isolation` module is removed
without reexports. This is the bounded environment pass, not complete execution
ownership or an operational qualification.

Production callers supply the current backend `BUILD_ID`. Validated isolation
retains it privately through plan/origin, cwd, bounded inspection and content
revalidation. No build field was added to persisted records. Rendering and
observation remain paired at the original native launch point; the immutable
rendered result grants no launch authority. Codex's borrowing scope adapter
preserves validation order without making execution depend on Codex.

The pre-relocation capture and post-relocation comparison both passed. Using the
same disposable filesystem and original build identity reproduced exact policy,
plan, sandbox profile, command/environment observations, derived-cwd results and
digests. The original corpus remains in
`kegari-execution/tests/fixtures/environment.json`; portable tests verify its
serialized contracts without accessing the captured filesystem. Local baseline,
after-results and both capture harnesses are retained under
`/private/tmp/kegari-execution-*`.

All original tests/assertions remain represented. Environment/domain/resource
cases moved with their implementation; mixed Codex coverage was split without
losing permission assertions. Native command tests and qualification remain
backend-owned. New tests cover build mismatch, repeated/derived validation,
paired rendering and serialized defaults/nulls/unknown fields.

## Review and verification

One bounded independent review found no material issues. It confirmed native
production behavior unchanged after relocation/accessor substitutions, concrete
journal cleanup retained, managed-resource implementation unchanged, all original
assertions retained, one-way dependencies and complete identity propagation.
Review metadata: `/private/tmp/kegari-execution-review.json`.

Verification used Rust/Cargo 1.99.1 through the verified `+stable` alias and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, with `--offline --locked` after
the offline lock update. Normal concurrency and the three native ignores remain.

| Check | Result |
| --- | --- |
| New execution crate | 50 tests passed; separate original-filesystem comparison passed |
| Focused backend command/worker/recovery/integration | 255 passed, zero failed; one existing native ignore |
| Formatting and strict workspace all-target Clippy | Passed |
| Contract inventory | 519 blocks, 140 named requirements; all eleven inventory tests passed |
| Complete workspace release suite | 1,352 passed, zero failed, exactly three native ignores |
| Optimized arm64 executable and five minimal-environment inspections | Passed; consistent metadata and no runtime state created |

The complete suite ran once on the reviewed candidate. No test assertion failed
and no broad rerun was needed. Early setup diagnostics were resolved: the first
split script used an incorrect declaration marker; the first all-target compile
found a duplicate build argument in a migrated test fixture and unused native
imports. No behavior or assertion was weakened. The failed compile is retained
at `/private/tmp/kegari-execution-all-target-check.log`.

Commands/results and counts are indexed in
`/private/tmp/kegari-execution-focused-results.json`,
`/private/tmp/kegari-execution-focused-counts.json`,
`/private/tmp/kegari-execution-final-candidate-results.json` and
`/private/tmp/kegari-execution-test-results.json`. The baseline and new-crate
logs are `/private/tmp/kegari-execution-baseline.log` and
`/private/tmp/kegari-execution-crate-tests.log`.

## Candidate and stopping point

Independent reconstruction verified 396 application-hash inputs,
including the new manifest and source tree, plus Cargo rebuild tracking.
All 829 reviewed source/test/build inputs match the inspected candidate.
The executable remains `target/release/kegari-v0`, arm64 Mach-O. Handover schema 5,
managed execution schema 5, configuration/API schema 1, asset version 7 and pinned
Git/Codex versions are unchanged.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build inputs | `9ab631804d8ed4b14559c49ab8aaabaf208684a4bb70c5235276bed9b36d2ec9` |
| Application build | `9dde19f91be86c53d76418368080a6d8891f4fce036bb0d298a7f925a8d70aba` |
| Optimized executable | `ec3ecd98850643fb32181f87e58c42fddf1f5cb1bbba60c6a8c6e8a6cbc56cf0` |

Inspection evidence: `/private/tmp/kegari-execution-inspection.json`.
The alpha.10 archive remains unchanged at SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.
No live accounts/providers, installed Codex, real Keychain, native qualification,
services, goal continuation, release packaging, commits or remote operations
were used. The diff is uncommitted; further lifecycle extraction needs its own
plan. Existing qualification and milestone requirements remain open.
