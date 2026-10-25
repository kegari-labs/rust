# Local control and worker integration checkpoint

The complete offline crate test command passed **411 tests**, with **2 native
qualification tests explicitly ignored because the outer sandbox denies the
required boot identity probe**. Strict all-target Clippy passed. This is evidence
for the registered library services and deterministic fixtures; the executable
entry, complete runtime scheduler and delivery progression are unfinished.

## Reproduction

Run from the Rust workspace with the already installed Rust 1.99.1 toolchain:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
```

The existing `stable` alias was verified as the exact pinned version; this does
not authorize toolchain installation. The inventory check reports 363 contract
blocks and 136 named identifiers and does not infer conformance from coverage.
SPEC 1.2 retains SHA-256
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.

## Observable evidence

The run covers private durable control records, admission and pause races,
unknown provider effects, caller cancellation, atomic capacity, exact scoped
worker tools, current account/profile bindings, current native comment evidence,
permission reductions across restart, active settings retention, actual disposable
Git repositories, independent candidate assembly, Unix peer authentication and
disposable-certificate TLS handshakes. Fixtures are bounded and owned locally.

| Suites | Passed |
| --- | ---: |
| Library / API / dispatcher / authority-scheduler | 11 / 10 / 9 / 8 |
| CLI / local CLI / connections / secrets | 24 / 10 / 2 / 2 |
| Codex protocol / configuration-harness / configuration | 27 / 30 / 19 |
| Control-storage / decisions / reconciliation | 21 / 13 / 24 |
| Delivery / Git-delivery / local execution | 20 / 33 / 18 |
| Housekeeping / providers / snapshots / transport | 11 / 23 / 3 / 13 |
| Orchestration / platform / process-group / process-records | 3 / 6 / 4 / 3 |
| Scheduling / supervisor / worker / evaluation graders | 6 / 10 / 28 / 20 |

The two ignored tests are additional platform cases, not included in the pass
count. Evaluation tests exercise production assembly and deterministic scorers;
they are not actual-agent evaluations. Native Keychain, launchctl activation and
real remote mutations were not invoked.

## Review corrections and retained failures

- Provider network waits previously held the serialized control gate. Prepared,
  Sent and completion stages now separate bounded gate work from owned asynchronous
  transport; stalled-wire fixtures verify responsiveness, exact replay and actual
  response handling after caller cancellation.
- Cached actors, permissions, admission settings and completion snapshots could
  preserve stale authority. Current principal checks, monotone persisted fences,
  retained active snapshots and exact-generation gates now cover those paths.
- Acceptance and retry publications could change after the first read. Exact
  native comment identity/body/writer checks, evidence-token checks and a shared
  correction publication fence reject those races.
- Review after the full-suite run found that retry-grant validity depended on
  an inspection refresh. Budget planning, retry selection and atomic worker
  admission now re-read the exact grant from the current projection and actual
  publication/admission receipts. Deleted, duplicate and edited records cannot
  preserve that capability. The affected 11 library, 13 decision, 6 scheduling
  and 28 worker tests passed again; the focused current-comment recovery cases
  and strict lint were subsequently rerun.
- Provider intervention could be bypassed by a fresh correction/retry budget.
  Historical account/work guards and serialized authoritative session checkpoints
  now retain the hold through later ordinary outcomes. Bound journal recovery
  preserves earlier intervention/input signals and rejects missing events or a
  different account. A supported provider-resolution integration remains needed.
- A real finite fixture demonstrated an escaping descendant writing after its
  original process group completed. Group/stream exit now fails closed with
  `process_tree_unverified`; it cannot mint passing native check evidence or clear
  full ownership. The containment guarantee remains an implementation gap.
- Shared schema migrations initially failed compilation at hosted proof context
  and target-artifact baseline constructors. Those constructors were corrected,
  and the complete run above subsequently passed. Formatting initially reported
  one small getter; formatting it resolved the check without semantic changes.

## Limits and next integration work

`main.rs` remains the original empty entrypoint. The prepared entry draft is kept
outside build inputs until the real backend is ready. Native host construction
requires mandatory runtime services and reports missing credentials; it does not
silently bootstrap them. Current component composition is not a runnable complete
application.

Remaining engineering includes the full serialized scheduler/phase graph,
correction classification/routing/steering, verified continuation, exact remote
Git acquisition, parent integrated evidence and hydration, deployment baseline
provenance, release/recovery composition, complete process journaling at every
spawn, and enforceable native descendant containment. Required live provider and
agent validation, native isolation and human calibration remain unrun/blocked
under the unattended authorization. No deployment or lasting coordinator launch
is authorized by this checkpoint.
