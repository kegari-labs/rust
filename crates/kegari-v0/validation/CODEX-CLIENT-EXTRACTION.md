# Final Codex client extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `e1806cd1688097c9f0e5cabb4c7d32be926c5b07` on `main`.
All thirteen packages retain `0.2.0-alpha.10`; this pass adds no package or release.

## Ownership and compatibility

`kegari-codex` owns the protocol client, session/history state, request sequencing,
deadlines, request/tool interfaces, protocol recovery and callback-driven
observations. Private implementation modules expose the existing interfaces
through the crate root. Production workspace dependencies are `kegari-types`
and `kegari-digest`; protected storage and temporary fixtures are test-only.
There is no backend dependency, including in tests.

Backend `codex_host::NativeCodexClient` owns the actual spawned child,
cancellation and diagnostic task. It lends its protocol client through
`client_mut()` and has no unchecked constructor or `Deref`. Native launch,
permission derivation and process journals remain backend-owned. Production
session validation delegates through `SessionScope` to `ValidatedIsolation`,
retaining binding/revalidation/account/catalog/permissions/path check order.
The plain transport constructor supplies startup context without process adoption.

`ResumeCandidate` establishes data consistency only. Backend
`worker_run::RetainedResumeSession` remains the protected-journal wrapper with
private construction and actual receipt-lineage checks. Successful joined
`VerifiedProviderRecovery` retains that original protected wrapper so final
runtime admission can verify receipts at the original point. Verified backend
resume performs session validation once, then its existing proof validation,
then protocol resume with the unchanged second history readback. Evaluation
keeps the native owner until after dropping its in-flight future and reads the
original transport tail after owned termination. Transport-end observations do
not establish native execution authority.

`RunReference` and `PhaseKey` move unchanged to `kegari_types::run`. Backend
`control::run_reference_from_receipt` replaces the former method. Validation
stays private to Control; paired-resume parsing still directly decodes and
validates without recursively calling the generic parser. Unknown-field refusal
for run references, unknown-field acceptance for phases and default/null
serialization are unchanged. All consumer paths migrate directly, with no
compatibility reexports or duplicate implementations.

The original implementation generated fixed fresh, failed, one-prefix,
two-prefix and legacy-prefix session fixtures before production changes. Exact
serialized bytes, state/candidate/recovery digests and real shared run references
are retained in `kegari-codex/tests/fixtures/codex-final-sessions.json`. The
baseline capture passed; its log remains at
`/private/tmp/kegari-codex-final-baseline-capture.log`. The extracted implementation
passes that unchanged corpus. A second bounded corpus independently checks the
private Codex and existing backend digest validators, including exact errors.

All 239 external lockfile entries, protocol assets, supported Codex version,
schemas and instruction assets remain unchanged. Existing build hashing already
tracks the full Codex and shared-types source trees; no hashing algorithm or
competing application identity is introduced.

## Verification

All 28 original mixed Codex harness tests remain: 18 pure protocol/client tests
moved to the adapter and 10 isolation/native fixture tests stay in the backend.
Protocol/history/resume/observation unit coverage moved with its owner. Receipt
lineage assertions stay backend-owned, including all missing-receipt, whole
receipt and typed-link tampering cases from the mixed repeated-resume test.
New bounded checks cover scope ordering, startup/home refusal, shared data
formats, digest parity and native-wrapper drop cleanup using a disposable shell
fixture. No installed Codex or live provider is involved.

Initial compilation caught duplicate/stale imports, native-wrapper call sites,
an internal test visibility edge and the final receipt check that needed the
original protected wrapper. Those were corrected without weakening assertions
or moving receipt policy into the adapter. Original logs are retained as
`/private/tmp/kegari-codex-final-check-lib*.log` and
`/private/tmp/kegari-codex-final-check-all*.log`. The first locked invocation
also recorded that the lockfile needed its authorized offline dependency update.
The subsequent complete all-target compile passed. The first final Clippy pass
then caught a redundant single-argument `concat!` in the new PhaseKey byte
fixture. Removing that wrapper preserved the exact string/assertion; the reviewer
confirmed the sole test-only correction. Its original output remains in
`/private/tmp/kegari-codex-final-final-clippy.log`. No lint suppression was added;
the corrected strict Clippy, format and contract-inventory checks passed. The
complete release suite had not run before this correction.

Verification uses Rust/Cargo 1.99.1 via the verified `+stable` alias,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` after
the offline lock update. Normal concurrency and three native ignores remain.
Focused command results are indexed at
`/private/tmp/kegari-codex-final-focused-results.json`.

All **181 focused release tests passed**, with zero failures: 66 Codex tests,
4 shared run-data tests, the protected multi-generation receipt test, backend
digest parity and 109 backend integration tests. Counts are retained at
`/private/tmp/kegari-codex-final-focused-counts.json`.

One bounded independent review found **no material issue**. Direct comparison of
67 relocated function bodies and the remaining ownership/interface differences
confirmed protocol behavior, authorization order, native cleanup and retained
assertions. Review metadata is `/private/tmp/kegari-codex-final-review.json`.
The final candidate matches all 805 reviewed source/test/build inputs; the
updated snapshot is
`/private/tmp/kegari-codex-final-source.json`.

Final verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks, 140 named requirements and 11 passing tests.
- Complete workspace release suite: **1,315 passed, zero failed, exactly three
  native ignores**, across 80 result groups with normal concurrency.
- Optimized `aarch64-apple-darwin` application build from `kegari-cli`.

The complete release suite ran once on the final reviewed candidate. No test
failed and no additional broad test run was needed. Final commands/results are
indexed at `/private/tmp/kegari-codex-final-candidate-results.json`; test totals
and exact native ignore names are in
`/private/tmp/kegari-codex-final-test-results.json`.

## Executable and source identity

`target/release/kegari-v0` is an optimized arm64 Mach-O executable. All five
minimal-environment inspections (`--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`) passed with consistent version/build identity,
unchanged schemas/assets and no runtime-state creation. Handover remains schema
5, configuration/API schema 1 and instruction assets 7.

Independent reconstruction matched the application hash over 386 inputs,
including all relocated protocol modules, shared run types and pinned assets.
Existing Cargo directory tracking covers those inputs. All 805 reviewed
source/test/build inputs match the inspected candidate.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build snapshot | `0682b46e7463f9f58b246950c30845a23aef1362357f1e5825ce4a91cfd245c8` |
| Application build | `f701daa7b8537ca57c8eaceb67dfc48a2484b32b94673292e54e945cec2b0ce2` |
| Optimized executable | `fcc8b651bad7ecc09ca43f4c4c5a4e489f341b140f1aa913d1488fe465e37bde` |

Inspection evidence is `/private/tmp/kegari-codex-final-inspection.json`.
The unchanged alpha.10 archive retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

## Retained limitations and stopping point

This completes Codex protocol ownership; backend integration and authority remain
explicit. It adds no feature, recovery redesign, Codex upgrade or qualification
claim. Native/live restrictions and existing milestone requirements remain.
Installed Codex, live accounts and real Keychain items were not accessed. No
native qualification, service activation, goal resumption, release packaging,
installation, commit, tag or remote operation was performed. The alpha.10 archive
is preserved; this pass stops with the reviewed working-tree diff.
