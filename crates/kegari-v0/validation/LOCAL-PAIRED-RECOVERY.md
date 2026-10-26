# Paired repair recovery evidence

The final source passed **798 tests, zero failures and two ignored native
checks across 39 test groups**, plus all-target compilation, strict Clippy,
formatting, the contract inventory and a dispatch-disabled evaluation Plan.
The full suite took 809.522 seconds. The ignored native checks remain required
and incomplete; these results establish local deterministic behavior only.

The 629 build inputs stayed unchanged at
`sha256:ad153d5922671337911480153f8d7d2230847eda3747dae5aae76de2feb7cb84`.
[Machine evidence](paired-recovery-checks.json) retains the exact inputs, all
19 invocation results (including failures), command lines, log hashes and
before/after Rust/Cargo 1.99.1 identities. Source and tools were unchanged during
every invocation. The approved SPEC 1.2 hash was also verified unchanged.

Every command below ran in the Rust workspace with `RUSTUP_TOOLCHAIN=stable`
and `CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

| Command | Result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass, 13.322s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass, 14.065s |
| `cargo test --locked -p kegari-v0` | 798 passed; 2 ignored; 809.522s |
| `cargo fmt --check -p kegari-v0` | Pass |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 planned trials; dispatch unauthorized |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | 496 blocks; 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

The final-round component also passed separately on the final source. Its one
test is included in the full-suite count, not added to it. The earlier 37
focused checks belong to source5 and are preserved with that source identity.

The production recovery path retains an interrupted paired repair's original
Findings round and implementation budget. Metadata readback, directory reuse,
model resume and completed-result consumption use distinct typed records and
actual registered capabilities. A serialized reference cannot recreate a joined
worker, resume bridge, source seal or native retry grant. The implementation
attempt counter retains its original floor independently of the review ordinal.

Completed resumed results retain their exact metadata/model/source/preparation
receipts, actual joined runs and bridges, source filesystem ownership and all
consumed native grants. Changed-source validation and later review/repair paths
consume that closure without replacing the original repair or renewing its
budget. Whole publication/receipt identity and unique current grant ancestry are
checked again before admission and result use. Legacy absent optional fields
preserve their original serialization.

Independent review and corrections included:

- Original result flattening omitted actual grant ancestry. The corrected flat
  result/budget leaves retain actual grant capabilities and publication receipts.
- The raw Control continuation reader omitted earlier failed-validation and
  passed-validation references. Its nonrecursive exact-reference checks now
  cover the consumed original and resumed repair chain, including a first
  new-grant Initial source with no review history in that new namespace.
- Repeated round/unfinished ancestry caused recursive and repeated traversal.
  One bounded heterogeneous worklist now handles grant, bridge, source and
  joined-run projections, with typed in-process identity and cycle detection.
- Iterative traversal did not fix recursive destruction. A separate Arc release
  queue detaches actual child edges before destroying leaves; shared owners keep
  their tails. Existing validation retry destruction remains intact.
- Failed validation after a resumed repair reached two direct-only predecessor
  checks. Both now require the exact retained completed-resume capability and
  terminal/plan closure; direct repairs keep their existing verification.

The traversal and cleanup tests use plain graphs through production algorithms,
including 65,536-node shared tails and concurrent final owners. They do not mint
production capabilities. Control component tests use actual protected admission
and receipt writes with explicitly synthetic provider/protocol observations.
The existing source/round component tests retain their native joined barrier;
they do not qualify a real resumed repair followed by qualified failed checks.

Retained integration failures:

1. Source1 all-target check failed because cleanup removed required projection
   wrappers and a newer refusal fixture lacked three optional None fields.
2. Source2 check passed with six dead-wrapper warnings. Root removed exactly
   those unused methods and retained required wrappers.
3. Source3 check passed; strict Clippy rejected four nested conditions and three
   inline test-module placements. Source4 changes preserve predicates and move
   the test bodies unchanged; check and strict Clippy then passed.
4. Source5 adds independently reviewed iterative destruction and passed check,
   strict Clippy and 37 focused tests. Source6 adds the resumed-pair continuation
   correction. Root caught a fixture variable shadow during static review and
   applied a rename-only follow-up before compilation. No assertion was weakened.

All invocation logs, failures, source snapshots and before/after observed
Rust/Cargo identities are retained by the machine evidence collector. Cargo uses
--locked with RUSTUP_TOOLCHAIN=stable and the existing isolated Cargo cache.
The approved specification remains unchanged.

The two required native probes remain ignored and incomplete. No live agent trial,
remote mutation, installation, deployment or denied probe was performed. The
native resumed-repair/qualified-failed-check positive, concrete evaluator caller,
repeated actual-agent/provider qualification and human calibration remain
incomplete. Carried-acceptance and evaluator follow-on drafts remain separate
TEMP work and are not part of this source's test claim. The Goal is not complete.
