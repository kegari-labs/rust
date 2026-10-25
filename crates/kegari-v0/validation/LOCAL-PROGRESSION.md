# Guarded delivery progression checkpoint

The frozen offline checkpoint passed **523 tests, zero failures and two native
qualification tests explicitly ignored**, across 35 completed test groups.
Strict all-target Clippy, formatting, coverage inventory and diff checks passed.
This report does not establish complete application conformance or operational
qualification. [Machine-readable results](progression-checks.json) retain the
per-group counts, source identity and test-log digest.

## Scope

This increment connects protected completed worker claims to child delivery,
exact candidate validation/review, parent candidate and integrated work, and
finite release preparation. Native metadata and worker processes share actual
control ownership, process journals and cancellation fences. Cleanup preserves
the exact retained bytes and rechecks authority before destructive local steps.

Release provider adapters publish isolated candidate objects and derive review
provenance from the approved child sources, verify hosted tests through exact
artifact/OIDC bindings, observe target-specific deployment records, and retain
unknown effects. The new runtime release queue is still being integrated with
operator decisions and recovery; public component methods are not an operational
release command.

The delivered role instructions describe these finite assignments and their
evidence boundaries. The evaluation inspector reads all declared trial slots,
preserves interruption/failure history, and compares declared and actual
conditions. It never dispatches model work or grants human qualification.

## Reproduction

Run from the Rust workspace using the already installed verified Rust 1.99.1
toolchain alias. No installation or host configuration change is required by
these commands:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
```

The tested 497 source, test, instruction, protocol, example, evaluation and Cargo
inputs have combined SHA-256
`60831f1db54e9807d560ae3d9a452a8d9a6d59301f409f7e1bb093f6610f8ac1`.
The evidence record describes its calculation. These inputs were rechecked after
Cargo exited successfully. The built evaluation CLI also returned all 56 slots
from `evaluations/manifest.json` with `dispatch_authorized: false`; no model work
was launched.

The coverage inventory contains 363 normative blocks and 136 named identifiers.
Its completeness does not mean the requirements are satisfied. SPEC 1.2 retains
SHA-256 `80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.

## Review corrections and retained failures

- Deployment reobservation initially rejected an unchanged target because its
  observation timestamp changed. Only that timestamp is normalized; native
  source/tree, artifact, repository, target and provider identity remain exact.
  The failed full run is retained in the progress record; targeted deployment
  and provider checks passed after correction.
- An async child progression error could discard the original completed claim
  and execution handles if reporting also failed. Pipelines now retain those
  capabilities independently of a polling future or report write.
- Child/parent launch intent could advance after a known pre-admission failure,
  then wait forever for a worker that never started. Preparation identity and
  actual admission history now distinguish no-effect waits from unknown work.
- Parent state keyed only by parent identity could block every later corrected
  generation. Retained records now distinguish generations and preserve older
  unresolved work. Candidate preparation and final designated E2E completeness
  are separate gates; final parent publication still needs every required child.
- Existing phase drain must retain its original settings and service handles,
  while rechecking current permission reductions and exact native authority.
  Maintenance may finish admitted work but cannot open a new model/phase merely
  because the runtime is paused or stopped.
- Cleanup could miss same-inode, same-size edits and changed modes between its
  inventory and unlink. Descriptor hashing plus final identity/timestamp checks
  preserve such changed files. Source, retained copies and receipts survive a
  rejected cleanup.
- Report inspection accepted impossible successful agent shapes and incomplete
  grading predicates; comparison omitted actual inputs and permissions.
  Successful reports now need turns, activity, evidence, complete declared
  predicates and ordered durable times. Missing historical evidence stays
  unavailable, with its original declared grader version.
- New-worker authority and quota serialization review found selection exclusion
  and exhaustion races. The final admission must use new-operation eligibility;
  only an already registered operation may use retained drain authority. Quota
  observations now share the serialized admission order. Two new regressions
  pass in the runtime unit suite and the complete frozen run.
- Release review found that receipt-reconciliation failure could free resource
  locks too early, and a parent-only preparation cache could block a later
  accepted token. Pending receipt reconciliation now retains locks and ownership;
  preparations remain distinct by exact accepted evidence token. Four backend
  regressions cover failure/restart/retirement, token replacement, global ordering
  by acceptance time, and newer same-generation native parent evidence.
- Final release admission rechecks the exact parent token, acceptance digest and
  acceptance timestamp selected by the sorted pass. Replacement after preflight
  leaves no admission receipt. Conflicting native acceptance records fail closed.
- A known terminal CI/review failure now retires its finished local phase while
  preserving the error and original operation. Pending or unknown effects retain
  ownership. Terminal failure does not authorize another attempt. Failed setup
  is fenced again before process/journal observations can authorize retirement.
- Failed provider observations now invalidate cached readiness under the same
  control lock as admission and successful hydration. The concurrent regression
  verifies ordering and rejects a subsequent admission without a receipt.

## Evidence limits and remaining work

Deterministic provider fixtures are explicitly simulated. Git tests use real,
owned disposable repositories and exact-byte copies of installed Git in private
fixture paths. No model evaluation, Linear/GitHub mutation, deployment, lasting
service installation or coordinator activation was performed.

The original empty main entry remains until the concrete decision/recovery
backend is complete. Remaining work includes executable composition; correction
steering and continuation binding; exact remote Git object acquisition; full
parent/E2E restart hydration and visual media; release resume/successor decisions;
and concrete live evaluation fixture/authority composition. A compile pass does
not substitute for any of these capabilities.

Native arbitrary repository commands fail before spawn because verified
descendant containment is not yet implemented. The retained escape regression
demonstrates why process-group/EOF completion is insufficient. This sandbox also
denies native boot identity and nested sandbox probes. Required utilities and
trusted production executable paths are missing. Actual-agent, real-provider,
native isolation and human calibration gates remain blocked/unrun; see
[PROGRESS.md](../PROGRESS.md) for the exact dependencies and supervised plans.
