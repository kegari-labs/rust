# Local declared observation component evidence

Summary: Exact source2 records verified. These are local component/integration results, not native qualification. Full deterministic suite: 931 passed,
0 failed, 2 ignored across 39 groups in 801.345s.
Tested source: `sha256:07ccd31609ff5d4cb534a780f5e6dc3dc5869b8bdecd59f169cb81fdce3ffa4d`. These totals are derived from the final raw full-suite
log only; focused counts and historical failures are not added to that total.

The increment adds an actual worker authority endpoint diagnostic and opt-in
bounded protocol observation for explicitly declared IndependentReview trials.
The callback binds the registered worker's original permit, contract and request
under the current retained authority gate. Missing/changed evidence remains
unavailable. Protocol capture retains observed request attempts before rejection
and exact completed messages in protected worker records, with counts and bounded
references in reports. Ordinary workers remain opted out. Existing facts and
errors survive grading failure and archive readback.

The machine evidence records exact commands, source/toolchain before and after,
timings, log hashes, every raw test group, the 14 new names and the two extended
grading failure tests. The earlier source1 check success and strict-Clippy failure
are retained. The correction removes one needless borrow in an unchanged report
assertion; it neither changes the assertion nor production behavior. Final
focused, compatibility, formatting, full-suite, data-only Plan, inventory and
diff checks must all exist and match the final source before this template is
published. The preserved plan's descriptive `dispatch_allowed` typo means the
actual Plan field `dispatch_authorized`, which must be false for all 56 slots.

Only four existing partial ledger rows receive appended local component evidence:
E07 and its clause, plus the two reporting/provenance clauses. All prior evidence,
row identities, statuses and stale history remain unchanged. No row is promoted
to passing, and the proposal does not claim that E07 has been exercised.

Enabled protocol capture has no Complete state. Missing bootstrap requests and
unobserved trailing transport frames cannot establish absence of disclosure.
Private records are bounded; report DTOs carry no execution authority. The actual
Control and ScopedWorker fixtures, protected I/O, synthetic SessionState/duplex
broker and data-only report fixtures are distinguished in the test inventory.
None constructs a native join or grants an effect. There is no actual-agent trial,
live provider/native containment qualification, canary non-disclosure result,
complete read/effect/output history, cold execution or human acceptance. Scorers,
fixtures, instructions and qualification predicates are unchanged. Two existing
native-platform tests remain ignored for their recorded OS prerequisites.

Recorded commands and immutable raw logs:

| Recorded run | Exit | Seconds | Command | Log SHA-256 |
| --- | ---: | ---: | --- | --- |
| check-1 | 0 | 24.98 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo check --locked -p kegari-v0 --all-targets` | `ae28de09e22136913af853dc7677be21c5e43141f6b6d895ead2b9e8a40744b4` |
| clippy-1 | 101 | 23.156 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | `c22c0ae0c834d6b9168a4cefe828b3d9ec1561363d8dab1f801f7d5ac71fa23b` |
| check-2 | 0 | 0.491 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo check --locked -p kegari-v0 --all-targets` | `4099e62f0b5a042319b0f7f86a747508367a5d6ea6a1aebaf7cfc74652c7da34` |
| clippy-2 | 0 | 3.468 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | `cf8e132335e7ccf6ad198799bd41f971c12a250ddfbf5d2044924423c2eeaf22` |
| protocol-2 | 0 | 45.294 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib codex::observation::tests::` | `a6981e1c56371481885b1c6b554f48ccfc96c3e1fab0aeaac87410d4dd6ea404` |
| boundary-2 | 0 | 0.219 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib runtime::worker_boundary::tests::` | `0190f894db303ade5af10fe0d9e31ca13276737fdec504a793f4a29fd043ce34` |
| compatibility-2 | 0 | 65.469 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --no-fail-fast --test codex_protocol --test evaluation --test evaluation_reports --test worker` | `0941ec6f631a78c17ae13f71576f873b6cab06528ef5eaba410eefec8a7c6d2a` |
| fmt-2 | 0 | 1.533 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --all --check` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| full-2 | 0 | 801.345 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --no-fail-fast` | `8f91abe3dc474ed856e0206b650995414752d9baf7d74de2f4ff5ca604cd3e92` |
| plan-2 | 0 | 35.615 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | `0771b120c2cf9378324ed3f84c4e370aa9fb084a323a506d0c2a5a2cb7304d81` |
| inventory-2 | 0 | 0.047 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache python3 crates/kegari-v0/tools/contract_inventory.py --check` | `de843922ff0517de68c07be40f91edfad2f63da24be0a5586d6f429bd7d943fc` |
| diff-2 | 0 | 0.021 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache git diff --check` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
