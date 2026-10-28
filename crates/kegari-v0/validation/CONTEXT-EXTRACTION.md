# Context extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `002a7c9b318db14c10b3ca6d28997eb76900c268` on `main`.
All fourteen packages retain `0.2.0-alpha.10`; this pass adds only `kegari-context`.

## Ownership and compatibility

`kegari-context::{workflow, instructions}` owns workflow documents, bounded
rendering and instruction assembly, repository guidance, embedded assets and
instruction-resume compatibility. Parser state and assembly/resume seals remain
private. There is no unchecked constructor or deserialization for assembled
instructions or verified resume instructions. The crate depends on config,
digest and shared types, with no backend dependency, including in tests, and no
async runtime, process, platform, storage, provider or Codex dependency.

`AssemblyInput::runtime_build` supplies the backend's existing application
identity. The production worker-preparation path passes `crate::BUILD_ID`, and
the assembler copies it at the original manifest-construction point. Version
metadata still uses the inherited workspace package version. The new crate has
no build script or competing application identity. Assembly validation, secret
rejection, encoded-artifact checks and filesystem resolution retain their order.

`ArtifactProof` moves unchanged to `kegari_types::artifact`, preserving its five
fields, field order, derives, unknown-field refusal and optional/null behavior.
Backend `execution::artifact_reference` replaces its old method at both actual
call sites. Protected artifact capture, reads, archive verification and evidence
acceptance stay backend-owned. The private context digest-syntax check preserves
the backend validator's accepted formats and exact error; both use a shared
bounded compatibility corpus. Canonical hashing continues using `kegari-digest`.
Configuration workflow loading and required-check cross-validation remain in
backend `config`; execution, services and admission authority remain unchanged.

The original implementation captured six assemblies before relocation, retaining
exact prompts, serialized manifests, manifest digests and resume results. The
extracted implementation matched the entire corpus with the same build identity
and canonical fixture paths. Evidence:

- Original capture/harness: `/private/tmp/kegari-context-baseline.log`,
  `/private/tmp/kegari-context-baseline-harness.rs`.
- Before/after bytes: `/private/tmp/kegari-context-baseline.json` and
  `/private/tmp/kegari-context-after.json`.
- Comparison: `/private/tmp/kegari-context-comparison.log` and
  `/private/tmp/kegari-context-comparison-harness.rs`.
- Retained disposable fixture: `/private/tmp/kegari-context-compat-tnk05m3v`.

Temporary capture harnesses were removed from the working tree after comparison.
Portable tests cover artifact serialization and supplied build identity, including
sealed resume compatibility. The real refactored application build hash changes;
manifests using that new identity legitimately change with it.

All ten relocated instruction/starter files are byte-identical to the baseline.
Asset version `7`, catalog hashes, starter create-new behavior, default selection,
explicit-workflow refusal, schemas and Codex pin remain unchanged. The backend
retains `examples/evaluation-binding.fragment.toml`. Build hashing tracks the new
manifest, source, instruction and example trees, retains backend examples, and
removes obsolete backend instruction-tree coverage. All 239 external lock entries
are unchanged after the authorized offline update.

## Verification

All 32 original context-owned external tests remain: eight workflow, three resume
and 21 pure workflow/assembly cases from the mixed configuration harness. Its ten
backend integration tests remain there. The protected-archive/public-projection
assembly test remains in a backend unit module with its assertions and private
fixture access intact. New bounded cases cover shared artifact bytes/field
handling, injected build identity/resume compatibility and digest-validator parity.
Existing coverage retains limits, templates, paths/symlinks, role/tool boundaries,
secret exclusion including decoded binary artifacts, provenance and sealed resume.

The first all-target compile identified one missed artifact conversion call in
`evidence_validation`. It now calls the retained backend helper; the second
all-target compile passed. Original logs are `/private/tmp/kegari-context-check.log`
and `/private/tmp/kegari-context-check-2.log`. An auxiliary lock comparison initially
used a parent-prefixed Git path; correcting it to the repository-relative path
confirmed unchanged external entries. No source behavior or test assertion was
altered for that inspection error.

Verification uses Rust/Cargo 1.99.1 via the verified `+stable` alias,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` after the
offline lock update. Normal concurrency and the three native ignores remain.
Focused commands/results: `/private/tmp/kegari-context-focused-results.json`.

All **171 focused release tests passed**, with no failures: 34 context, four
shared artifact, two backend context/archival checks and 131 backend integration
tests. Counts are retained at `/private/tmp/kegari-context-focused-counts.json`.

One bounded independent review found **no material issue**. It confirmed dependency
direction, private seals, exact data/conversion preservation, identity injection,
validation order, asset relocation, build tracking and retained test assertions.
Review metadata is `/private/tmp/kegari-context-review.json`; the reviewed candidate
covers 812 source/test/build inputs in `/private/tmp/kegari-context-source.json`.

Final workspace verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks, 140 named requirements and 11 passing tests.
- Complete workspace release suite: **1,322 passed, zero failed, exactly three
  native ignores**, across 84 result groups with normal concurrency.

The complete suite ran once on the final reviewed candidate. No test failed and
no additional broad run was needed. Final commands/results are indexed at
`/private/tmp/kegari-context-final-candidate-results.json`; counts and exact ignore
names are in `/private/tmp/kegari-context-test-results.json`.

## Executable and source identity

The optimized `aarch64-apple-darwin` application build from `kegari-cli` passed.
`target/release/kegari-v0` is an arm64 Mach-O executable. All five minimal-environment
inspections (`--help`, `--version`, `setup --help`, `__build-info`,
`__handover-info`) passed with consistent identity, unchanged schemas/assets and
no runtime-state creation. Handover remains schema 5, configuration/API schema 1
and instruction assets 7.

Independent reconstruction matched the application hash over 390 inputs,
including all 15 context manifest/source/asset/starter inputs and the new shared
artifact type. Cargo rebuild tracking covers the same relocated trees and retains
backend examples. All 812 reviewed source/test/build inputs match the inspected
candidate.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build snapshot | `6f3ab65c7fc5883166f0c66d9576024b5284aa403748847e96359ea251cb247d` |
| Application build | `392ac7ccce4c0407f124eef7e9d87e94e6c964ded6b03dbc02cc6f0e50750510` |
| Optimized executable | `c30f4389021e4968cfcd2fb39f54b2ce42e259183d7f042f94bd6d3bb50b3585` |

Inspection evidence is `/private/tmp/kegari-context-inspection.json`.
Asset/dependency checks are `/private/tmp/kegari-context-boundary.json`.
The unchanged alpha.10 archive retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

## Retained limitations and stopping point

This is a refactor, with no workflow redesign, configuration requirement,
toolset support, command-discovery change, harness expansion or qualification
claim. Native/live restrictions and milestone requirements remain. Installed
Codex, live accounts and real Keychain items were not accessed. No native
qualification, service activation, goal resumption, release packaging,
installation, commit, tag or remote operation was performed. The alpha.10 archive
is preserved; this pass stops with the reviewed working-tree diff and verification
results.
