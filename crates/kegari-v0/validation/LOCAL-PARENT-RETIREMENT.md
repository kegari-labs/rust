# Retained parent-run provenance: local validation

The final source8 passed **879 tests, 0 failures and 2 required
native ignores across 39 groups** in 716.047s. All-target compilation,
strict Clippy, the delivery integration target, formatting, contract inventory,
diff and dispatch-disabled Plan passed. All sixteen added tests, all three
successor fixtures and the exact delivery regression passed in the final full
suite. These are local component/integration results, not native qualification.

 Source7 passed all 361 library tests and all
three successor fixtures, then failed a later delivery integration fixture. Its
actual logged aggregate is 563 passed and one failed across fifteen groups, not a
successful full suite. Source8 changes only that fixture's protected records path
to its actual workspace UUID. All original failures and corrections remain retained;
the final result rests on fresh source8 checks, not an inference from that correction.

The collector verified zero failures, exactly the two named native ignores, and
every named new/strengthened test in the same final full log. The total is derived
from that log; historical focused counts are not added to it.

All 679 source8 source/test/asset/evaluation inputs remained unchanged during the
final checks at:
`sha256:581ef09275d7b370bbfc4dba4382e1e69cca75f3cb112fc5ffd4282193eaecb1`.
[Machine evidence](parent-retirement-checks.json) retains 38 invocations, exact before/after snapshots, command lines, Rust/Cargo identities,
log hashes, failed intermediate checks, correction diffs and independent reviews.
The nine parent-storage/data regressions and seven startup regressions are sixteen
new tests, not sixteen new native qualifications. SPEC 1.2 and the reviewed coverage
scope remain unchanged: eight rows, eleven added entries, all partial. Local
evidence verification is complete; remaining native/human obligations stay open.
No additional OS requirement is marked passed by this correction.

Commands ran with `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

| Command | Observed final result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass; 1.199s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass; 0.828s |
| `cargo test --locked -p kegari-v0 --test delivery` | 21 passes; zero failures/ignores/filtering; exact previously failing test passes; 19.664s |
| `cargo test --locked -p kegari-v0` | Actual full totals above; all sixteen new tests, three successor fixtures and exact delivery regression pass |
| `cargo fmt --all --check` | Pass; 1.422s |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 declared slots; dispatch unauthorized |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | Pass; 496 clauses, 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

The source5 successor filter and source7 focused filters are retained historical
evidence, not replacements for the source8 full-suite requirement. Source7's
startup filter passed seven tests (24.881s); process integration passed 46 across
three complete groups (108.964s); origin passed three (0.340s); retirement passed
eleven (12.702s). The process groups had zero failures, ignores or filtered tests.
These unchanged filters are not rerun for source8, and their totals are never added
to the final full total. Source7 compilation, Clippy and formatting also passed,
but its full failure remains a failure. The collector also
verified the exact auxiliary set SPEC.md, tools/contract_inventory.py and
validation/coverage.json, with matching before/after and current file hashes.

## Original authority and historical evidence

The two parent producers capture the actual assignment, signed vector/candidate/
intent/delta, selected integrated child publications, source descriptors and
flattened reassembly history. The successful preparation path binds the exact
instruction/context/prompt/tool/manifests and original preparation receipt into
protected signed origin data. Its private in-memory origin travels with the real
registered contract, outside the assignment's existing serialized digest. There
is no decoded-assignment constructor or caller-provided JSON capability.

Only the actual ordinary `OwnedWorkerRun` join can create the retirement candidate,
with the identical registered contract Arc and original permit/budget. A trial's
joined flag, externally supplied completion or retained JSON cannot create it.
The historical writer then checks closed preparation/model receipts, actual
execution/check/journal/process observations and the original source/assignment
closure. Failed results remain failed history. A signed seal records those facts
without creating a new worker, model admission, budget or current delivery credit.
These production boundaries were inspected; the new data tests do not exercise a
genuine model run or mint its opaque capabilities.

History errors remain separate per-operation diagnostics and retry in the same
process after ownership closes. The original authentic seal identity is remembered
before later fallible closure reads, so removal cannot authorize reminting. Retained
history readers authenticate the exact original records and bytes; they do not
reconstruct runtime worker state after a restart. Existing evaluator ownership,
terminal completion retries, PR text and prior cold-carry paths were preserved in
three independently reviewed clean merges.

## Exact bytes and meaningful regressions

Three artifact tests use real protected local files and the production chunk
collector. They mutate bytes before the first history capture and require refusal
against the original signed-contract digest, with no partially updated hash map.
They cover ordered concatenation, expected length, duplicate/missing chunks,
conflicting shared chunks, per-chunk size and the existing empty-delta distinction.
The collector checks the actual same ordered bytes before retaining file hashes;
hashing whatever bytes happen to remain would not establish their original identity.

Two journal tests cover captured-file replacement and strict decoding. The hash
and decoded event must describe the same previously captured buffer. The shared
storage decoder preserves schema version, unknown-field refusal and payload digest
checks; journal events keep their existing 256 KiB limit. Scope binding and recovered
session history are independently checked by their existing paths. The test's check
and hook payloads are labeled data, not genuine check results or native exit proof.

Four retirement tests exercise receipt-data and protected-file inventory boundaries.
They reject unresolved/unretired owners, changed owner generation/scope/phase,
duplicate supplied effects, and missing or altered worker/hook bytes. Failed effect
results stay in the provided inventory. The test named `complete_effect_inventory`
checks the supplied receipt set only; it does not prove all emitted effects are
present. A separate regression requires the sole serialized history discriminator
to remain `retained_receipt_scan_without_emitted_index` and rejects invented
complete-history values.

The three-origin and eleven-retirement filters cover the nine new parent tests plus
five existing related tests. The separate startup filter covers seven new tests.
The collector verified all sixteen exact qualified test names in the source8 full
log, plus source definitions; it also retains their source7 full-log passes. The
source7 process integration results are historical and are not presented as final
source8 command results. All final totals derive solely from full8.

## Findings and corrections

Independent review found that the original artifact snapshot could accept changed
chunk bytes before its first capture. The correction compares the exact bounded
ordered bytes with the original artifact digest/length before storing any hashes.
Root review also found that journal hashing and interpretation read the same file
twice. The correction decodes the already-captured bytes through the unchanged
protected-envelope decoder. Independent rereviews found no remaining material
defect in those changes or the three composition merges.

The first compile failed because the carry-history import pointed one module too
high and three test vectors inferred unsized strings. The correction fixes the
module path and explicitly types those vectors as `Vec<String>`. Strict Clippy
then rejected a needless clone in a single-element test slice; `slice::from_ref`
preserves the same assertion. The assertion was subsequently formatted.

Source3's origin filter passed all three tests, but the retirement filter passed
ten and failed one. Its actual protected-file test exposed a production defect:
both new history modules requested 16 MiB reads from `PrivateDir`, whose existing
limit is 8 MiB. Such a request fails before reading even a small or absent file,
preventing origin/retirement persistence. Source4 lowers both call-site bounds to
8 MiB, matching the existing reader/writer API. It preserves every storage check
and all pre-write length checks. Independent review confirmed consistent origin
prompt/record and retirement-seal bounds; artifacts still use 4 MiB chunks and a
256 MiB aggregate maximum, and journal events retain 256 KiB. The original failing
regression passed in the historical source4/source5 focused results. The same regression also
passed in the final source8 full suite. No assertion, storage protection or lint rule was
weakened.

Source4's full suite passed 353 library tests and failed the corrected runtime
assembly fixture. Its deliberately synthetic upstream source had recorded a
constant merge UUID without creating that receipt. Production provenance capture
correctly refused the dangling reference. Source5 uses the existing typed merge
broker with the local synthetic provider adapter to produce an actual acknowledged
Control receipt, then records its exact request ID in both retained delta references.
Assertions retain the acknowledgement's provenance_verified=false classification;
upstream checks/review/staging authorization remain explicitly synthetic.

The existing corrected successor test now removes that exact protected receipt,
requires assignment refusal, restores its original bytes, verifies the whole Control
inventory is unchanged, and continues the original positive assignment/evidence
checks. A background-observation barrier isolates the temporary deletion. No new
receipt is fabricated, no production gate is relaxed, and this strengthens an
existing test rather than increasing the test count. The source5 successor filter passed all three tests in 250.528s, but the later
source5 full invocation failed after 415.778s: 353 library tests passed and the
chained deployment-only predecessor fixture failed during `capture_delta` at
`tests/support/release_wire.rs:2006`. The corrected-assembly fixture passed in that
full run. Both full4 and full5 failures remain required historical evidence.

The full5 error was `operation_uncertain`: the initial owned-child identity snapshot
could not establish exactly one matching child identity. The log does not identify
the Git subcommand, observed candidates or syscall timing. It does not prove a
Darwin dying-process race. Source inspection did identify a narrower liveness gap:
the Git runner made only one immediate terminal/group observation after a failed
identity snapshot, then permanently retained that error even if cleanup later
observed the original child exit.

The bounded correction waits only for actual terminal/group closure of that same
unreaped child, for at most 100ms further limited by the original command and
operation deadlines. It never retries `ProcessProbe`, replays a command, supplies
pending stdin during uncertainty, creates a `Running` identity, or treats timeout
as exit. Other probe/storage errors keep their previous immediate-terminal fallback.
Cancellation and deadlines use local predicates inside the existing start gate;
there is no Control lock reentry. Independent v2 review found that gate/guard setup
could consume the command budget before launch; v3 checks the same deadline at
entry, start and immediately before spawn as well as during the wait.

Seven new regressions use actual owned disposable subprocesses with explicitly
synthetic host metadata and deliberately empty child snapshots. They cover a
pipe-held first nonterminal observation followed by actual exit, timeout and
cancellation with the owner/guard retained, withheld input and original failure,
expired entry and delayed-start no-spawn refusal, and expiry after a real spawn
with actual cleanup. They never mint a running process identity or claim native
boot identity, detached-descendant containment or native parent execution.
All seven also passed in the final source8 full suite. Source7 removes the unused
test-helper `mut` identified by source6 strict Clippy. No production logic,
assertion or storage/ownership guard changes; the source6 failure remains retained.

Source7's full invocation passed all 361 library tests, including the corrected
assembly, chained deployment-only predecessor and ordinary historical release
fixtures. It then failed `parent_slice_uses_actual_production_and_preserves_dirty_operator_checkout`
at `tests/delivery.rs:1785`: `parent_retirement: parent history requires its exact
workspace namespace`. The delivery group had twenty passes and one failure; the
whole incomplete invocation logged 563 passes and one failure across fifteen groups
in 491.551s. No later groups or native-ignore results are inferred from that run.

The integration fixture supplied `directory/records` directly to DeliveryServices,
whereas production creates the child directory named by the actual workspace UUID.
Source8 uses the same PrivateDir child construction with the configured workspace.id.
It changes no production code, assertions, receipts, source objects or capability
constructors. The fixture uses an actual local production commit and an empty
staged-delta list; no missing merge receipt is manufactured. The full7 failure,
exact source7 snapshot and correction/review artifacts remain required evidence.

The final source8 full log contains all sixteen added tests plus exact passing
names for the three successor fixtures and the delivery test above. The dedicated
source8 delivery target also passed that test. Historical source5/source7 passes
were not used to satisfy these final-source requirements.


Independent review of the unexecuted source7 finalizer found that an inventory,
collector or render failure after the coverage write could leave partial published
state. The corrected v2 preflights the exact commands, source/log/toolchain records
and rendering before any write, then restores its own ledger and preserves unpublished
outputs in an owned backup on ordinary post-write failure. It refuses to overwrite
changed external bytes and retains all raw logs; it does not claim crash atomicity.
The original finding, v2 resolution and source7 collector/render review remain
archived. Neither source7 finalizer nor collector ran after full7 failed. Any
source8 finalization still requires its own exact source8 guards and successful logs.

## Remaining limits

This history authenticates a scan of currently retained Control receipts. It
cannot detect an acknowledged receipt removed before the first capture. The
explicit incomplete discriminator prevents presenting that scan as a complete
owner-emitted receipt inventory. The separately designed Control membership index
is uninstalled TEMP work and contributes no evidence here. Historical process
membership and coherent rollback of all trusted anchors also remain separate
questions; this implementation claims no general anti-rollback guarantee.

A genuine native parent-run positive, actual closed model/descendant/hook execution,
complete historical membership and fresh current cold reconciliation remain
unqualified. Stored provenance cannot recreate `OwnedWorkerRun`, `RunCompletion`,
`ConfinedExecution`, a permit or permission to reuse a predecessor. The two required
native probes remain ignored because their existing host prerequisite is blocked.
Actual-agent repetitions, provider/containment qualification and required human
review/calibration remain incomplete. No live model/provider work, remote mutation,
installation, deployment or previously denied native probe was attempted.
See [PROGRESS](../PROGRESS.md) for the current checkpoint and dependencies.
