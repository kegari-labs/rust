# Explicit retry, owned evaluation and resume prerequisites

The frozen offline checkpoint passed **638 tests, zero failures and two explicitly
ignored native qualifications**, across 38 groups. Strict all-target Clippy,
formatting, contract inventory and diff checks passed. The
[machine-readable record](retry-owned-evaluation-checks.json) retains commands,
per-group results, source identity, log hashes and remaining limitations. These
results do not establish complete implementation or operational qualification.

## Delivered behavior

An actual unfinished implementation can consume an explicit native retry grant
after exhausting its original budget. The retained proof binds the completed
implementation, source, failed checks/reviews, prior admissions and current signed
comment. The first new admission consumes that proof; ordinary completed work
cannot become generically retryable. Original phase limits and receipts remain.

Preparation now has durable typed provenance for its exact worker, workspace,
account and phase. Current retry/continuation eligibility, attempt, caps, trigger
and spacing are rechecked inside serialized admission. Deleted or edited grants
leave no preparation admission, permit or effect. Preparation metadata cannot be
reinterpreted as model, metadata-budget or release authority.

Retry command completion verifies its actual published grant and prior history
even if the exact successor preparation or model run has started. A narrow local
acknowledgement writes fixed decision metadata and preserves successor unknown
effects. A different budget remains blocked. Actual Control regression cases
exercise preparation/model admission with exact/wrong grants and held effects.

Evaluation fixtures can bind to an actual admitted worker's owned clone, scratch,
full context, instructions, workflow and scope. Seed and clone bytes/modes remain
exact. Private scorer/report paths are excluded from OS grants and host global,
workflow, supplied and provider-discovered instruction inputs. Hardlinked
instruction inputs are rejected. Initial scratch must be empty, including hidden
files and directories, at binding and immediately before trial hooks. Actual Git
and production-assembly fixtures exercise these checks without launching a model.

Resume prerequisites retain exact assembly compatibility and ordered full
provider terminal-turn fingerprints. Only typed operation/attempt metadata can
normalize; rendered workflow and effective instructions remain exact. Known
interrupted turn IDs and cumulative usage baselines are preserved. Malformed
history views fail, and actual same-thread policy intervention is checkpointed
before ordinary late/duplicate/malformed completion errors. Legacy records retain
their prior serialization when optional provenance is absent. This does not yet
implement a runnable provider-session resume path.

The corrected-successor fixture exercises actual registration, admission, native
signed publication/correction, predecessor recovery and journaled Git candidate
assembly. Historical accepted/check/plan seed inputs are explicitly synthetic.
Missing genuine integrated execution retains the hold and blocks production
progress; no confined execution or real provider effect is fabricated.

## Reproduction and identity

Run from the Rust workspace using the verified installed Rust 1.99.1 toolchain:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
target/debug/kegari-v0-evaluation plan crates/kegari-v0/evaluations/manifest.json
```

All 545 source/test/instruction/protocol/example/evaluation/Cargo inputs retained
`sha256:bc2f0ef33219f9f1d4fe29c085f1afd59bb2670c597908536a4936426440db19`
before and after checks. SPEC 1.2 remains
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.
Inventory reports 363 contract blocks and 136 named requirements, with no implicit
passes. The evaluation plan retains 56 trial slots and dispatch disabled.

Raw logs are `/private/tmp/kegari-v0-retry-owned-evaluation-full.log` and
`/private/tmp/kegari-v0-retry-owned-evaluation-clippy.log`; source inventories use
the same prefix with `-source.json` and `-after.json`. Durable JSON retains their
identities after temporary logs disappear.

## Review and retained failures

Independent reviews found the publication self-selection bug, preparation and
model-start acknowledgement races, private instruction discovery and scratch
fidelity gaps, malformed history normalization and policy-hold loss. Root review
also found stale advisory preparation eligibility. Corrections and meaningful
regressions passed; bounded rereview found no remaining material issue in those
paths. Full independent conformance review and human calibration remain required.

Development failures remain recorded in PROGRESS.md: concurrent fixture imports
and helper compilation, the initial generic acknowledgement overlap, mismatched
native/work target identity, an initial zero-test filter, and formatting while
typed preparation was still being edited. The native retry fixture's completion
assertion was moved to its intended postconsumption position before the final
run. None of these earlier results substitutes for this frozen complete run.

## Remaining work and qualification

Actual provider resume, safe shared-directory cleanup ownership, cold unfinished
proof recovery, review/validation retry lineage, cross-correction continuation,
chained historical integrations and carried acceptance remain engineering work.
The complete native evaluation fixture migration, runtime/CLI live authority and
actual host-result composition also remain incomplete.

Native descendant containment remains incomplete; arbitrary repository execution
fails closed. The two ignored qualifications depend on the previously denied boot
identity prerequisite. Denied native probes were not repeated. Real provider,
background/remote HTTPS, Tailscale/media, actual-agent repetitions and required
human calibration remain unrun. No live model, remote mutation, deployment,
installation or lasting coordinator activation was performed.

This increment establishes a distinct explicit-retry and evaluation-binding
verification boundary after the previous continuation checkpoint. Keeping that
boundary is preferable to rewriting the previous commit. No consolidation,
published-history rewrite or push is needed. The operator remains unattended.
