# Native recovery selection and historical release evidence

The complete crate suite passed **683 tests, zero failures and two explicitly
ignored native qualifications** across 38 groups. Strict all-target Clippy,
formatting, contract inventory and diff checks passed. The freshly built
evaluation plan still declares 56 trial slots with dispatch disabled. All 564
inputs remained at `sha256:c438a22a3ea57152fcf48f92ae9023c1fb753cec9f18c621d875da9dac159c58`.
The [machine-readable evidence](native-recovery-history-checks.json) records source
versions, commands, grouped results, failures and review corrections. Native
containment, actual-agent evaluations, real provider qualification and required
human calibration remain incomplete.

## Scope

The native scheduler selects only a retained, actually joined failed child
implementation session. Provider metadata recovery, original clone inspection
and a resumed model session are separate bounded stages. Successful opaque
proofs remain in memory across ordinary waits; durable stage records identify
uncertainty and prevent replay, but cannot reconstruct those capabilities after
a cold restart. The ordinary scheduler's failure taxonomy remains unchanged.

The original execution clone receives a fixed metadata read root through its
protected registration and complete closed directory family. Configuration still
names the operator checkout. Fresh scoped Git inspection and later source/family
revalidation establish whether reuse is valid. A read root alone grants no model
execution, clean-source or process-exit authority.

Another failed child review within the same native grant requires a fresh proof
bound to its actual completion, unchanged assignment/source and entire phase
inventory. Only supported transient terminal provider classifications permit an
automatic attempt. Original captured caps, contiguous ordinals and backoff remain
in force. A separately approved new grant remains a separate budget lineage.

Historical deployment evidence uses an immutable authenticated proof identity
and exact original dispatch, workflow run attempt, deployment and successful
status. Replacement deployments and later inactive statuses do not rewrite the
past event. This separate capability cannot satisfy a current target baseline.
Terminal ancestor recovery requires the exact recorded successful effect and
retired, reconciled original admission. Current-target recovery remains strict.

## Review corrections

- Automatic review retry initially treated any closed `turn_incomplete` as a
  sufficient cause. The adapter now retains minimal typed failure provenance
  bound to the full terminal content digest; missing, unknown, nontransient and
  policy-bearing failures do not acquire automatic retry authority.
- A saved review assignment applied a fresh-observation age check to its original
  contract. Exact semantic gate equality now binds the immutable record; current
  producer, admission and publication authority still require fresh observations.
- Metadata-stage scheduler reservations need a later release path when a hold
  outlives the actual joined task. Only an in-memory actual join plus the exact
  durable positive exit observation may release that reservation after recovery.
  Loaded dispatch records cannot supply join evidence.
- Native recovery blockers must appear in diagnostics rather than remain only
  in an internal map.
- Actual release permits bind the admitted plan scope, while target challenges
  previously retained the per-step scope. Challenges now bind the actual permit.
- Reconciliation replaces the initial dispatch acknowledgement with the exact
  release effect. Retained proof recovery validates this representation against
  original admission, plan, immutable payload and run identity.
- A crash after writing a target proof but before its lookup indexes must not
  require a new deployment or expired attestation token. Validated same-dispatch
  replay repairs the missing indexes under its actual current permit. Only the
  new-proof branch accepts the original acknowledgement as creation evidence.
- Independent review found that same-dispatch repair still depended on the
  original live permit. A separate read-only fallback for a missing current
  index now observes actual current target facts, selects exactly one
  authenticated retained proof within the scan bounds, and performs the usual
  full current target/run/main checks. A present corrupt index cannot invoke
  this fallback; no token, dispatch or model authority is created.
- The same crash can leave a valid older current pointer after a later proof has
  been saved. Fully authenticating that pointer and then observing different
  current native facts permits the same bounded exact-proof lookup. Corruption
  or a missing referenced proof remains a blocker. An old-pointer/new-proof
  regression covers this boundary after actual Control restart.

Independent bounded reviews of the review retry path and retained execution
read-root bridge found no additional material defects after those corrections.
These reviews are not a complete conformance audit.

## Validation and retained failures

The first shared `cargo check --locked -p kegari-v0 --tests` failed because the
new terminal consumer called `ReleasePlan.operation_id` as a method. The field
access is corrected. The failed log is
`/private/tmp/kegari-v0-native-recovery-history-check-1.log`.

The corrected shared check passed in 16.72s. The recovery planner check passed
1/1 in 0.13s, and the joined metadata reservation check passed 1/1 in 0.10s.
Logs are `/private/tmp/kegari-v0-native-recovery-history-check-2.log`,
`...-planner.log` and `...-driver.log`. The complete crate invocation in
`...-full-1.log` failed after the library group: 187 passed, three failed,
164.85s. Subsequent integration targets did not run. Two target-history fixtures
failed because the new retained-proof reader rejected an actual active admission,
whose durable phase is `Prepared`. The correction requires the exact live
Control permit for that case; passive or unknown prepared records remain
insufficient, and the terminal ancestor consumer still requires retirement.
The existing corrected-successor fixture separately failed with
`observation_unavailable`; the fixture now completes a bounded stable provider
refresh after its publications before the unchanged serialized admission gate.
Both successor fixtures pass in the final full run.

After correction, the deployment integration target passed 18/19 tests in 5.32s
at 564-input source `sha256:7dd2c69789e4c3d035606deaf50d085e2cc08c3882f89c67f6b2e48e977e961e`
(`...-deployment-2.log`). The remaining fixture expected a missing-proof message
but had not supplied valid native facts for its second configured environment,
which the new lookup now reads. The fixture now supplies that valid environment,
deployment and status, while preserving the exact missing-attestation assertion.
The new live/cold
Prepared-owner, missing/stale-index, ambiguity and corruption cases passed.

Strict all-target Clippy on source
`sha256:6e12af73e8e8ce719dac5f74a8af6acc1ad98b2192b0a1499e141701aa1d1540`
reported two structural issues in `runtime_recovery.rs`: too many arguments to
the stage executor and a large proof enum. These were corrected by grouping
stage inputs and boxing proof payloads, without lint suppression. Log:
`...-clippy-3.log`. No behavioral result is claimed for that intermediate source.

On source `sha256:c438a22a3ea57152fcf48f92ae9023c1fb753cec9f18c621d875da9dac159c58`,
strict all-target Clippy passed in 17.76s (`...-clippy-4.log`) and the complete
deployment target passed 19/19 in 5.47s (`...-deployment-4.log`). The terminal
historical target regression passed 1/1 in 1.41s (`...-terminal-4.log`).
The subsequent full run passed all 683 tests; the library group passed 190/190
in 255.01s, including both successor fixtures. Focused checks repeat tests in
the complete suite and are not additional unique passes.

The first failed full invocation used 564 inputs at
`sha256:450f8d4739f6a063d1ff51d9528775e2f2d3492bfb36a98a01b9b47ac7647045`.
The final full run and every final check use
`sha256:c438a22a3ea57152fcf48f92ae9023c1fb753cec9f18c621d875da9dac159c58`,
unchanged in snapshots
during and after the run. Raw full log:
`/private/tmp/kegari-v0-native-recovery-history-full-4.log`, SHA-256
`cc8c38ede972b5366cb96662ea18cdbf3c7d459b6a3f69c4a25e2c3cc2aa3b21`. The contract inventory remains 363 blocks and 136 named
requirements,
with no implicit passes. SPEC 1.2 retains its approved SHA-256.

Reproduce from the Rust workspace using the verified installed Rust/Cargo 1.99.1:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --check -p kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
git diff --check
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json
```

Cargo builds/tests must remain exclusive: relinking a running fixture executable
can invalidate macOS process identity observations. The source/family and review
fixtures use actual Git/Control/protocol observations with explicitly synthetic
terminal or upstream provider facts; they cannot mint the native joined-run cache
or qualified confined-execution evidence. The planner and reservation tests are
component evidence. No native resume/model call, remote mutation, deployment or
lasting coordinator activation was performed.

This increment connects the prepared worker recovery capabilities to scheduling
and adds immutable historical/current target recovery. Its distinct reviewed
behavior and verification boundary justify a new local commit after the earlier
worker/release primitives checkpoint. No history rewrite or push is needed.

## Remaining engineering

Terminal transitive hold completion, findings-to-repair/validation retry lineage,
cold/corrected/repeated session recovery, configured-hook recovery, carried
acceptance and native evaluation fixture migration remain separate work. The
operator remains unattended. Existing denied native probes are not retried.
