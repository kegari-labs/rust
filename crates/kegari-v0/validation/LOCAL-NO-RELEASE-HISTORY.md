# Local no-release history validation

Status: local deterministic validation passed. The complete unfiltered suite
passed 717 tests with zero failures and two explicitly ignored native
qualifications across 39 groups, in 669.858 seconds. All 214 library tests passed
under normal parallelism. This report does not establish native containment,
actual-agent behavior, real-provider integration or human calibration. The Goal
is incomplete.

## Scope and evidence boundaries

This increment implements deployment-free immutable no-release completion,
original outcome/generation/time retention, exact native publication/vector and
metadata ancestry, fresh provider access checks for historical dependency use,
zero-spacing queue history, authenticated identity anchors, persistent inventory
refusal and read-only historical inspection. Deployed-target immutable observation
provenance remains a separate producer obligation.

Fixtures use an actual owned disposable Git checkout, admitted candidate assembly,
signed complete candidate vector, injected native provider protocol, Control
receipts and production completion recovery. Integrated checks, human acceptance
intent and correction classification remain explicitly synthetic upstream facts.
The genuine missing integrated-execution gate is asserted; no joined process or
qualified execution capability is invented. Cached/cold delivery-facade tests
reconstruct RuntimeDelivery with the existing runtime coordinator. A separate
passing fixture creates new ObservationCache, RuntimeCoordinator, ExecutionRegistry
and delivery instances from retained records, with empty outcome and anchor state;
only scalar expected identities are transferred. It proves actual saved-seal
reconstruction and first-seen missing-reference refusal. It shares the registered
ControlHost and does not constitute a full process restart. App display component tests use synthetic rows
and grant no dependency credit.

## Retained checks and findings

All commands use the verified installed Rust/Cargo 1.99.1 via
`RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`.
The local runner captures each command, unchanged before/after inputs, compiler
identity observations and hashed logs under
`/private/tmp/kegari-v0-no-release-history-<label>`; labels are never reused.

| Label | Check | Result |
| --- | --- | --- |
| check-1 | `cargo check --locked -p kegari-v0 --all-targets` | Failed: missing imports, 15.039s |
| check-2 | Same all-target check | Passed with unused getter warning, 24.080s; getters then used by actual inspection construction |
| no-release-3 | `cargo test --locked -p kegari-v0 --lib runtime_delivery::release::no_release::tests -- --nocapture` | Failed: 1 passed, 8 failed, 69.419s wall |
| no-release-4 | Same focused suite | Failed: 6 passed, 3 failed, 80.896s wall |
| no-release-5 | Same focused suite | Failed: 8 passed, 1 failed, 71.806s wall; fixture skipped strict queue restoration |
| no-release-6 | Same focused suite | Passed: 9/9, 67.915s wall |
| inspection-6 | `cargo test --locked -p kegari-v0 --lib inspection::tests -- --nocapture` | Passed: 6/6, 10.256s wall |
| no-release-7 | No-release suite with fresh runtime cache regression | Passed: 10/10, 71.995s wall |
| app-7 | App/reconcile test selection | Command failed before tests: no target named `app_reconcile`, 0.073s; corrected to `reconcile` in app-8 |
| app-8 | `cargo test --locked -p kegari-v0 --test app --test reconcile` | Passed: 39/39 across two groups, 38.601s wall |
| clippy-8 | Strict all-target Clippy | Failed: fixture `.err().expect()` lint, 18.500s; replaced with an explicit match preserving the negative assertion |
| clippy-9 | Strict all-target Clippy | Passed, 17.664s |
| fmt-9 | `cargo fmt --check -p kegari-v0` | Passed, 1.216s |
| inventory-9 | Contract inventory with auxiliary input hashes | Passed: 363 blocks/136 named requirements, 0.054s |
| diff-9 | `git diff --check` | Passed, 0.027s |
| plan-9 | Dispatch-disabled evaluation plan | Passed: 56 slots, dispatch_authorized=false, 31.393s |
| full-9 | `cargo test --locked -p kegari-v0` | Passed: 717 passed, 0 failed, 2 native ignores, 39 groups, 669.858s wall |

The eight focused failures stopped at parent publication selection. The upgraded
fixture emits the real typed ParentResultRecord instead of a simplified generic
JSON payload. It exposed four production generic-JSON decode sites whose exact
re-encoding changed field order and rejected genuine signed typed publications.
The correction uses the producer's schema while retaining exact framing and
signature verification. Superseding-parent checks need the same typed reader.
Earlier assertions were not reached and are not counted as verified.

Run 4 passed typed selection and reached later checks. The selector fixture added
native comments without host receipts; correction tracking correctly retained them
as pending after deletion. Its substitutions now run after the original assertions
and explicitly assert the retained blocker. Seal-tamper code now reads the signed
payload through PrivateDir instead of parsing its outer storage envelope directly.
The lost-reply case exposed a production recovery fast path: locally exited
metadata admissions were marked Reconciled while retaining unresolved publication
IDs, and generic reconciliation returned early without the exact readback needed
to clear them. The bounded correction routes only typed no-release metadata with
that retained uncertainty through the existing authorized sole-effect readback;
immutable history continues to require actual closed ancestry.

Independent pre-integration reviews also found and corrected stale ephemeral
fallback after a known seal failed, inventory refusal lost during unrelated queue
rebuilds, loss of first-seen authenticated identity with missing refs, missing
inspection fallback, and current-authority rechecking after awaited history reads.
Known sealed failures now stop direct hydration before queue/map/snapshot writes.
Native provider reads happen before the final serialized current gate; missing
required immutable references deny completion and authority.

## Final evidence and remaining qualification

Focused no-release producer, queue, missing-reference, selector and recovery
checks plus new runtime cache recovery passed at 584 inputs:
`sha256:f2b2dd894b96a1e2bbe03063c84c746e24eecab07d00d289e0e4eed91ef69eff`.
After the fixture lint correction, the full unfiltered suite, strict all-target
Clippy, formatting, contract inventory and dispatch-disabled evaluation plan
passed at the same unchanged 584 inputs:
`sha256:ea4cfbf47c45bf7f4573818f36f1f6d3ad63d60c066d93d0f7f9b3d45ea011cc`.
The [machine evidence](no-release-history-checks.json) verifies actual retained
log hashes, complete test-group summaries, before/after source snapshots and
compiler identities. The contract checker inputs are hashed separately. Failed
invocations remain retained and are not final-check evidence.

The origin boundary retains exact observed publication effects and the complete
candidate vector. SPEC §8.6 keeps approval-command completion and later Linear
closure separate, so closure success is not an added historical acceptance
requirement. Independent reviews covered current-authority rechecking, immutable
ancestry, cache/queue refusal, producer-schema decoding, lost-reply recovery and
typed inspection. No further material issue remained within that scope.

Owned deployed-target and external-baseline history remain separate producer
obligations. Full process restart, native descendant containment, actual-agent
evaluations, real-provider integration and required human review/calibration
remain unqualified. No live Kegari agent evaluations, remote provider writes,
remote mutations, deployment, installation or host security changes are authorized
by these local fixture results.
