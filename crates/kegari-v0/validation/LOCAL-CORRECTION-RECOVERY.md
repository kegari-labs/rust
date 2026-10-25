# Correction and delivery recovery checkpoint

The frozen offline checkpoint passed **577 tests, zero failures and two native
qualification tests explicitly ignored**, across 36 completed groups. Strict
all-target Clippy, formatting, contract inventory and diff checks passed. The
[machine-readable record](correction-recovery-checks.json) contains per-group
counts, commands, source identity and log hashes. These results establish local
deterministic evidence; implementation and operational qualification remain
incomplete.

## Delivered increment

The executable now composes the native control factory, shared worker lifecycle,
concrete delivery decisions and runtime progression. Active corrections retain
original process/admission ownership while installing a verified native correction
chain. New checks and completed claims bind to the effective corrected generation;
historical results do not acquire current authority. Pending classification keeps
source/effect gates closed and uses the original finite deadline. An acknowledgement
of an already sent protocol request remains tied to that exact original request.

Scoped Git acquisition validates bounded protocol-v2 packs, exact object closure,
provider account/repository/fork identity and the pinned Git executable. Retained
source discovery verifies signatures, actual import receipts and local bytes before
allowing fixed metadata reads. Missing retained sources while STOPPED cause no
fetch, import or directory creation. Acquired clones retain the configured operator
checkout attribution and preserve dirty reuse.

Explicit release recovery uses actual acknowledged commands, original immutable
plans, distinct resumed admissions and exact step-attempt receipt lineage.
Abandonment preserves unknown effects; replacement holds remain assigned to the
exact successor. Historical parent/production readers do not substitute current main
for the original source proof. Nonempty repository maps in retained records use
validated sorted tuple arrays, with actual signed JSON roundtrip coverage.

No-release completion publishes an authenticated record only after current integrated
validation, human acceptance, complete production observations and resource checks.
Exact native readback and actual host receipts establish dependency credit. It
consumes no release spacing and sends no production change. Local ownership survives
caller cancellation; unknown effects are not resent. Known-not-sent attempts retain
their original finite budget. Safe replanning of an unadmitted intent preserves its
prior bytes and cap. Actual owner loss before provider dispatch prevents the write.
STOPPED readback, explicit lost-reply recovery, stale publications, changed production,
held resources and acceptance/history invalidation have deterministic runtime tests.

The evaluator now requires exact whole fixture context, declared variants and sealed
preparation state on both direct and admitted-worker paths. Nested `.git` additions
are visible to scope grading. Suite version 2 and `kegari-observable-v2` explicitly
version these rules; old results remain historical and require regrading for
comparison. The executable plan command reports 56 slots with dispatch disabled.

## Reproduction and source identity

Run from the Rust workspace with the installed verified Rust 1.99.1 toolchain:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
target/debug/kegari-v0-evaluation plan crates/kegari-v0/evaluations/manifest.json
```

All 525 source/test/instruction/protocol/example/evaluation/Cargo inputs retain
`sha256:d8ecbd165a7320a864934c4eea40451e0ed1d80ebdd78d334d08f3f3ea2146e9`.
The hash was checked before and after the complete run. SPEC 1.2 remains
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.
The coverage inventory still distinguishes its 363 contract blocks and 136 named
requirements from actual qualification; inventory completeness is not conformance.

Raw logs are `/private/tmp/kegari-v0-correction-recovery-full.log` and
`/private/tmp/kegari-v0-correction-recovery-clippy.log`. Input hashes are in
`/private/tmp/kegari-v0-correction-recovery-source.json`. The durable JSON record
retains their identities when these temporary files are no longer available.

## Review corrections and retained failures

Independent bounded review and affected regressions addressed recovery selection of
a newer admission's effects, healthy-plan retirement before resume, incomplete PR
identity comparison, correction/ACK races, stale no-release dependency credit,
provider-target/work-key confusion, pre-admission configuration races, lost owner
checks and unsupported JSON map keys. The no-release review also distinguished
semantic target identity from a newer equivalent native provider record while
retaining original publication provenance.

Targeted development runs encountered in-flight compilation errors and fixture
canonical-path/schema mismatches. They were corrected without weakening production
gates. A fixture's successful merge now closes its PR; a pending-case fixture needed
to explicitly reopen it. Strict lint caught an unnecessary let-and-return, corrected
without suppression. The first manual evaluation-plan invocation used unsupported
`--suite` syntax and returned usage/exit 2; the documented positional invocation
passed. The final frozen full run had no test failures.

## Remaining work and qualification

Required implementation still includes genuine unfinished-work continuation,
compatible resumed-thread ownership, deployment-only corrected successors, complete
historical UI reconstruction, full historical runtime resume/supersede fixtures and
native evaluation fixture composition. Remote source recovery cannot fetch missing
objects while STOPPED, and uncertain import/enterprise transport qualification
remains incomplete.

Arbitrary repository commands remain blocked before launch because verified native
descendant containment is not implemented and qualified. Native process prerequisites,
real Linear/GitHub/model activity, installed/background service, remote control and
required human evaluation calibration remain unrun. No live model/provider mutation,
installation, deployment, service activation or remote repository write was performed.
The unattended goal remains active; this checkpoint is not permission or readiness
to coordinate the lasting build.
