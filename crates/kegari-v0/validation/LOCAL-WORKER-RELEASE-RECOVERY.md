# Bounded worker and release recovery

The checkpoint passed a complete base suite of **668 tests, zero failures and
two explicitly ignored native qualifications**, followed by **122 affected checks**
on the final two-file protocol correction. Strict all-target Clippy, formatting,
contract inventory and diff checks passed. The [machine-readable evidence](worker-release-recovery-checks.json)
records the full-run source and final delta separately; the affected checks are
repetitions, not 122 additional unique tests. Full implementation and operational
qualification remain incomplete.

## Delivered behavior

Session recovery retains actual submitted-input fingerprints, complete provider
history, terminal hook observations and the original operation. A separate finite
metadata admission reads only the retained thread through the guarded provider
transport. It requires actual joined process exit before issuing an opaque
recovery observation. The native consumer independently assembles current
instructions, checks the actual retained worker completion and original phase
bounds, and admits the new operation through the common runtime gates.

Source reuse requires actual fresh host-owned Git inspection and complete bounded
filesystem observations. Raw source blob identities detect hidden index changes.
Protected directory families bind all operations to the original marker and
directory identities. Ordinary registrations cannot alias an existing family.
Cleanup checks every member's receipts, processes and current canonical hook
ledger, then retains the source and writes removal tombstones for every member.
Source and family state are checked again at serialized resume and first-turn
dispatch. The evaluator rejects a resumed admission it cannot faithfully run.

The new provider session retains historical turn identities without copying their
results or totals into the new operation. Historical start, acknowledgement and
completion replays are rejected. Delayed old-turn usage is explicitly unattributed.
The first resumed known-turn counter establishes an explicitly uncertain usage
boundary; its increase is retained without claiming it belongs to the new
operation. Same-thread policy signals persist before acknowledgement bookkeeping
can fail, including owner loss and malformed ordinary turn fields.

An exact-source child review can consume its current native retry grant without
changing the retained review assignment. Its actual failed completion, original
source, native grant and complete phase inventory remain bound through new
preparation, admission and publication. This first slice covers the initial
granted review and a clean result; further failures/findings need more lineage
integration.

Release recovery verifies a finite authenticated chain of original admissions,
signed accepted publications, supersede receipts and successor intents. Actual
Git ancestry and patch identities retain original contributions exactly once
through an intermediate deployment-only predecessor. Complete current baseline
inventory and temporary exact-successor hold projection preserve unrelated
resources and durable predecessor holds. Historical import authorization is
separate from fresh current admission/readiness.

## Evidence and reproduction

Run from the Rust workspace with the verified installed Rust/Cargo 1.99.1:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
target/debug/kegari-v0-evaluation plan crates/kegari-v0/evaluations/manifest.json
```

The complete base run used 561 code/test/instruction/protocol/example/evaluation/
Cargo inputs at
`sha256:49b29e829fa6b366f7661222929201d139a6bd211aee55f748fea64cb0099dc4`.
The same hash was observed after all 38 groups finished. Library tests passed
184/184 in 246.61s, including both actual successor fixtures without teardown
errors. The raw full log is
`/private/tmp/kegari-v0-worker-release-recovery-full.log`, SHA-256
`430eaeee0b66d27b7c5639b961c87369593da6c733e677b5bc6aac20a85a5a81`.
Only `src/codex.rs` and `src/codex_resume.rs` changed afterward. Final 561-input
identity is `sha256:4a2cae298403517bb107b5d80584b7fd4b090bd8143a3e69f91028628b35c631`,
unchanged through final checks. The affected protocol tests passed 2/2, worker
protocol 1/1, library evaluation 17/17, and integration protocol/worker/evaluation/
report tests 102/102. Reproduce those final-source checks with:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib codex::resume::tests
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib worker_run::protocol_tests
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib evaluation::
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --test codex_protocol --test worker --test evaluation --test evaluation_reports
```

The whole suite was not repeated after this narrow correction. All-target strict
Clippy passed on final source in 18.66s. The freshly built evaluation executable
retains 56 declared trial slots and dispatch disabled. Inventory reports 363
contract blocks and 136 named requirements with no implicit passes. SPEC 1.2
retains SHA-256 `80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.

## Review and retained failures

Independent and integrated reviews corrected stale source between binding and
dispatch, unlocked registration updates, missing legacy cleanup fencing, stale
copied hook history, changed canonical-ledger attachment, enrollment replay
without source revalidation, historical turn replay and discarded usage turn
identity. The final accounting/policy corrections passed the affected checks above;
bounded independent rereview found no further concrete defect in those paths.

Earlier failures remain in PROGRESS.md: fixture type/constructor errors, strict
host Git pin rejection, two fixture stack overflows, stale observation and
concurrent Cargo relinking that invalidated macOS process-path observations, and
poll-task teardown racing temporary-directory removal. Boxed stage futures,
bounded read retries, explicit poll cancellation/join and an exclusive Cargo
window corrected those fixture failures without weakening production checks.
The final base suite passed both successor cases cleanly. The first final
protocol test also exposed a fixture using single-thread Tokio for a guarded
dispatch that requires a multithread host. That prerequisite remained strict; the
fixture runtime and failure diagnostics were corrected, then 2/2 tests passed.
Its failed log and source hash remain in the machine-readable record.

The source/family fixtures use actual Git, Control and protected ledgers with
explicitly synthetic terminal metadata. They cannot mint the native joined-run
cache or provider recovery capability. Release fixtures use actual signed
publication/recovery and journaled Git, with explicitly synthetic historical
accepted/check seed evidence. They retain the genuine integrated-execution
barrier and do not establish live qualification.

## Remaining work

Automatic native recovery selection, successful metadata caching, cold/corrected/
repeated and configured-hook session recovery remain incomplete. Further granted
review attempts and findings-to-repair/validation lineage remain engineering work.
Historical target proofs after replacement, terminal transitive hold completion,
carried acceptance and native evaluation fixture migration also remain open.

Native descendant containment remains incomplete; arbitrary repository execution
fails closed. The two ignored qualifications depend on the previously denied boot
identity prerequisite. Denied probes were not repeated. Real provider/background/
remote HTTPS, Tailscale/media, actual-agent repetitions and required human
calibration remain unrun. No live model, remote mutation, installation, deployment
or lasting coordinator activation was performed. The operator remains unattended.

This checkpoint joins the common admission provenance, bounded recovery and
source-preservation changes at one reviewed boundary. It remains a distinct
verified recovery increment after the preceding retry/evaluation checkpoint;
preserving that boundary is the reason for a new commit rather than rewriting
the earlier tested history. No consolidation or push is needed.
