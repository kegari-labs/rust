# Carried acceptance and bounded parent reassembly

The final source passed **819 tests, zero failures and 2 ignored native checks across 39 groups** in 771.633s. All-target compilation, strict Clippy, formatting, the contract inventory, diff and a dispatch-disabled evaluation Plan also passed. The required native checks remain incomplete; these are local deterministic results.

All 648 build inputs stayed unchanged at
`sha256:741b805e1a0209cb2994bf66190e7a1aca4648978bd5e0f527abbe1ab4e4184b`.
[Machine evidence](carried-acceptance-checks.json) preserves 22 actual
invocations, including failures, with exact source/tool/log hashes and command
lines. Rust/Cargo 1.99.1 identities were observed before and after each command.
The approved SPEC 1.2 and coverage ledger remained unchanged.

Commands ran in the Rust workspace with `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

| Command | Result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass, 3.716s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass, 4.940s |
| `cargo test --locked -p kegari-v0` | 819 passed; 2 ignored; 771.633s |
| `cargo fmt --check -p kegari-v0` | Pass |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 planned trials; dispatch unauthorized |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | 496 blocks; 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

The final source also passed the affected original-release test separately in
29.401s. Source5 passed 27 focused grant,
parent and no-release tests before the test-only source6 correction; the final
full suite includes those tests. Focused results are not added to its count.

The change reassembles a previously human-reviewed unchanged work slice when a
known unrelated release advances its production baseline. It retains the original
human acceptance and exact reviewed token, builds a new complete parent vector,
runs new integrated/review/production validation, then publishes a separate carried
acceptance link. It cannot relabel the human's acceptance as review of new evidence.
The actual current published carry is required before release or an empty-slice
no-release completion can use that acceptance.

Owned source acquisition verifies each original production, accepted head, squash
and current-main object through the admitted Git operation. Missing-object reads,
imports and unknown outcomes remain finite and journaled. Scoped owned source
fixtures preserve the operator checkout. The observed Git 2.56 object-query result
is CLI compatibility evidence only, not a source capability.

Reassembly retains existing phase budgets and complete source/preparation/run
lineage. A real explicit native grant can create the next bounded namespace after
an exact failed predecessor. Known transient retries stay within that same grant,
advance its ordinal, retain minimum caps and honor exponential delay plus longer
Retry-After. Actual joined results, native grant publications and whole protected
receipt identities remain required. Signed data cannot recreate a joined worker
or permit. Pending assignments and operation IDs survive reporting failures;
ambiguous effects never justify a replacement attempt.

The no-release path retains the originally empty selected slice and a complete
nonempty parent repository vector. Its current carry is checked at admission,
provider callback, after historical reads and final serialized installation.
Historical references remain distinct from live current authority. Legacy absent
fields preserve direct signed bytes. A copied completion cannot create current
carry authority.

Independent review corrected successful-preparation receipt shapes, complete
same-work grant ancestry, active owner/pinned configuration checks and exact E2E
child gate retention. Active work uses the actual admitted permit and retained
parent/E2E FreshGate through harmless reload; new admission and current readiness
remain strict. The independent installed-source review found no further material
issue. Native positive qualification is still required.

Retained integration failures and corrections:

1. Source1 check failed with 11 errors and one warning: missing imports/method
   visibility, boxed effects, Arc workspace shape and one unused export. Seven
   narrow corrections were independently reviewed.
2. Source2 check passed; strict Clippy failed on four unused accessors, an unneeded
   unit expression, three large parameter lists and test-module ordering. Root
   removed unused wrappers, grouped typed inputs with unchanged checks and moved
   the same test module; source3 check and strict Clippy passed.
3. Source3 parent tests failed because optional UUID keys could not serialize as
   a JSON object. Granted inventory now hashes ordered namespace/value pairs;
   legacy ungranted bytes remain unchanged. Tests also verify order invariance
   and the original legacy digest. Source4 passed all seven parent tests.
4. Source4 grant test incorrectly expected a data reader to enforce new-admission
   filtering. Source5 asserts the excluded parent and actual NewOperation gate's
   `excluded` refusal; actual active E2E, grant and fence checks remain intact.
5. Source5 full suite stopped with 308 library passes and one failure: an old
   release-origin test expected the removed unsupported-carry error. The actual
   reader rejected the synthetic plan's non-workspace record namespace with
   `acceptance_carry`. Source6 checks that exact refusal, unchanged whole receipts
   and record tree, and absence of provider mutations. Prior native-publication
   negatives remain. The corrected test and final full suite passed.

Earlier TEMP compositions v4/v6 stopped on incorrect base-directory assumptions
before repository writes. Original files, failed inputs and all invocation logs
remain retained; successful composition verifies exact base/draft hashes. No test
was removed, native capability invented or lint suppressed to obtain these results.

Component checks use real protected Control/DecisionManager/read paths with
explicitly synthetic offline provider observations. Pure semantic tests supply no
opaque execution proof. Actual native complete carry production, carried release
and carried empty-slice current-to-cold positives remain unqualified. The required
ignored probes are `child_signal_uses_owned_unreaped_handle_and_never_adopts_pid_metadata`
and `native_host_and_birth_identity_are_observed_without_environment_trust`.

Completed-current carry reconstruction, cold worker reassembly and the concrete
native evaluator caller remain separate uninstalled TEMP work. Repeated actual
agent behavior evaluations, real provider qualification and human calibration
remain incomplete. No live trial, remote mutation, installation, deployment or
previously denied native probe was attempted. The Goal remains incomplete.
