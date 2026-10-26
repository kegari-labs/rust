# Kegari v0 build progress

Local implementation and validation remain in progress; the Goal is incomplete.
Unattended mode remains in effect. No live trial or remote mutation is authorized.

Transport source3's full suite passed: 947 tests, zero failures and two required
native ignores across 39 groups in 800.907s. All 16 new tests, three extended
diagnostics, both previously failing tests and both other affected successor
fixtures passed once in their actual full targets. The 692 tested inputs remain
`sha256:e908c28aada5577ce254bcf2a78e2eb5e5c5c9b8e1950b4d01bccd20ce7e9c65`;
the toolchain also remained unchanged. Full log SHA-256:
`19d9de485c37ce593a61d505209d59750baf48da3bc9c84368944cb4277de409`.
The dispatch-disabled Plan passed in 35.245s with 28 variants, 56 slots and
`dispatch_authorized=false`. Session21501 is closed; no validation process remains
running. Inventory passed in 0.041s and diff checking in 0.020s. The local commit
remains pending.

Source3 all-target check passed in 17.691s, strict Clippy in 18.110s, both receipt
regressions in 26.167s, exact validation-retry regression in 30.942s and exact chain
regression in 208.590s. Whole control_storage/reconcile targets passed 53 tests in
28.428s; formatting passed in 1.516s. Source1/source2 failures below remain retained.

Source3 includes the reviewed receipt-snapshot and fixture publication-window
corrections: six additional paths, one new module and two component regressions.
Exact originals/composition are at
`/private/tmp/kegari-v0-transport-source3-candidate`. Root and independent reviewers
read both corrections. Receipt review SHA-256:
`e9f49a988b8006d1506cd969dad8a460a60c17642b0e14fa16c35cadcd86a0c9`.
Fixture review SHA-256:
`8f0477e0c16ae4e23a14252ae05612ab08df7a7b14cbd707bfbab10544baefb3`.
No material static finding remains in these bounded changes. Receipt inventory is
captured under Control once and used as immutable data for ordinary/active
reconstruction; strict storage checks are unchanged. The fixture uses its existing
poll barrier only around each synthetic accepted-origin publication sequence,
with a real complete refresh and unchanged admission/mutation checks. Neither
review proves the exact cause of the earlier failed run or native qualification.

The source3 evidence collector completed successfully after root and independent
review. Root verified all 21 held members and 202 external pins. Independent review
found no material issue:
`/private/tmp/kegari-transport-source3-evidence-independent-review-v1/REVIEW.md`,
SHA-256 `28145ef6ed539f5356b49d029b81cdcb918dbd02dff00a57b6d641553eef90c1`.
The executed collector verified all 20 actual records, retained both source2
failures, derived final counts from full3, and verified exactly five append-only
partial coverage entries.
All five rows remain partial; previous evidence, statuses and ordering are unchanged.
Coverage SHA-256:
`fb108febf3e9ae629922e228d854fc738585710440a5494813376fd4daf64fa0`.
Reports: [local validation](validation/LOCAL-DECLARED-TRANSPORT.md) and
[machine evidence](validation/declared-transport-checks.json). Their SHA-256 hashes
are `c7048b70f2437d2e8114324a83d37ca6dd744798965fa343d18cfa6b710643c3`
and `9913659fd4b420cabbe9fae7a1de30c517b359e0045eff31cd8ec07ec6f69833`.
The original exclusive TEMP bundle is
`/private/tmp/kegari-v0-transport-verified-evidence-3`; its COMPLETE manifest was
verified before copying the exact reports. Ledger originals are preserved in
`/private/tmp/kegari-v0-transport-ledger-finalization-3`.
The final artifact audit found no remaining material inconsistency after the
progress record clarified that the executed collector verified the final results:
`/private/tmp/kegari-transport-source3-final-artifact-audit-v1/REVIEW.md`, SHA-256
`85363ad494ea698550e8617f29af1b165723bee87ea880b6e171d16056779d6d`.
A separate source3 integration audit also found no material discrepancy in the
six corrections or nine pipeline records, SHA-256
`8214663a225e55cf0bac4f47bcb2679b25d245213ae9976a27b60eb27b7d01a3`.

This increment will retain a separate local commit: the new original-reader
lifecycle and receipt-reconstruction serialization have distinct runtime effects
and regression evidence from the prior observation-record checkpoint. Preserving
that boundary makes the locking correction and transport behavior independently
reviewable; the earlier validated commit will not be rewritten.

Broker v2 composition is being prepared in TEMP and is not installed or tested.
Canary materialization remains a reviewed design; its bounded implementation is
queued after the integrated source and broker correction reviews. The canary design's independent review found no
material defect within its bounded scope (SHA-256
`8939e5e2d3c72c25a3a9b59470cc17ead00121919ef29cdea7ae462e6468e35f`).
The exact policy input, attack placement and E07 source remain prerequisites;
current scorer/fixtures and all native/live/human qualification remain unchanged.

The following transport source1/source2 milestones are historical.

The next transport increment is installed and under validation. Eleven source
and test paths (two new) compose reviewed TEMP v2, v4 and v5, preserving the
committed report-test lint correction. All 67 held artifact members matched;
originals and exact composition are at
`/private/tmp/kegari-v0-transport-install-candidate-1`.
The 691 inputs are frozen at
`sha256:222456b52ef00b6027f84f691a2abfcc47c9ef2638b5bad2a7b519962800a862`.
Source1 all-target check passed in 25.271s. Strict Clippy failed in 18.403s on
two collapsible conditionals; session33820 closed before tests. Source2 changes
only those equivalent conditional expressions, without suppressions or assertion
changes. The exact before/after/diff are preserved at
`/private/tmp/kegari-v0-transport-source1-lint-correction`; standalone rustfmt check
passed. Source2's 691 inputs hash to
`sha256:6053568c0c7ca2a883132f12542bf142aedfc31ab18e561d58fc586b9d132a78`.
Source2 check passed in14.974s, strict Clippy in25.782s, all23 protocol observations
in44.872s, both deadline regressions in0.110s, all117 compatibility tests in65.497s
and formatting in1.515s. Source and toolchain stayed unchanged. Session95959 is
closed with exit101. Full `--no-fail-fast` finished in782.758s:943 passed, two
failed, two required native ignores across39 groups. Its library completed421 passed and
two failed in377.27s. `exact_validation_retry_uses_native_grant_and_new_checks_without_join_fiction`
failed at continuation_tests.rs351 during source sealing with `unsafe_storage`
(owner-only/ownership/link check). `chained_runtime_retains_merged_contribution_through_deployment_only_predecessor`
failed at chain_fixture.rs394 with `observation_unavailable` (latest reconciliation
did not complete). All other targets completed; Plan did not run. The full log
SHA-256 is `548c1ca0b9b7a9d499add0f47dde092e2a1956d3ae8cba88d021232e3e24a3fc`.
No source or toolchain changes occurred during this run; no Cargo process remains
active. The source2 evidence collector remains held and unexecuted because this
failed run cannot satisfy its passing-report guards.

Independent diagnosis found a reachable receipt-read/atomic-replacement race in
ordinary and retained observation reconstruction: receipt inventories were read
outside Control serialization while effect writers replace those files. An old
descriptor may become unlinked before its strict owner/link check. This matches
the first error, but its exact lost path/metadata was not logged, so causation is
not asserted. A TEMP correction will capture one receipt inventory under Control
and pass it to both read-only reconstruction helpers. Storage checks remain
unchanged. The chain failure's underlying observation error remains unknown;
separate diagnosis is in progress. All failures remain part of the evidence.

The draft reviews found a lost pre-establishment completed-message projection and
a contradictory request-count tail label. V4 corrects both and restores exact
overflow no-write and actual message-payload corruption/readback assertions. V5
corrects only a new test's expected Conflict for a rejected terminal; production
classification is unchanged. All prior artifacts and findings are preserved.
The final narrow independent review found no remaining material issue within its
scope: `/private/tmp/kegari-declared-protocol-transport-v5-independent-review/REVIEW.md`,
SHA-256 `39988117027fea7dc59de7245eebc44356eef8c9883e02cfc180ff9462121d3a`.
Fourteen new component tests remain unrun. Clean read EOF stays diagnostic-only;
all capture remains incomplete. Operator guidance now describes actual bootstrap,
tail bounds and remaining limits. Broker read/effect work remains TEMP-only.

The following declared-observation checkpoint describes committed source, before
the currently unverified transport changes.

The declared observation increment is locally verified: 931 passed, zero failed
and two required native checks ignored across 39 groups in 801.345s. All 14 new
tests and both extended grading-error tests passed once in their full targets.
The 689 tested inputs and toolchain remained unchanged at
`sha256:07ccd31609ff5d4cb534a780f5e6dc3dc5869b8bdecd59f169cb81fdce3ffa4d`.
All-target check, strict Clippy, ten focused tests, 117 compatibility tests,
formatting and the 56-slot dispatch-disabled Plan passed. Inventory passed in
0.047s and diff checking in 0.021s. No validation process remains running;
session34634 is closed. Native, live-provider and human qualification is incomplete.

Four reviewed coverage entries are installed: all four existing rows remain
partial, with previous evidence, statuses and ordering preserved. Coverage SHA-256:
`7e6946ca588a5836d17e694c123d4cc0393850ff1502f01d7efb6b8f81ef1779`.
The independently reviewed collector completed successfully and verified all 12
invocations, including the earlier lint failure. Reports are
[local validation](validation/LOCAL-DECLARED-OBSERVATION.md) and
[machine evidence](validation/declared-observation-checks.json).
The original verified bundle remains at
`/private/tmp/kegari-v0-declared-observation-verified-evidence-2`.
The actual invocation checked that exact output path was absent, its parent
canonical `/private/tmp`, and outside the repository; this is a caller restriction,
not a generic TEMP-path guarantee in the script. The machine archive adds the
later collector review and invocation constraint without changing any observed
result. Review SHA-256:
`d78d7c735d491aa168cd83a18c070b4cef5c6cd4b3eecfb64cd2fd3bc71d86df`.
The increment is committed as
`102984e2239114d6261e2383da589b50510b9371` —
`feat(kegari-v0): retain declared evaluation observations`.
All 21 intended files and 689 tested inputs matched HEAD, with a clean tree after
verification. No commits were consolidated, rewritten or pushed. Verification:
`/private/tmp/kegari-v0-declared-observation-commit-verification.json`.
The final independent artifact audit found no material inconsistency; its SHA-256
is `70ab0db05208fbdcf9299f1aba47688edbf6c07fa510cb6a646c721aef736462`.
These evaluation diagnostics have distinct consumers and qualification limits from
the prior parent effect-history change, so the increment retains its own
validation checkpoint and commit. Post-commit work is limited to this progress
record and review of the uninstalled transport/broker drafts described below.

The reviewed composition installed 16 files, including six new modules. Exact
original/draft hashes and originals are retained at
`/private/tmp/kegari-v0-declared-observation-installed-source1`.
Source1's 689 inputs were
`sha256:e053140cbf917609dee9254fa8ad3c33eb6ddb5fe656a61f2a7d6452251457ee`.
Its all-target check passed in 24.980s; strict Clippy failed in 23.156s on one
needless borrow in a new archive test. Session66513 closed without running tests.
Source2 removes only that borrow; rustfmt made no additional change. Production
behavior and all test assertions are unchanged. Originals and exact one-line diff:
`/private/tmp/kegari-v0-declared-observation-source1-lint-correction`.
Source2 check passed in 0.491s, Clippy in 3.468s, nine protocol tests in 45.294s,
one boundary test in 0.219s, and four complete compatibility targets in 65.469s
(32 protocol, 29 evaluation, 15 report, 41 worker). Formatting passed in 1.533s
and Plan in 35.615s. The full log SHA-256 is
`8f91abe3dc474ed856e0206b650995414752d9baf7d74de2f4ff5ca604cd3e92`.
The exact serial commands and immutable results use the
`/private/tmp/kegari-v0-declared-observation-` prefix. The independent source/test
inventory confirmed all original assertions and the one-line lint correction.

Both source reviews found no material defect within their static scope. Protocol
review SHA-256: `266ec493fb7b165262783814710d8c16dbf7a32c6089bece4e114dbe797dfb2f`.
Ordinary workers never enable raw capture. Declared Independent trials retain
bounded protected records; reports contain references/digests. Capture stays
explicitly incomplete because bootstrap and transport-tail coverage are missing.
No scorer, fixture, tool grant or qualification changes through these diagnostics.
Operator documentation explains both optional observations and their limits.
A separate TEMP draft addresses bootstrap, matched responses and actual reader
EOF while retaining incomplete capture; it is uninstalled and unverified.
The same-owner broker read/effect inventory design passed independent static
review and is being implemented in a separate TEMP draft. Missing receipts stay
unknown, and acknowledgements do not establish remote-content truth. Neither
future draft is part of this tested checkpoint. Descendant containment remains
an actual code gap requiring a permitted ownership mechanism; the existing
`process_tree_unverified` refusal and native prerequisites remain unchanged.

The Control increment is committed as
`1e14788bddababfcd065cdfc9c7ef5dc575044d0` —
`feat(kegari-v0): retain indexed parent effect history`.
All12 files and683 tested inputs matched HEAD; the tree was clean before the
observation installation. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-control-index-commit-verification.json`.
The following Control checkpoint is historical; its commit is recorded above.

Current Control source3 is locally verified:917 passed, zero failed and two
required native ignores across39 groups in814.456s. All38 new names passed once in
the full --no-fail-fast log. All-target check, strict Clippy,42 focused results,
37 compatibility tests, formatting and the56-slot dispatch-disabled Plan passed.
All683 source inputs remained unchanged at
`sha256:e6ae6dea17e4c78d1d112ef0e9bde39123190e17b3c3c930860141ed86c7185f`.
Session59032 is closed. The independently reviewed v4 finalizer completed:
inventory passed0.033s, diff passed0.014s, and the collector verified all16
invocations and retained both earlier failures. Seven reviewed coverage rows gained
eight partial entries; exactly four not_assessed statuses became partial. No row
became passed. Reports are [local validation](validation/LOCAL-CONTROL-EFFECT-INDEX.md)
and [machine evidence](validation/control-index-checks.json). The original collector
bundle is preserved; the machine report adds five separately hashed later reviews
without changing observed results. The authorized local commit is recorded above.
The Control reservation/reconciliation mechanism and paired-history schema form a
separate coherent change from the prior retirement producer; preserve its own
validation checkpoint in a distinct commit rather than consolidating the two.
Native/live/human qualification is incomplete.

The unexecuted finalizers retained three independent findings: partial writes could
escape rollback; one cleanup error could prevent other cleanup; and a late stdout
error could leave COMPLETE despite rollback. V4 uses flushed atomic ledger
replacement, complete exclusive report publication, independent owned cleanup and
a publication boundary before final checkpoint/stdout. All findings were rereviewed
as resolved before execution. No multi-file crash atomicity is claimed. Finalizer:
`/private/tmp/kegari-v0-control-index-finalize-3-v4.py` (SHA-256
`06de93bb6ed8d7b6aa61966daa478f5ddf518a2b8b98b833047883549231ae17`).

The initial reviewed Control owner-effect index v5 and paired parent history
consumer v1 installation changed eight files (four new), after the verified
parent-retirement commit. Its683 source/test/asset inputs were frozen at
`sha256:81bce073afb21f17c3cb6b9ca6961714bbe89e9629838a7b3b6432fa47f40ea5`.
All exact original and layered draft hashes matched before installation; originals
and manifest are at `/private/tmp/kegari-v0-control-index-installed-source1`.
The serial pipeline `/private/tmp/kegari-v0-control-index-checks-1.py` is starting
all-target check, strict Clippy, the three focused filters, four Control/runtime
compatibility targets, formatting, full `--no-fail-fast` suite and the disabled Plan.
Source1 all-target check passed23.498s; strict Clippy failed16.631s on the
240-byte ParentEffectHistory variant. Session42887 closed without running tests.
Source2 boxes only that payload and adjusts its constructor/comparison uses;
serialized fields and test assertions remain unchanged. Exact original bytes and
five-line correction are at `/private/tmp/kegari-v0-control-index-source1-lint-correction`.
Its683 frozen inputs hash to
`sha256:6bc3de92d1c0477b07896748ee0b98a8bd38a91e367c7e094df039e89a1c94bb`.
Source2 check passed14.330s and strict Clippy24.230s. Its index filter passed34
and failed one in46.427s: the new deletion/substitution fixture restored a receipt
whose callback result was Null; ordinary Option<Value> readback makes that None,
so the unchanged reader correctly refuses its missing terminal observation.
Session17882 closed; no later source2 checks ran. Independent review confirmed
the diagnosis. Source3 changes only that callback to explicit component result
JSON; all assertions and production guards are unchanged. Formatting made no
additional changes. Exact originals/diff are at
`/private/tmp/kegari-v0-control-index-source2-fixture-correction`.
Its683 inputs hash to
`sha256:e6ae6dea17e4c78d1d112ef0e9bde39123190e17b3c3c930860141ed86c7185f`.
Source3 check passed14.354s and strict Clippy15.279s. The index filter passed35
in16.851s, identity passed1 in0.097s and parent passed6 in0.145s. All38 exact new
qualified names passed once across the42 focused results (four are existing tests).
All snapshots/toolchains are unchanged. The four compatibility targets passed37
tests with no failures/ignores/filtering in42.948s; formatting passed1.473s.
The full source3 suite passed917/0failed/2required native ignored across39 groups
in814.456s. Plan passed32.197s with56 slots and dispatch false. All38 exact new
names passed once in the full log; source/toolchain stayed unchanged. Session59032
is closed. Source1/2 failures remain retained. Native qualification is incomplete.
The boundary/protocol observation work remains separate, uninstalled TEMP work.
Its explicit declared-evaluation opt-in is off for ordinary workers; the private
trial declaration binds suite/fixture/trial/operation identities. Raw bounded
arguments/messages stay in protected evidence; reports contain references/digests.
Static review found bootstrap DenyTools and unread trailing-frame boundaries, so
capture must remain incomplete without actual full channel drain/EOF evidence.
Processed callback counts cannot prove absence. No scorer, fixture or qualification
is changed by that draft; independent review is pending its immutable handoff.

The parent-retirement increment is committed as
`cf6b93474835710be4fba7691c6f43934a7f9a7f` —
`feat(kegari-v0): retain authenticated parent-run retirement history`.
All25 intended files and679 tested inputs matched HEAD, with a clean tree before
the next increment. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-parent-retirement-commit-verification.json`.
A final independent artifact audit confirmed the raw totals, exact source and
partial coverage without material inconsistency. Its review is at
`/private/tmp/kegari-parent-retirement-final-artifact-independent-review-v1/REVIEW.md`
(SHA-256 `dc80aae70a282bef62b16c6974412d555350b31b93292c4dd339dc15c89558b0`). Historical checkpoint follows;
its uninstalled/pending statements describe the state before the new increment.

Current parent-retirement source8 is locally verified: 879 passed, zero failed,
two required native checks ignored across 39 groups (716.047s). All-target check,
strict Clippy, formatting, all 21 delivery tests, inventory, diff and the
56-slot dispatch-disabled Plan passed. All 679 tested inputs remained unchanged.
The final collector verified 38 invocations, preserved every earlier failure,
and installed only eight reviewed partial coverage rows with eleven added entries.
Reports: [local validation](validation/LOCAL-PARENT-RETIREMENT.md) and
[machine evidence](validation/parent-retirement-checks.json).
Session75488 is closed; no validation process remains running. This increment
is committed as recorded above. Native/live/human qualification is incomplete.
The reviewed Control membership index/consumer and authority-boundary observer
remain uninstalled TEMP work; they contribute no evidence to this checkpoint.

The verified native evaluator checkpoint is
`ddc44eaf35d3865640c8ba615c3d0877292c917a` —
`feat(kegari-v0): run explicit owned native evaluation trials`.
All 61 changed files and 672 tested inputs matched the commit; the working tree
was clean afterward. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-native-evaluation-commit-verification.json`.

Parent-retirement source1 is now installed from the independently reviewed
15-file composition, manifest
`59a386b092e23e6b31c593fc77f29e24ae4b1f0d31a1ca932505b917bd24aae0`.
Exact current bases/new paths and copied bytes were verified. Originals are at
`/private/tmp/kegari-v0-parent-retirement-installed-source1`. Three clean merges
preserve all native evaluator changes; two separate reviewed corrections validate
ordered artifact bytes and hash/decode each journal event from the same read.
Source1 all-target compile failed in 14.677s: the carry-history import pointed
one module too high and three new test vectors inferred unsized strings. Source2
corrects the import path and adds explicit Vec<String> types; production behavior
and assertions are unchanged. Original bytes and exact four-line correction are
retained at `/private/tmp/kegari-v0-parent-retirement-source1-compile-correction`.
All-target check then passed in 25.492s. Sessions86718 and16454 are closed.
The 678 current source2 inputs are at
`sha256:82931aec917dc5015df556a790998b1038b7e18af916a832367a9d5a9a190271`.
Source2 strict Clippy failed in 20.451s on a needless clone in a one-element test
slice. Source3 uses slice::from_ref; all-target check passed in 13.891s and strict
Clippy in 18.964s. Three origin regressions passed in 40.411s. The retirement
filter passed ten tests and failed one protected-file inventory test in 12.539s.
That test exposed a production mismatch: both new history modules requested16MiB
reads from PrivateDir, whose existing maximum is8MiB. The rejection occurred even
for small or absent files. Source4 lowers both call-site limits to8MiB, preserving
all storage checks, and formats the new test assertion. Original source3 bytes,
runner and exact diff are at
`/private/tmp/kegari-v0-parent-retirement-source3-bound-correction`.
An independent bounds audit found no further incompatible bound in either module
or its adjacent chunk/journal paths. Sessions47919 and70952 are closed; no full
source3 run occurred. Source4 has678 inputs at
`sha256:6f46bf53dd1a8ded600f8a00b29d952cf63f1b074a8a9b927532a3eef3b867d7`.
Source4 all-target check passed in14.036s, strict Clippy in14.943s, all3 origin
regressions in7.964s and all11 retirement-filter tests in12.451s. The nine new
regressions are included in those filters; five existing related tests also ran.
Formatting passed in1.446s. Full source4 failed after353 library passes and one
failure in450.256s (unchanged source digest). The existing corrected runtime
assembly fixture failed during integrated_worker_assignment: new provenance
capture requires the delta's merge receipt, but its synthetic upstream fixture
hard-coded merge_request818 without producing that receipt. Production validation
must remain strict; a genuine Control/provider-fixture effect correction is being
prepared and independently reviewed. The full failure log and result remain at
`/private/tmp/kegari-v0-parent-retirement-full-4{.log,-result.json}`; log SHA-256
`b015309204b29c3bc5160cf8f2267c79607dc719a4a966d5f08283734ed8ea32`.
Session67031 is closed. Later full-suite groups and Plan did not run. The report
collector correctly refuses qualification; no current full-suite or native
qualification is claimed.
Source5 installs only the two independently reviewed fixture corrections from
`/private/tmp/kegari-parent-retirement-merge-fixture-correction-v1`, manifest
`2eb5064fc0e1c9e58b1dfd078f669b7442ddd68e8d7004d694c2cfd16dd3cf75`.
The real typed merge broker/Control path supplies the actual receipt through the
synthetic provider; upstream review/check/staging qualification stays synthetic.
The existing corrected assignment test additionally deletes that receipt, restores
its exact envelope, and requires refusal plus unchanged Control inventory before
its original positive assertions. No production guard or test assertion was weakened.
Originals, exact diff and coordinator review are retained at
`/private/tmp/kegari-v0-parent-retirement-source4-fixture-correction`.
All678 source5 inputs hash to
`sha256:5e0cbd876b4b666c723b4ea9fdd4b4c0b94a2a5a49312b0f217e7dbc4783fa4b`.
All-target compilation passed15.664s and strict Clippy16.461s. All three successor
scenarios passed250.528s, all three origin tests passed0.343s, all eleven retirement
filter tests passed12.975s, and formatting passed1.542s. The full suite failed after353 library passes/one failure in415.778s on unchanged
source5. The corrected assembly regression passed. The chained successor failed
at release_wire.rs2006 during actual capture_delta: owned child OS identity could
not be established. Its focused run had passed; this is not accepted as a lucky
rerun. Full5 log SHA-256 is
`2ab5384bd433baf02e2bc8129c777744b6e3782746b5c77355504631841af4b4`.
Session69270 is closed. Later integration groups and Plan did not run. Root and
an independent agent are tracing the existing immediate identity/terminal fallback.
The one-shot retained-child scan can lack an exact identity; the existing Git
fallback accepts only immediately observed actual terminal/group closure. A bounded
wait on the same unreaped owned child is under review; exact OS failure cause is
not established by the generic error. No denied native/global probe was retried,
production guard weakened or unmodified lucky rerun started at that diagnosis.
Source6 now integrates the independently reviewed three-file startupv3 correction
from `/private/tmp/kegari-owned-child-startup-exit-v3`, manifest
`f3e417a09a9ad8a766e5bc9773ba25732474cef9e727272fcbcae58b9832598b`.
It waits only for the same unreaped owned Child's actual terminal/group closure,
at most100ms capped by original command/operation deadlines, after identity
unavailability. It retries no ProcessProbe and sends no stdin during uncertainty.
Other capture errors retain their existing immediate fallback. A review finding
was corrected before integration: the command clock begins before startup and is
also enforced at entry/start/after ownership lock/immediately before spawn. Seven
actual-owned-child component tests distinguish missed capture, timeout/cancellation,
no-spawn expiry, post-spawn deadline and withheld input without a Running identity.
Native boot identity and detached containment remain unqualified. Originalsource5,
exactpatch and artifact provenance are retained at
`/private/tmp/kegari-v0-parent-retirement-source5-startup-correction`.
All679 source6 inputs are
`sha256:98fb015e79d1af187df46edc39a67585499ba3b83e03b0995bebb06ef977c9c2`.
Source6 all-target check passed15.781s; strict Clippy failed9.319s on an
unnecessary mut at git_startup_exit_tests.rs133. No later source6 tests ran.
Session90899 is closed. Source7 removes only that mut; original bytes and exact
diff are at `/private/tmp/kegari-v0-parent-retirement-source6-lint-correction`.
Its679 inputs hash to
`sha256:fb7ed9a0872dac4f823c704b4712f05c280a8aa934c659aaacf9e5f785e77d89`.
Source7 all-target check passed14.411s, strict Clippy15.415s, all seven startup
regressions24.881s, and all46 Git/process tests across three complete groups
108.964s with no ignores. Origin passed3 in0.340s; retirement passed11 in12.702s;
formatting passed1.470s. All recorded source/toolchain snapshots are unchanged.
Full source7 failed in491.551s on unchanged source/toolchain. All361 library
tests passed, including all three successor fixtures. Later tests/delivery.rs
passed20 and failed parent_slice_uses_actual_production_and_preserves_dirty_operator_checkout
at1785: new origin capture requires the exact workspace namespace, but the
fixture constructs its protected delivery directory with the generic leaf records.
Root and independent review are comparing that fixture to production construction;
namespace validation remains strict. Full7 log SHA-256
`8e3791f47f0045663904a61d6632fdffc469b271f531bd9f0ca5853495a30326`.
Session42895 is closed; later groups and Plan did not run. No source7 collector,
finalizer, coverage update or final report ran. Existing source7 success templates
are now obsolete and remain contingent/unpublished.
Source8 installs only the reviewed delivery fixture namespace correction from
`/private/tmp/kegari-parent-retirement-workspace-fixture-correction-v1`, manifest
`eff38a2b48fa6289ce4caeb9672c5f8ca13b46256ff314a4d0bcdc6435b86fdf`.
The constructor now uses PrivateDir.child(actual workspace UUID), exactly as
production RuntimeDelivery does. All dirty-index/source/candidate/admission
assertions and production guards are unchanged. Root and independent caller audit
found no other affected integration caller. Original bytes, patch and reviews are
at `/private/tmp/kegari-v0-parent-retirement-source7-fixture-correction`.
Its679 inputs hash to
`sha256:581ef09275d7b370bbfc4dba4382e1e69cca75f3cb112fc5ffd4282193eaecb1`;
independent comparison verified only tests/delivery.rs changed from source7.
Source8 check passed in 1.199s, strict Clippy in 0.828s, all 21 delivery
integration tests in 19.664s, and formatting in 1.422s. The previous namespace
failure now passes with its original assertions. Session75488 completed: the full
suite passed 879 tests with zero failures and two required native ignores across
39 groups in716.047s; the 56-slot dispatch-disabled Plan passed in31.104s.
All sixteen new tests, three successor fixtures and the delivery regression passed
on source8. Unchanged focused source7 passes remain historical evidence.
Final source8 remains frozen; its local report is now published. Native, live
integration and human qualification remain incomplete. The evidence
collector's exact auxiliary key requirement now binds SPEC, contract_inventory.py
and coverage.json. Independent source5 evidence review found no remaining material
issue within its scope; the final source8 success claims now rest on actual full
results and independently reviewed source8 evidence collection.
The source5 finalizer remains unexecuted and obsolete. Source7 collector/render
drafts are independently reviewed at
`/private/tmp/kegari-parent-retirement-evidence-source7-independent-review-v1`;
no additional material finding was confirmed. Root prepared finalizer7, whose
independent review found that a later failure could leave prematurely finalized
coverage. The unexecuted corrected
`/private/tmp/kegari-v0-parent-retirement-finalize-7-v2.py` validates exact commands,
snapshots/logs and rendering before writes, then restores its own ledger and
preserves unpublished outputs if a subsequent step fails. Independent v2 rereview
confirmed the original finding resolved; crash-atomic publication is not claimed.
Source8's separately reviewed guarded finalizer subsequently completed. It
installed the final report, machine evidence and only the eight approved partial
coverage rows; inventory and diff checks passed. All38 invocation records and
all earlier failures remain archived. The producer records actual assignment, preparation and
original joined parent-run provenance; it cannot reconstruct worker/exit/current
authority from historical JSON. Its retained receipt-scan
schema explicitly cannot prove completeness before first capture. A separate
bounded Control index implementation is held at
`/private/tmp/kegari-control-owner-effect-index-v1`; it remains uninstalled.
Independent review found an effect-owner-field removal could bypass membership
validation at async launch/ACK (including an absent-marker cold downgrade), and
a late effect settlement after a retirement-save failure could prevent cleanup
retry. Both findings are recorded in
`/private/tmp/kegari-control-owner-effect-index-independent-review-v1/REVIEW.md`;
v2 corrections are held at `/private/tmp/kegari-control-owner-effect-index-v2`,
manifest `35d713adf0393e624870c6169b5e1582602a9990606180df8299797b9d7e6b0d`.
Root read both complete layers and verified all bases; independent rereview found
the earlier two defects corrected but a further same-host remint seam: deleting a
known owner's receipt/index files could allow its UUID to be readmitted, replacing
retained anchors. A v3 pre-write known-identity guard corrected that remint. Its rereview found an
error path narrowed existing resource holds; v4 unions all prior targets across
four error-only sites and tests actual pending-A/already-active-B exclusion after
exact receipt restoration without ACK. V5 fixes two test usize→u64 arguments.
Latest held composition is `/private/tmp/kegari-control-owner-effect-index-v5`,
manifest `e867c1ac51b8f8e9dd677f8a5a728ea6230365d947d82f3560c8286127fd058d`.
All30 authored tests remain unrun. Root read every layer; independent v5 review
found no remaining material issue in scope. No Control-index layer is installed
or Cargo-tested.
The boundary observer's12-file implementation is separately held at
`/private/tmp/kegari-independent-boundary-observation-v1`, manifest
`026b8e7a77badc46f3743e9e6a4fb3da7d4285e876f68093f97baac8dbcd8bff`;
it is uninstalled; independent review found no additional material defect, and
compilation/execution remain pending. The parent
report/collector independent review is retained at
`/private/tmp/kegari-parent-retirement-evidence-independent-review-v1`. Its two
unsupported lifecycle coverage mappings (§8.3 retention and §11.5 preservation)
remain unchanged from the base ledger. The corrected proposal at
`/private/tmp/kegari-parent-retirement-coverage-correction-v2` (eight rows, eleven
new partial entries) is now installed and verified. Next: commit this verified
parent-retirement increment, then integrate and validate the held Control index
and consumer. A paired Control-index consumer design
was read in full at
`/private/tmp/kegari-parent-retirement-effect-index-consumer-design-v1/PLAN.md`.
The independent design review found no material gap. The six-file implementation
is held at `/private/tmp/kegari-parent-retirement-effect-index-consumer-v1`,
manifest `1771532aad9e71bdd15e43a13cb60411f270e50498138478633dcf40e5f5ebc2`.
Root read the full patch and verified all held/current bases: one serialized batch,
strict indexed membership, explicit legacy/mixed provenance, unchanged unresolved
owner/join/process guards, and preservation of existing schema-1 signed bytes.
Eight authored component/schema tests are unrun. Independent implementation review
found no additional material issue at
`/private/tmp/kegari-parent-retirement-effect-index-consumer-independent-review-v1`.
The source8 collector/render and guarded finalizer also passed separate static
reviews; finalization completed only after full8 and Plan passed with unchanged
source/toolchain identities. No Control index or consumer is installed or
contributes current evidence. The PR-attack observation design at
`/private/tmp/kegari-native-pr-attack-observation-design-v1/DESIGN.md` was read in
full; independent review and a first actual authority-boundary observation slice
are proceeding in TEMP. Scorers, fixtures, grader versions and native qualification
are unchanged.

Native source14 installs the reviewed native evaluator caller, literal PR text,
exact trial subjects, owned failure cleanup, grading-error evidence, first native
E05 context fixture, source-preparation tool and operator guidance. Its 672 build
inputs are unchanged at
`sha256:6f962a238f28a5c05a6fad1fc4f90226d4d638512fee109b782114bb6e2e0f8e`.
All-target check passed in 14.425s, strict Clippy in 14.666s, and all 13 Python
source-tool tests in 0.780s. Those tests include actual disposable local Git data
preparation; they do not qualify native execution. Retained source preparation
also passed in 0.251s and reproduced the base/head below. Output is owned local
fixture data at `/private/tmp/kegari-v0-native-evaluation-source-output-14/prepared`.
The final Rust suite passed **863 tests, zero failures and two required native
ignores across 39 groups** in 835.770s. Formatting, both dispatch-disabled Plans,
contract inventory and diff checks passed. Session72931 is closed; no owned check
process remains. All 35 invocation artifacts, source/tool/log identities and
retained source-preparation bytes/modes were verified. See
[the native evaluator report](validation/LOCAL-NATIVE-EVALUATION.md) and
[machine evidence](validation/native-evaluation-checks.json), SHA-256
`97ea13128dff8d31a766f9546b1617926f33c94ee276dad87d3c9a446356b151`.
The ledger appends partial local evidence to 18 clauses and eight named items;
459 clauses remain not_assessed/37 partial, and 121 named items
not_assessed/15 partial. No requirement is passed. Coverage SHA-256:
`71790c6b2e3f6445b093c0a4d47f3c61fc7f654e03851a32b4e32ed3868a1c5b`.
Actual-agent/provider/native containment and human qualification remain incomplete.
The native evaluator increment was committed separately from prior carry
recovery because they affect distinct runtime paths. Its exact verification is
recorded above; actual-agent/native and human qualification remain incomplete.

Source13 only rustfmt-formatted the exact-TrialKey test assertion; its intentional
source transition from source12 and original bytes remain retained. Source12
strict Clippy and 49 focused evaluation tests passed on their own exact inputs.
Earlier source10 compile errors (two test-only PrivateDir::clone calls) and
source11 lint errors are preserved with the corrections. Root reopens the actual
protected test directory, derives Default, keeps the existing boxed transfer on
error, simplifies equivalent borrowing/conditions and reorders a helper. No lint
suppression, weakened assertion or replacement authority was used.
The exact reviewed owned-return composition is
`/private/tmp/kegari-native-owned-return-on-source9-v2`, manifest
`49016fbd334d030a67d7106b6dd09bce2fbb5c0528ba7173eefd252bbbc07ed0`.
Source-tool v2 fixes bounded directory enumeration and protected output ownership;
independent review found no remaining material defect in those corrections.
Six tool/docs integration paths and originals are retained at
`/private/tmp/kegari-v0-native-evaluation-installed-source14`.
Coverage/report integration is complete. The evidence collector
independently binds exact final commands, log paths, source identity and both
required native-ignore names after review identified filename-only classification.
No claim is derived from a label alone. The next-fixture data proposal is held at
`/private/tmp/kegari-native-next-fixtures-proposal-v1`, manifest
`9ec55d56f3858bfeb1f7da4c99645c4400f582b297588887e2a5eb8b437923f3`.
`independent_eligible` needs coordination and `clean_review` needs product review;
neither is relabeled. PR-attack migration preserves three currently unavailable
host-observation assertions and has no invented base/head OIDs or runnable manifest.
At that earlier native checkpoint, the separate15-file retired-parent producer
composition was uninstalled in TEMP. It is now installed as recorded above, and
still cannot prove receipt-set completeness from retained receipt scans alone.

The 28-file native evaluator source1 is installed from the reviewed caller-v4,
subject-v2 and terminal-report layers, rebased on current main. Exact composition:
`/private/tmp/kegari-native-evaluation-caller-composed-v5`, manifest
`5289c0d7765868493717d4cfaaf4169cde9d14bd2bbdd5a74e7bad82644952c8`.
All bases/new paths and installed bytes were verified; originals are retained at
`/private/tmp/kegari-v0-native-evaluation-installed-source1`. Both clean merges
preserve current cold recovery, exact per-trial subjects and terminal classification.
Initial all-target check passed in 24.809s on 662 unchanged inputs at
`sha256:4e0d2ee0c9a33522d66110a63ed7954e486f5d7ada23b976d3604777f5f30ac2`.
Formatting changed no bytes. Strict Clippy2 failed on a nested if and a
one-element loop; source3 simplifies those without changing their conditions or
responses. Original source and failed invocation remain retained. Session59176
and session18639 are closed. Source3 strict Clippy passed in 24.649s; focused
evaluation tests then reported 36 passed/1 failed because the new report test used
a symlinked macOS temporary path. Source4 canonicalizes only that actual owned
test directory, preserving storage protection and every assertion. Session38918
is closed. Source4 focused library evaluation passed all 37 tests in 13.268s.
Source5 adds two independently reviewed CLI/HTTP boundary regressions; all
136 tests across 7 related API/CLI/app/config/evaluation integration groups
passed in 68.952s. Session30344 and session29780 are closed. Actual
ownership through preparation/unused-trial failure remains a separate in-flight
TEMP correction. No native evaluator is
qualified and compatible native fixtures remain incomplete.

The reviewed ten-file PR-text layer is now installed as source6 from
`/private/tmp/kegari-native-independent-pr-text-on-source5`, manifest
`d85e60668cd2fb24355a78391d6233b86949e0bcab411ba15543c6dfc19787ab`.
All exact current bases and bytes were verified. The two clean merges preserve
actual evaluator transfer gates and the provider lint correction. Metadata and
literal text come from the same bounded wire response; full current/active/source
comparison binds text without changing review/work identity. General status omits
raw text; protected worker claim provenance also retains the gate text. Root read
the complete patch; independent review found no additional material defect
(`/private/tmp/kegari-native-independent-pr-text-independent-review/REVIEW.md`,
SHA-256 `9399d35663e001547dc5a447ca361ca997a0de055a176f1270a7b05ebb6997b7`).
Source6 all-target check passed in 23.234s, strict Clippy in 24.814s and all
98 provider/reconcile/worker tests across three groups in 59.254s. Its focused
library evaluation group passed 38 tests in 42.548s, and the retained
import-history regression passed in 20.331s. Sessions48446,80599,32742
are closed. Exact invocation/log/source identities are retained under
`/private/tmp/kegari-v0-native-evaluation-{check,clippy,text}-6-*`.
Scoring-error loss of already-collected report evidence is confirmed separately
and under correction; no false success or qualification is claimed.

Evaluator source6 had 662 build inputs at
`sha256:7fbe90aefd480e2c012bb7a7906fc32f8f72b4add9ae4332e0bcfc42255d6cdb`.
All-target compile, strict Clippy and focused results retain their exact earlier
source identities. No current full-suite or native qualification is claimed.
The four-file grading-evidence layer is now installed as source7 after full
root review and exact base/hash verification. Independent review found no further
material production defect; its exact report is
`/private/tmp/kegari-native-evaluation-grading-evidence-independent-review/REVIEW.md`
(`c9381deafa10f4ec9e5026c112ec126e065c7f483995b5ab7284c1fc26df51bc`).
Source7 all-target check passed in 21.839s, strict Clippy in 22.825s and 42
library evaluation tests in 35.521s. Integration checks passed 29 evaluation
tests and 12 report tests, with one new report test failing because it selected
comparison row zero instead of its exact trial key. The original failure is
retained. Source8 corrects only this test lookup; all 13 report tests then passed
in 3.338s. Sessions85815 and95686 are closed.
Manifest `ae9c391ebcaa09b724953565cf7301593950ec595e57acee7f104f13abbf440e`;
663 inputs at
`sha256:0e7e5dfac055469156da19c8b8b41de550579c054b2fd2bce5bd5f9fbbcaf5f8`.
Current source8 retains 663 inputs at
`sha256:922a318a5e25c18417618cf6b3de178d611f99c4a9cd29ab58ff205796b8d9f5`.
It retains exact scorer errors and observed effects without substituting a
successful grade or changing execution/cleanup authority. Original inputs are at
`/private/tmp/kegari-v0-native-evaluation-installed-source7`.
The original owned-return TEMP layer was held while root found two compile defects
(signature and missing original preparation reference); separate overlays correct
both. Independent review then found receipt-capture retry and cross-owner drain
liveness gaps. The shared exact admission encoder and independent drain loop
resolve those findings in installed source10. A third compile correction removes
a nonexistent Receipt.created_at reference; the actual Permit.registered_at
remains bound through the immutable payload digest. Original drafts are retained.
The retired-parent producer is also held and explicitly records that retained
receipt scans cannot prove completeness without an owner-emitted receipt index.

Root prepared the first native E05 fixture at
`/private/tmp/kegari-native-independent-fixture-draft-v1`, manifest
`b165ab63a368cc4a0a8a3780af55e079ac62949494d1a586b6df74c858005746`.
Its real owned local Git base is `f27eea6d92b3a6e7cb6973c5e389fedd225c31d5`
(`test(evaluation): seed inclusive boundary review base`), and head is
`2b2b21aca2c287e5f89a35100fb3aa5c3fec1d06`
(`test(evaluation): introduce boundary defect for independent review`).
These are disposable fixture commits, with signed human attribution and no
consolidation or remote access. Exact commands/objects/modes are retained in
SOURCE-EVIDENCE.json. The suite-v4 draft preserves 28 variants/56
trials and an unresolved profile; exact production Plan passed in 32.082s with
dispatch_authorized=false. All fixture/scorer inputs were unchanged. Session67298
is closed. The five fixture files and two-file component test bridge are now
installed as source9 after exact current-base/hash verification; originals are at
`/private/tmp/kegari-v0-native-evaluation-installed-source9`. The bridge reads the
required native scope declaration and exercises actual fixture capture and whole
context matching with explicitly data-only host values. It creates no worker or
source capability. Independent bridge review found no further material defect.
The actual captured-context component passed in 14.444s and all 42
evaluation/report integration tests passed in 13.074s. Session99121 is closed.
Source9 has 664 inputs at
`sha256:e79fbe3d8a4665cf81f622b21d759b3f1ee0a189930f3cdc1dec7a0a06f083be`.
Actual native context/source admission and reusable source-preparation tooling
remain incomplete. The PR-text production path is installed and tested above.
Fixture/source data grants no runtime capability.

The completed-current carry checkpoint is
`2770aa5c43bc24343a2344ce687306928f14fd16` —
`feat(kegari-v0): recover completed carried acceptance from exact history`.
All 23 committed files and 653 tested inputs matched; the tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-completed-current-carry-commit-verification.json`.

Completed-current carry recovery source3 passed **825 tests, zero
failures and 2 required native ignores across 39 groups**
in 843.114s. All-target check, strict Clippy,
formatting, dispatch-disabled Plan, inventory and diff checks passed. Six new
recovery/inventory/import-history component tests also passed separately and are
included in the full total. All 653 inputs stayed unchanged at
`sha256:d0288da1dcd547c8a670373630ebad3542c8ff2aed57b63576e5ddb6d62db46b`.
Session40448 is closed. All 15 invocation artifacts, source/tool/log
identities and the unchanged SPEC were verified. See
[the completed-carry recovery report](validation/LOCAL-COMPLETED-CARRY-RECOVERY.md)
and [machine evidence](validation/completed-current-carry-checks.json).

The real live writer retains exact recovery selections; a canonical protected
reader yields historical provenance, followed by actual scoped current metadata
reobservation and final serialized installation. Additional production integration
proofs retain the original closed owner/import/source and predecessor closure.
No worker authority or joined cache is reconstructed. One initial private-sibling
compile error was corrected by ancestor-only visibility with unchanged behavior;
earlier failures/results are retained. The final inventory formatting failure
was corrected without semantic changes; inventory4 and diff4 passed afterward. Root and independent complete-layer reviews
found no remaining material defect in this scope. Native cold carry, visual
hydration and cold worker reconstruction remain unqualified or incomplete.

The independently reviewed ledger now marks five carry clauses and V0-T17 partial,
linking committed deterministic evidence and explicit native not_run. It contains
465 not_assessed/31 partial clauses and 126 not_assessed/10 partial named items;
no requirement is marked passed. Coverage hash:
`e7ff810b945b9008a405ed835be980b833aad1bf82b433ca0f786419990f016c`.

Next useful work is to verify the installed retired parent-run producer, then
complete the owner-emitted receipt membership and remaining native fixture/observation paths.
The exact 15-file retired composition is
`/private/tmp/kegari-retired-parent-run-on-native-source13-v1`, manifest
`59a386b092e23e6b31c593fc77f29e24ae4b1f0d31a1ca932505b917bd24aae0`.
Root and independent review verified all three clean merges preserve evaluator
ownership, exact identity, completion retries and existing cold-carry behavior.
Its source bases match source14; it is now installed as parent-retirement source1
and awaits compilation. Receipt
completeness still needs an actual dispatch-time effect index; retained scans
alone cannot supply it. Native model/provider/containment execution and human
calibration remain incomplete. Root owns repository changes/checks/commits;
agents work only in separate TEMP layers. Earlier caller-v4 and terminal-report
artifacts remain preserved at their recorded TEMP paths; source14 supersedes
those intermediate uninstalled checkpoints.

The verified carried-acceptance checkpoint is
`7b933f2cb5d11f149846ba81628ef5c5cc6c1501` —
`feat(kegari-v0): reassemble carried acceptance on known baselines`.
All 53 committed files and 648 tested inputs matched; the tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-carried-acceptance-commit-verification.json`.

Carried-acceptance source6 passed **819 tests, zero failures and
2 required native ignores across 39 groups** in
771.633s. All-target check, strict Clippy,
formatting, dispatch-disabled Plan, contract inventory and diff checks passed.
The affected release-origin test also passed separately. All 648
build inputs remained unchanged at
`sha256:741b805e1a0209cb2994bf66190e7a1aca4648978bd5e0f527abbe1ab4e4184b`.
Session36561 is closed. All 22 invocation artifacts, including earlier
compile/lint and regression failures, were checked against exact source/log/tool
identities. See [the carried-acceptance report](validation/LOCAL-CARRIED-ACCEPTANCE.md)
and [machine evidence](validation/carried-acceptance-checks.json).

The 50-file root composition and narrow integration corrections implement known
baseline reassembly, owned finite object acquisition, explicit/same-grant retries,
carried empty-slice no-release and actual active owner/E2E child gate retention.
The granted inventory JSON digest defect was fixed while preserving legacy bytes.
Two stale test assumptions now assert the actual gate/namespace boundary and
unchanged protected effects. All failures and review corrections remain in the
report. Current native carry and release qualification remains incomplete.

Root owns all repository changes, checks and commits. Next useful work is the
independently reviewed completed-current carry recovery draft, then the concrete
native evaluator caller and remaining native fixture migration. The caller's
scorer/deadline, active lifecycle and actual-retirement corrections are composed
into 23 independently reviewed TEMP files at
`/private/tmp/kegari-native-evaluation-caller-composed-v4`, manifest
`231a86032d1c7fb376074b6cd05d5736e89781bef557458b067a98a353fe92be`.
They remain uninstalled/uncompiled. Independent review found repeated
trial identities also need distinct legitimate scheduling authority. Native
preparation failure closure and richer failed-trial observations remain under
review. Production integration history hydration and cold worker reassembly
are separate bounded follow-ups. No serialized history creates native authority.

The evidence ledger checkpoint is `0599cebb67ae3ab80340fa71fb95d7b7314f3d3d` —
`docs(kegari-v0): map verified recovery and trial evidence`. Both committed files
match and all 629 runtime inputs remained unchanged; tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-recovery-evidence-commit-verification.json`.

Evaluation terminal-finalization source1 passed **100 focused tests across six
groups**, all-target check, strict Clippy, formatting, inventory, diff and a
nonexecuting Plan. All 629 inputs stayed unchanged at
`sha256:95f47600a9a67c9b8f7399eb526a4949caa3f4ed08081b788478de3aa9ce554a`.
Sessions88617 and94425 are closed; no root Cargo process remains running. All ten
invocation artifacts and tool identities were verified. See
[the terminal-finalization report](validation/LOCAL-TRIAL-FINALIZATION.md) and
[machine evidence](validation/trial-finalization-checks.json). The four-file
change retains actual execution, checkpoint, hook and owned-exit evidence
independently of rubric outcome. Actual native finalization remains unqualified.
The verified local checkpoint is `b295c0fdc5e56cc402c043c391ed9b2710cc8938` —
`fix(kegari-v0): finalize failed trials from actual lifecycle evidence`. All seven
committed files and 629 tested inputs match, with a clean tree afterward. No
commits were consolidated, rewritten or pushed; this separate increment changes
evaluator lifecycle semantics. Verification:
`/private/tmp/kegari-v0-trial-finalization-commit-verification.json`.

Carry43, same-grant transient6, the test-module ordering correction and carried
no-release8 now compose cleanly into 48 TEMP Rust files at
`/private/tmp/kegari-carry-composed-on-finalization-source1-v5`, manifest
`17e3968c579bea9a531d2620966c96ecd6c1e30bf0ab061e16fa89ebbce3f581`.
The exact three-way merge preserves terminal-finalization runtime behavior.
The later active-grant8 correction is composed into 50 files at
`/private/tmp/kegari-carry-composed-on-finalization-source1-v7`, manifest
`8d4bddf3b7f661ade59b5866b238e3dbd6cf350af0252d01e67c9fed199cbc8b`.
It fixes the ordinary-admission lookup during active carry and release, including
outer publication selection. Independent review then found its exact-parent work
check wrongly rejects the actual E2E child's integrated proof. Codex owns a narrow
follow-up that must authorize the exact retained child; installation remains
pending. Partial v6 preserves a second TEMP base-layout script failure with no
repository source changes. The previous active8 remains immutable.
All copies parse; they are uninstalled/uncompiled pending the active-grant
observer correction and independent review. Partial v4 remains preserved after
a composition script assumed the wrong TEMP base layout; no product source was
changed by that failed script. Cold-current carry planning and the concrete
one-trial caller continue in separate TEMP assignments.

Paired recovery source6 passed **798 tests, zero failures and two required native
ignores across 39 groups** in 809.522s. All-target check, strict Clippy, formatting,
dispatch-disabled Plan, the 496-block/136-named inventory and diff checks passed.
All 629 tested inputs stayed unchanged at
`sha256:ad153d5922671337911480153f8d7d2230847eda3747dae5aae76de2feb7cb84`.
Session11140 is closed; no root Cargo process remains running. The collector
verified all 19 invocation artifacts, including compile/lint failures and older
source results, against their exact source/log/tool hashes. See
[the paired-recovery report](validation/LOCAL-PAIRED-RECOVERY.md) and
[machine evidence](validation/paired-recovery-checks.json).

This increment preserves original paired-repair review and implementation
budgets through actual metadata/model resume, completed-result consumption and
subsequent failed validation. Independent review corrected omitted grant and
receipt ancestry, repeated recursive traversal, recursive Arc destruction and
two direct-only completed-repair checks. Required native positive execution
remains unrun; no serialized reference manufactures a capability. Earlier
source1 compilation and source3 Clippy failures are retained in the report.
The verified local checkpoint is `8bce8ff5e2511412cbd2adfb95a4f5602a3f257a` —
`feat(kegari-v0): preserve paired repair recovery lineage`. All 49 committed
files and 629 tested inputs match; the tree was clean afterward. No commits were
consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-paired-recovery-commit-verification.json`. The separate
commit preserves the runtime-recovery boundary from the earlier evaluation grader.

The combined carry43 TEMP draft is held at
`/private/tmp/kegari-carry-composed-on-paired-source6-v3`, manifest
`a8ece743d274e104d4165a1c347744c8c59fb9e35366c75d3a63b868826fdf9d`.
Its same-grant transient retry and eight-file carried no-release overlays remain
separate, uninstalled and uncompiled. Carried no-release passed independent
bounded static review; its qualified native current-to-cold positive remains
unproven. The granted-parent observer has one confirmed active-drain regression:
it consults current ordinary admission after harmless workflow reload instead
of actual retained active authority with pinned configuration. Codex is preparing
a narrow correction; new work must retain strict current admission checks.
Config continues the concrete native one-trial caller in TEMP. No evaluator or
carry draft is included in the paired source's validation claim.

Mandatory actual-agent repeats, native/provider and containment qualification,
human calibration, cold/current carry and other recorded implementation gaps
remain incomplete. The coverage ledger now links six recovery/budget clauses to exact executed
component regressions and adds two scoped terminal-finalization references. These
remain partial: 470 of 496 clauses and 127 of 136 named requirements are not
assessed; 26 clauses and nine named requirements are partial. No full pass was
added. Referenced files and passed test names were checked against retained logs;
the inventory check passes. This documentation-only update occurred after both
source-bound validation sets, whose auxiliary hashes remain historical evidence.
The ledger/progress evidence reconciliation is ready as a separate documentation
checkpoint spanning the two verified increments; runtime source is unchanged.
The dated checkpoint narrative below records prior states, including work that
has since advanced; the current summary and linked source-bound reports govern.

Structured-review evaluation source1 passed96 focused tests (29 evaluation,
10 report/archive,39 worker,18 module), all-target check, strict Clippy, formatting,
496-block inventory, diff and dispatch-disabled Plan. Eight invocation artifacts
retain unchanged616 inputs and before/after Rust/Cargo1.99.1 identities:
`sha256:19ea8279293431fe4db5cb7c0cbdb162e2f0321062eb4d65e6a1e01fb52d0bfd`.
Session86937 is closed. The verified checkpoint is
`e55ee191ec027c9c8be8f60bccdf1e4f6dad4773` —
`feat(kegari-v0): grade recorded structured review outcomes`. All45 committed
files and616 tested inputs match, and the tree was clean afterward. No
consolidation, history rewrite or push occurred. Verification is retained at
`/private/tmp/kegari-v0-structured-review-commit-verification.json`. This
evaluation change remains separate from the preceding runtime admission fix
because it changes grader semantics and report provenance. The original precise-
URL false rejection was corrected in v2 and independently reviewed by Config.
No full crate suite or actual-agent trial is claimed for this increment. See
[the structured-review report](validation/LOCAL-STRUCTURED-REVIEW.md).

Paired recovery composition remains TEMP at
`/private/tmp/kegari-paired-recovery-composed-on-e55ee19-v3`:41 source files,
original bases and merge outputs preserved. All616 current source inputs and
composition bases still match clean e55ee19. The two original end-of-file test
conflicts retain every test and existing source4 fixture fix. Config v2
(manifest0e7a8c6abcf7a4d9e1559c0a972f7e03d83883ff08b7d96f184ae8ab20581a12)
and leaf v2 (70577a55932b9edf4a3dbda4b80a45133eb0399db35ffad4f1b04a44f8da1984)
merged without conflict; two newer guard literals gain only optional None fields.
The corrected actual native-grant closure passed independent static review.
The raw Control reader also now checks earlier repair/failed-validation ancestry,
but independent review found its passed-validation references still unchecked.
Control v3 now fixes that through a nonrecursive shared exact-reference reader;
manifest2ba06ed6b20825def53581991da5300b9949b51a72a24a26d8bf8fb4af5265e5
merged three files cleanly and includes a first-new-grant Initial source
regression. Original v1/v2 artifacts remain unchanged. Mandatory opaque maps
already rejected those missing records, so no native authority bypass is claimed.
A separate material traversal defect is confirmed: continued repair queues its
previous plan and calls that same plan recursively through unfinished work with
a fresh visited set, causing exponential repeated work and linear stack depth.
This affects the new grant/bridge collectors; source/joined paths are under audit.
Codex owns a shared bounded traversal correction before installation.
Rustfmt parsed the combined draft; it is not installed or compiled and no
native/data joined success is inferred. Carry's33-file vertical draft and its
11-file object-acquisition overlay passed independent bounded static review with
three confirmed corrections pending: successful preparation result shape,
active carried-release configuration pinning, and same-work reassembly markers
across grant namespaces. Object overlay manifest
bd64dbb9b06700cfbcc39dbc1ebe139338b02ad8cf20c8a3f8de235078165b42 is held at
`/private/tmp/kegari-carried-object-acquisition-overlay`; no Cargo or native
positive is claimed. Root's actual Git2.56 query compatibility fixture is retained
at `/private/tmp/kegari-carry-object-query-review-4wwn484k/results.json`
(sha43781a8373cbc9a3d9c26e940792a9e3141ce309a00c290614adb6371431b4cb):
present object exits0 with exact SHA; missing object exits1 silently; wrong type
exits1 with stderr. This tests CLI behavior only. No remote requests occurred.
Cold/current carry recovery, parent granted reassembly, carried no-release and
qualified integrated/visual/native positive behavior remain incomplete.
A concrete one-trial native evaluation caller design is
retained at `/private/tmp/kegari-native-evaluation-one-trial-design/DESIGN.md`;
it is not implemented and no live trial is authorized or running. Root traced
the failed-trial retirement prerequisite through actual evaluator/lifecycle/broker
and runtime code; findings and four exact inputs are retained under
`/private/tmp/kegari-native-evaluation-finalization-review`. A failed report is
not an actual joined capability, and no such capability was fabricated.

The isolated first-new-grant review-round omission fix passed all-target check,
strict Clippy,24 unique child-continuation/scheduler tests, formatting,496-block
inventory and diff checks. Source1 retains615 unchanged inputs at
`sha256:223772a0e6b513f7ce0aba2ecf456772f98957b8c72cee50417d5f220b00fabb`.
Sessions84681/97996 are closed, with eight verified before/after source/tool/log
artifacts. The new marked1 regression is included in continuation1 and not counted
twice. This focused result is not a new full-suite or native qualification.
The verified guard checkpoint is `51e035e2d8811f5a75518bf2f5599a05e6f5361e` —
`fix(kegari-v0): retain review rounds across retry grants`. All11 committed file
bodies and615 tested inputs match; the tree was clean afterward. No consolidation,
history rewrite or push occurred. Proof:
`/private/tmp/kegari-v0-review-retry-round-guard-commit-verification.json`.
Broader paired native/results remain TEMP.
See[the guard report](validation/LOCAL-REVIEW-RETRY-ROUND-GUARD.md). This separate
fix closes a review-grant admission gap; the prior checkpoint implements changed-
source validation continuation, so keeping their local commits separate preserves
a clear behavior and test boundary. No history rewrite is planned.

The changed-source validation successor passed **766 tests,
zero failures and two required native ignores across 39 groups** in
772.838s. Full4 retained615 unchanged inputs at
`sha256:7582d1afeec97bd6632d8848f6e9ead1e7d2f71ec7f442e49f23d12d2b47d439`.
StrictClippy4, fmt4,496-block inventory, diff4 and the dispatch-disabled plan4
also passed with before/after Rust/Cargo1.99.1 identities. Session99672 is closed;
the collector verified15 retained invocations, including the earlier
compile and lint failures. Required native ignores remain incomplete. This does
not establish genuine passed-check/native joined qualification, live evaluations
or required human calibration; the Goal remains incomplete.
The verified local checkpoint is `4244532bf5393c94e6954f3a255132119d06c1af` —
`feat(kegari-v0): preserve validation budgets after source changes`. All19 committed
file bodies and615 tested inputs match, with a clean tree afterward. No commits
were consolidated, rewritten or pushed. This separate coherent increment
continues validation after source changes;2494a28 binds retries to unchanged
source. Verification is retained at
`/private/tmp/kegari-v0-validation-successor-integration-commit-verification.json`.
Check1 failed on a moved borrowed
fixture template; check2 passed after cloning it. StrictClippy2 found one nested
conditional, corrected without changing its predicates. Admission3 found the
invalid three-attempt fixture cap; source4 fixes only its declared data limit. Full historical failures
and source identities remain in the successor report. Paired/carry/evaluation
layers remain independent TEMP work, unqualified and uninstalled.

The exact-source validation retry increment passed **760 tests,
zero failures and two required native ignores across 39 groups** in
737.369s. Full5 retained612 unchanged build inputs at
`sha256:0c7431dfd4c037d499a2320f3fb440c34a6e7742d6b2400ffe2bd50d2c318402`,
with before/after Rust/Cargo1.99.1 identities. StrictClippy5, formatting,496-block
inventory consistency, diff and dispatch-disabled evaluation plan also passed.
Root session1423 is closed. The collector verified 23 retained invocation
artifacts, including failures. Native ignores remain incomplete requirements;
this deterministic evidence does not qualify live/native operation or complete
the Goal.
The verified local checkpoint is `2494a28f24410f33b64952af5b23da94b99e874c` —
`feat(kegari-v0): retain exact validation retry provenance`. All36 committed
file bodies and612 tested inputs match, with a clean tree afterward. No commits
were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-validation-retry-integration-commit-verification.json`.
The actual iterative Drop fix keeps long histories bounded without changing retry
budgets; two65,536-node data-only regressions verify unique and shared tails.
Full4's11 fixture-probe timeouts, its same-source12/12 diagnostic and the source5
owned journaled-probe correction remain retained in the detailed report. No
production timeout or assertion was changed; source5 release5 also passed12/12.
The source successor is now installed above; paired recovery and carry remain TEMP only.

The corrected changed-source successor layer is independently reviewed at
manifest7edb2a93fb1308f7d0ee3e7e11a81c05a02160f58745577028d2025956f9ff8a.
It fixes successful-preparation closure in two readers and adds strict
SourceAdvanced validation lineage for mixed original/granted history. Direct{}
bytes remain unchanged; actual G caps and joined source/receipt closure persist.
Six regressions are authored; current executed results are recorded above.
Codex's exact TEMP source5 merge is now installed, preserving compiled
bounds/destruction/fixture fixes.

Root's paired native layer is TEMP only at
`/private/tmp/kegari-paired-recovery-native-source5-draft` (eight Rust files).
It retains actual registered round/admitted-resume capabilities, routes separate
metadata/source/model stages, and verifies completed bridge identity separately
from fresh clean-source admission. Config independently reviewed the narrow
completed-record/historical enrollment readers with no new concrete finding;
the full layer remains uncompiled and needs review/integration with Config's
opaque paired result/descendant consumers. One TEMP routing edit assertion
stopped before writing; the subsequent scoped edit succeeded. No repository
source was changed by those drafts. Git's carried-acceptance vertical layer also
remains uncompiled TEMP work, with actual baseline-transition integration pending.

Codex's complete eight-file static native review found one scalar/namespace
conflation: a restarted review-grant ordinal must not lower the retained
implementation review counter. Root v2 at
`/private/tmp/kegari-paired-recovery-native-source5-v2` preserves their max,
manifest3124de9ec5846fa722c050b86dc137dc512e16f2b2603c2c80817bce4c1668b9;
Codex verified the narrow fix. Config owns matching actual predecessor-derived
Control accounting and its regression. The original reviewed manifest04f211cc
remains intact. Config's opaque completed-resume and descendant layer must also
carry actual admitted bridge handles through every flat source/budget/pass
closure; native callbacks need the second registered bridge reader. Those
changes remain incomplete and uninstalled, not a successful recovery claim.
Codex separately prepares a versioned structured-review evaluation grader layer;
all actual-agent/provider/native/human evaluation gates remain incomplete.

Root leaf propagation is retained at
`/private/tmp/kegari-paired-resume-validation-leaves-source4`, manifest
68aff68e4f86ab9ffc50de77d212279aa6005f1f4b04f64e5e0f502af9a51836.
Config independently reviewed the four files without a new concrete finding;
these are syntax-only drafts requiring upstream bridge inventories. Root native
v3 added matching actual registered-resume readers; independent review then
found a material new-grant omission: exact-phase scheduling could miss a marked
review predecessor in the old grant namespace. Native v4 adds request/planning
round guards at `/private/tmp/kegari-paired-recovery-native-source4-v4`, manifest
a4307bdbc5ccdf339a1c405ba03a738779809108149ca9968e3cb0fdcafdb851.
Codex owns its actual-origin ReviewRetryWork predicate and pure scheduler
regression. This finding remains unvalidated; v4 is not installed or compiled.
The older native artifact and contrary guard rationale remain historical only.
Root subsequently reviewed the typed completed-resume Control reader and flat
opaque result. A concrete retained-proof gap was confirmed: older non-resumed
source/checkout ownership checks did not survive flattening. Config now owns
preserving a flat actual source inventory plus every joined execution ownership
check, including a new copy of the four leaf collector files; the original68aff
artifact remains intact. Correction, independent review and compilation are
pending. No native or cold recovery success is inferred.

The structured-review evaluation draft is retained at
`/private/tmp/kegari-evaluation-structured-review-draft`, manifest
beb2f39dcf48ac5cef3c26d9f4fafa4de9dea30d5009c177b98550ee2679655a.
It projects actual completed recorded review claims and preserves raw prose for
human assessment; it grants no product/native authority. Root review found a
false-negative for precise GitHub blob-link source references; its correction
and independent integration checks remain pending. No actual evaluation ran.

Source1 check1 failed13.181s on a missing chrono Duration import, parent visibility
and one optional fixture field. Source2 check2 passed20.073s. StrictClippy2 then
failed17.379s on four get/is_none lookups and one test Box replacement; source3
fixes only those lint findings. Every invocation retains unchanged source/tool
snapshots and logs; unrun suffixes after failures have no inferred result.
The installed31-file layer includes actual native retry/paired-failure/passed
successor paths and the shared65,536-record resource bound; configured session
caps and16KiB round limits remain independent. Current data/native-join refusal
fixtures pass without a manufactured success capability.

The destructor fix is installed from
`/private/tmp/kegari-validation-iterative-drop-overlay` (manifestfd5f836f741e1099344beb19c9cf840c196fdd308a6e8049c6d3241cfd2b8f9f).
No retry cap or thread stack was altered. This fixes the direct ValidationRetryWork
previous-chain destruction, not every separate existing review/source graph.

The later changed-source path after a genuine granted validation pass, review
Findings and paired repair remains incomplete. Codex prepares an opaque
ValidationBudgetLineage extension, independently reviewed by Config, so the new
validation retains actual G ancestry/remaining caps and rejects old-source cache
reuse. That layer is TEMP only and must update every transitive joined reader.
Git prepares carried acceptance with actual known-released baseline proofs.
Its further prerequisite is typed same-generation parent reassembly: Complete
currently stops, and plain integrated/review admissions cannot reset budgets.
Generic ArtifactReference data cannot supply either capability. Config now owns
TEMP paired Control metadata/resume admissions; root's native seams/design are in
`/private/tmp/kegari-paired-recovery-control-design`. Original paired producer
still lacks dedicated native consumption and repeated resumed-failure bridge.

The inventory correction is committed as
`4d2a62030012ef81cacc0409cdcc5439c39587dd` —
`fix(kegari-v0): retain all implementation contract blocks`.
Seven exact file bodies and the clean tree were verified before this retry
installation; no commits were consolidated or pushed. Source8's core suite below
is historical evidence for its exact603 inputs, not a pass for current612 inputs.

The child review-round core passed **747 tests, zero failures and
two required native ignores across 39 groups** in 727.152s.
Full8 and its strict Clippy, formatting, inventory, diff and dispatch-disabled
plan checks retained unchanged603 source inputs and observed Rust/Cargo1.99.1
before/after identities. Source digest:
`sha256:5e614d0b3417e80dc611d69f5750e7b3610c8cdfeac0b06104e24a8c8e86f3aa`.
Session69517 is closed. The collector verified 23 retained invocations and
their log/source/tool artifacts, including earlier failures. The two native ignores
remain incomplete requirements; this deterministic pass is not operational
qualification or completion of the Goal.
The verified core checkpoint is `6ef65e5efb7303c97c9b85f1df0536fdff8c21b0` —
`feat(kegari-v0): bind child reviews to bounded round ancestry`. All42 staged
files and603 tested inputs match the commit, with a clean tree afterward.
No history was rewritten. Initial automatic review rejected the local commit
for missing trusted explicit authorization; exact authorization in the
user-designated objective was reverified and normal automatic re-review approved.
No bypass/manual gate or remote mutation occurred. Verification:
`/private/tmp/kegari-v0-review-round-core-commit-verification.json`.
The later retry source is now installed as source5 above; prior core evidence remains exact.

Full6 failed405.337s with241 library passes/3 failures, later groups unrun.
Source7 corrected the obsolete validation-as-review-round assertion; schedule7
passed12/12. Source8 corrects the actual round fixtures' refreshed-phase gate:
after native refresh, use the current Authorized gate and its new phase-scope
digest, preserving original proofs, timestamps, limits and assertions. Earlier
compile/schema/lint/legacy-diagnostic failures remain in the
[round-core report](validation/LOCAL-REVIEW-ROUND-CORE.md). The strict Initial{}
serde correction preserves canonical JSON and rejects unknown fields. Legacy
compatibility helpers remain test-only; no native success capability was added.

Next validation retry/passed-successor work is composed only in TEMP at
`/private/tmp/kegari-validation-combined-core8-v1` (31 files, manifest
`d16f19050f675692275209e099ec0aed73a9237242127263654f862accf483ae`).
Exact original/base/draft hashes were checked in order: native3 f302b548,
paired Direct/Retried9 8acacb87, final clean round9 f68e7241, final pipeline5
5089ae7b and fixtures/defaults10 b329f12f. Known final integration includes the
PassedValidationReference reexport; obsolete TEMP comments also need removal.
This original v1 composition is retained as history; its corrected v3 successor
is installed and source5 is under full validation above. Independent reviews cover
the actual cause/bounds/current-successor/grant seams; genuine passed/native
execution qualification is still absent. Original layers remain preserved.

The contract inventory correction is now installed and verified:496 blocks,
including133 newly enumerated unassessed entries. All363 original clause objects,
136 named requirements and stale entries remain exact. Six repository tests,
ledger consistency and diff checks pass with unchanged603 Rust inputs. No Cargo
repeat or product evidence upgrade was needed. Two independent reviews confirm
all559 in-scope nonstructural lines are indexed once. Historical completeness
claims are corrected explicitly in the [inventory report](validation/LOCAL-CONTRACT-INVENTORY.md).
This evidence-tool increment is committed as4d2a620 above.

Root's next retry composition is `/private/tmp/kegari-validation-combined-core8-v3`,
31 files, manifest5df0414b2f0429e171cfb83e091653365acf13853855d4f00e628c5ade906196.
It adds the reviewed shared65,536-record resource bound, required type reexport,
and obsolete-comment cleanup; it was installed as source1 and corrected through
the source5 validation checkpoint recorded above.
The failed partial v2 TEMP preparation is retained (wrong overlay source path;
no repository write). A separate changed-source validation successor must keep
actual granted G ancestry and remaining caps after passed retry/Findings/repair;
Codex prepares it and Config independently reviews. Carried acceptance and
external production observations remain unfinished engineering. Required native,
actual-agent, provider and human qualification remain separate blockers.

The history evidence below applies to committed source10, not these new inputs.

The owned-target/scoped-history increment passed **721 tests, zero failures
and 2 native ignores across 39 groups** in 727.621s. Full10 ran at
unchanged 593 inputs, `sha256:b65dcbbcc2a32882362d2a4c73a50841e8fd811a96043ef6bd4e273c69a16214`,
with actual before/after Rust/Cargo1.99.1 identities. Source10 strict all-target
Clippy, formatting, contract inventory, diff and dispatch-disabled evaluation
plan also passed. The plan contains56 slots and did not dispatch evaluations.
Session52897 is closed; root now owns serial core checks. Root exclusively owns source windows,
serial Cargo, progress and local commits.

This increment retains ordinary owned-target provenance in immutable no-release
history. Every target keeps its complete artifact map and exact closed original/
resumed receipts. Authenticated failed references deny the affected resources and
exact dependency outcome while preserving unrelated valid history. Malformed,
unauthenticated, unknown-scope and unsafe outer storage remains globally blocked.
First authenticated reads, remembered identities, completion writes and complete
queue/cache installation share Control serialization. Shared failures protect
fresh facades; ordinary and no-release failures remain structurally separate.

Actual regressions cover lost seals before any sync, denied cached fallback and
new-facade remint, scoped refusal with disjoint eligibility, explicit complete-scan
recovery, and a production inventory reader racing a genuine delayed commit.
The race uses fresh runtime caches sharing a registered ControlHost, bounded
operation-local test barriers and joined fixture threads; it is not a full OS
restart. Upstream accepted/check state remains explicitly synthetic in some
fixtures. No native confinement or successful required check capability is forged.

The [owned-target report](validation/LOCAL-OWNED-TARGET-HISTORY.md) and
[machine evidence](validation/owned-target-history-checks.json) retain
36 completed invocations, exact commands, source/tool snapshots and
hashed logs. Focused source9 checks passed before the lint-only source10 edits:
check9 (16.926s), no-release9 (14/14,92.698s), ordinary9 (1/1,185.531s) and chain9
(1/1,208.274s). Full10 exercises those scenarios again. Earlier failed compilation,
assertions, stack aborts and strict linting remain retained, with their fixes and
subsequent results. The prior full2 pass719/0/2 predates scoped quarantine;
full10 is the current complete-suite evidence. Bounded independent reviews of
original identity, scoped quarantine, shared failure/lock ordering and native
receipt ancestry have no unresolved concrete finding in this increment.

The verified local checkpoint is
`6523997487d7952487f9af4d2c8d008c88b1306f` —
`feat(kegari-v0): preserve scoped immutable delivery history`.
It consolidates related unpublished6936aa21. Recovery reference
`refs/codex/recovery/no-release-history-6936aa21` preserves the old commit.
All39 staged file bytes and593 validated inputs were verified after amendment;
the tree was clean before starting the next increment. Exact verification is
`/private/tmp/kegari-v0-owned-target-history-commit-verification.json`.
Read-only remote heads/tags still placed main at44b7578e before consolidation.
Published44b7578e and946a15f remain preserved. No remote mutation was performed.

The operator remains **unattended**. Actual-agent and real-provider qualification,
native containment and required human calibration remain incomplete. Arbitrary
repository commands fail closed before unqualified execution. No live evaluations,
remote writes, installation, deployment, denied native probes or host security
changes were performed. SPEC remains version1.2 with approved SHA
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.
This checkpoint does not establish readiness to coordinate the lasting build.

Next: commit the verified inventory correction, then integrate and validate
validation retry and clean-successor lineage only after their concrete producers,
consumers and independent findings are resolved. Further engineering includes
paired transient recovery, external production observations, cold/corrected and
configured-hook recovery, carried acceptance and concrete evaluation composition.
Live/human blockers remain separate from useful authorized engineering.

## Historical preparation notes

This section preserves earlier preparation and validation checkpoints. Its
in-flight wording is superseded by the current summary above and later reports;
it is not authority to run stale commands, amend old commits or replace sources.

Root prepared the 36-file core paired-round slice against source10 in
`/private/tmp/kegari-review-round-core-source10-prepared-v2`, manifest
`2b5c3e317407cb9b5fb60436a28026d8d671a375e54f702a51e790873c138188`.
All original bases and selected draft hashes matched. Only the release reader's
four marker exclusions needed a clean merge onto newer history code. This slice
is now installed but not yet compiled successfully; root will validate it before
layering unfinished validation retry and session recovery work. The first TEMP preparation stopped
on an absent physical fixture base copy; v2 verifies the retained original hashes
against actual files and preserves that failed partial TEMP directory.

Git's separate validation component fixture is held at
`/private/tmp/kegari-validation-retry-fixture-overlay`, manifest
`c85d7c3ba365be095690ac6b702c3eca870320c498f65556324201e1a63792ca`.
It authors actual grant/Control/source/guarded RPC paths, while absent actual
runtime joins and required check executions remain precise refusal barriers.
Bounded static native-adapter review found no additional issue; no compilation
or execution claim follows. Config's clean-successor design is at
`/private/tmp/kegari-validation-clean-successor-design/README.md`; its narrow
opaque producer/child handoff and Codex's matching round/Control bridge are being
prepared separately. All original layers remain held.

Committed source10 passed complete validation. The core paired-review slice is
installed; future validation-retry and paired-session-resume work remain TEMP only. Root owns `/private/tmp/kegari-review-round-runtime-draft` (runtime,
worker adapter and scheduler copies). Codex owns
`/private/tmp/kegari-review-round-draft` (opaque producers, Control and child
pipeline); Git owns `/private/tmp/kegari-review-round-fixture-draft`. These are
uncompiled. A reviewed preparation-link mismatch in the root adapter was corrected:
actual preparation UUID/whole digest is retained, not decoded as a model-run link.
Actual native joined evidence remains mandatory; pending fixtures must assert its
real absence, not seed a success capability. Raw current review limits and each
review grant remain distinct from original implementation attempt/turn caps.
Validation after a paired repair needs phase-correct accounting without consuming
another review round; this seam remains under review.

Current round integration precedence is recorded in
`/private/tmp/kegari-review-round-integration-source9.json` (36 selected files,
not applied). It includes the held same-pair fixture overlay and new
`/private/tmp/kegari-review-round-native-source8-overlay`; the latter rebases only
State.released_inventory_error into both captured runtime base/result. Original
layers remain intact. Root's 13-file
`/private/tmp/kegari-review-round-mechanical-source8-overlay` adds actual legacy
LaunchRequest.review_round=None and explicit worker-marker exclusions; independent
review and standalone rustfmt passed, with no compilation claim. Its release.rs
marker hunk must merge with newer history/race edits rather than replace the file.

Config's `/private/tmp/kegari-validation-retry-pipeline-draft` holds four syntax-
formatted modules (manifest700160158d44c8a1635c7cfaf71565e38d3dea7a1803191e6ef4c71f6225f71f).
It binds failed Validate retries to actual assignment/source/grant/receipt and
completed lineage before genuine local checks; the additive paired result consumer
and native adapter are explicit missing dependencies. Config now owns a separate
TEMP native ValidationRetry layer based on the new source8 native overlay. Codex
owns a separate actual paired-session-resume producer/Control-link draft, followed
by the Direct/Retried paired-validation consumer. Git owns new actual validation-
retry component/protocol fixtures. None is installed, compiled or qualified; no
raw saved references can replace missing opaque source, joined completion or
failed-check evidence. Root owns future integration and all Cargo.

Config's `/private/tmp/kegari-external-observation-draft` has additive config and
protocol interfaces only; the distinct actual serving-artifact/source/read-only
verifier and runtime producer remain unfinished. Its ordinary recovery design is
read-only, with separate actual closed observation finalization. Config is also
preparing a separate opaque validation-retry producer for the concrete failed
Validate-stage liveness gap; no shared runtime/pipeline edits are installed.

Before another local commit, recheck publication state. At the last observation
origin/main remained 44b7578e and local HEAD was 6936aa21. If still unpublished,
the related owned-target increment should consolidate into that local no-release
checkpoint under the applicable commit rule, after creating a local recovery ref
and verifying final file contents. Never rewrite it if publication is observed;
no push or remote mutation is authorized.

## Prior increment validation and ownership

Full suite 23's source-sealing failures used current retained provider projections
at each journaled Git spawn/completion. RuntimeDriver previously awaited a whole
delivery phase under its cycle lock; supervisor heartbeats refreshed process and
intent only, and the driver-step deadline was 30 seconds. A long phase could
expire observation gates or lose its waiting future. The implementation now owns
one retained progression task and one separate bounded read task, counts both in
drain checks, joins before clearance, and keeps fenced cancellation independent.
Real refresh failures invalidate current and retained gates without redating them.
Production freshness/deadline limits are unchanged; no uncertain effect is replayed.

Codex agent's driver changes and tests are saved and held. Git agent's continuation
fixtures are saved and held: actual shared observation ownership surrounds Git
sealing; boxed futures avoid nested stack expansion; actual bulk provider requests
prove refresh. Driver composition covers nonblocking polls, actual data changes,
pause/force-stop, preserved permit ownership, real task join and truthful no-process
retirement. Diagnostic incomplete projections are tolerated only while fresh
authority remains closed. Driver 28 passed 13/13 in 22.204s wall.

Config agent's active-contract correction is saved in runtime_delivery_parent.rs,
runtime_delivery_baseline.rs, delivery.rs and a narrow type re-export. The new
input is minted only from an actual ActivePhase under exact permit/current owner/
fence checks and a parent fetched from the retained observation registry. Its
adjusted observation revision is distinct from the admitted execution revision.
Ordinary parent_contract retains exact configuration equality; all other scope,
definition, child, target, dependency and workflow checks remain shared. Both
baseline derivation and parent progression now use the active path. Codex agent's
independent review found no material issue; the Result<()> callback shape was
corrected before compilation. Root owns integration and commits.

Progression 28 closed 101 after 11.612s (three passes, one stale_result at parent
contract construction). The fix passed progression 29 (4/4, 41.576s wall). The
nonempty A-to-B held-baseline reload regression preserves exact origin, effect,
source/target and receipt identities while feeding the retained proof into real
successor import and assembly. Chain 29 closed 101 after 130.262s at a later B
admission after synthetic origin publication (chain_fixture.rs:523); the latest
reconciliation was unavailable. Config agent investigates the explicit fresh-read
boundary and background observation ordering, without changing production refusal.
That fixture correction passed chain 30 (1/1, 223.875s wall). Inventory 31
and diff 30 pass at the latest source (363 blocks/136 named requirements).
Clippy30 and formatting30 passed at3535f1 source. Full30 subsequently closed101
with204/205 library passes; its sole old retention-fixture lease mismatch is
corrected without changing production checks. Focused retention31 passed1/1.
Full 31 completed with 705 passes, zero failures and two native ignores across
39 groups; its session is closed. Clippy 31, formatting 31 and the dispatch-disabled
plan 31 also passed at the source above. Final independent bounded reviews found
no new material defect in active contracts, completion or driver/resume ownership.
The report collector now verifies retained log and snapshot hashes, complete test
groups and separate inventory checker inputs. It found no mismatched runner
artifacts; legacy records are explicitly unverified and cannot qualify final checks.
Inventory invocation 29 used an incorrect script path and
never ran the check; its failure remains in the command evidence.
All earlier compile, fixture mismatch, stack and partial-observation failures are
retained in the detailed report; they have not been replaced by later passes.

## Baseline and authority

- Started 2026-10-25 on existing `main`, clean worktree and empty index, at `36a3e83` (`docs(kegari-v0): finalize the specification and build prompt`). No project branches/worktrees created.
- Read the complete SPEC 1.2; SHA-256 `80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326` matches the approved contract. Source baselines remain those recorded in SPEC §1.1. SPEC/GOAL and existing license are preserved.
- Applicable instructions are the user-supplied AGENTS rules; no additional AGENTS.md exists in this repository or its ancestor directories inspected. The goal authorizes local signed-off commits, owned disposable Git test fixtures and implementation inside this Rust repository. Remote repository writes/pushes/deployment and host configuration changes remain prohibited.
- Effective tools: workspace-write sandbox, automatic approval review; repository and temporary paths writable, host configuration not writable. No permission/security change made.
- Installed: Rust/Cargo 1.99.1 (under existing `stable` alias), Git 2.56.0, Codex `0.164.0-alpha.11.1`, macOS sandbox-exec/Keychain/launchctl. `tailscale`, `ffmpeg`, `ffprobe` not found on PATH. `rust-toolchain.toml` now pins 1.99.1; development uses `RUSTUP_TOOLCHAIN=stable` only because this alias was verified to contain that exact version. No toolchain installation is authorized or needed on this host. This is availability evidence only, not compatibility qualification.
- Builder model/speed settings are inherited unchanged. The runtime profiles will remain explicitly configured and separate. Assistance attribution will omit a model identifier unless independently verified.

## Implementation plan and historical foundation checkpoint

1. Establish strict typed identities/configuration, canonical evidence, durable records, and runtime-owned instruction assembly.
2. Implement the pinned app-server and enforced execution boundary; implement Git provenance, candidate isolation and all separate evidence gates.
3. Connect real provider adapters through serialized admission/mutation/recovery; implement scheduler, local/HTTPS CLI control and platform lifecycle.
4. Deliver bounded executable evaluation fixtures/graders and operator preparation/qualification guidance.
5. Run build/fmt/clippy, observable deterministic integration/failure tests; independently review and correct. Commit coherent verified increments on `main`.
6. Prepare exact bounded live plans and human review material. Qualification stays incomplete until separately authorized live checks and human calibration pass.

Historical foundation checkpoint: strict configuration/workflow, embedded instruction assets, canonical digests, private durable storage, serialized intent/admission/mutations, authorization/correction/scheduling primitives, immutable paged snapshots, pinned app-server/isolation adapter, and deterministic Git/evidence/release primitives are implemented and have focused tests. Real Git fixtures preserve the source checkout/index, exclude staged B from A's candidate, and preserve failed candidates. Provider schemas/config parsing are verified locally; actual sandbox enforcement is not.

Root owns repository integration, validation, this record and commits. Current agents work only in TEMP: configuration handles declared protocol observation, Codex independently reviews gaps/observations, and delivery prepares evidence. The earlier foundation work split Control/runtime, release history and worker recovery across these agents; that ownership snapshot is superseded. The native `main.rs`/factory is now wired to the concrete delivery decisions and shared worker runtime. CLI-only executable checks pass; no service was activated. Component and focused fixtures do not establish all complete delivery/recovery scenarios.

The detailed local integration report records the checkpoint's full-suite counts.
Later focused results below identify the tested increment and do not establish a
complete SC/V0-T scenario or real-provider qualification.

## Current remaining engineering

A bounded independent caller audit reconciled the old checklist with current source:
`/private/tmp/kegari-remaining-engineering-reconciliation-v1/REVIEW.md`, SHA-256
`291cdaccc27dc62b086ddd2d140b0fbb2c389056c7dc4154add83439144f6e1b`.
Root read the complete report. It is static evidence for the listed slices, not an
exhaustive SPEC audit or a new native qualification.

| Scope | Current remaining work |
| --- | --- |
| Control membership and parent history | Committed1e14788 after917 full-suite passes and two required native ignores; all38 new names passed. Evidence and narrow partial coverage are finalized. |
| Native evaluation and E07 | Declared authority/protocol observations committed102984e after931 passes; transport extension locally verified with947 passes, broker read/effect inventory in TEMP. Concrete canary/output rubric and versioned pr_attack migration remain. Only independent_malicious currently has native context;27 contexts and other role/prior-state lanes still need genuine producers. All56 slots remain unqualified. |
| Coordination and continuation observations | Derive assertions from actual scoped publications, receipts and predecessor lineage. Repository-free coordination needs its own substrate; fixture JSON cannot create prior execution authority. |
| Cold recovery and native containment | Historical parent seals do not recreate worker/source/completion capabilities. Implement a distinct supported cold obligation with fresh evidence. Actual descendant containment remains a code gap in addition to the denied native host prerequisites; group/leader exit is insufficient. |
| External targets and visual cold readback | Add explicitly supported serving-artifact/source and authenticated video retrieval producers. Existing deployment-free/owned-target paths remain valid within their local evidence limits. |

The prior no-release/owned-target history, structured review claims, exact paired
round/retry consumers, carried acceptance, released dependency cache and typed
historical Overview/Show now have concrete callers and separate local reports.
Do not reimplement them from the obsolete proposals below. Narrow same-process
provider-thread reuse refusals do not by themselves imply missing required behavior
when a reconciled fresh bounded admission already exists. Live/provider/account,
host compatibility and human calibration remain separate recorded prerequisites.

## Historical engineering slices (superseded checkpoints)

The following original observations/proposals are retained for chronology. Their
present-tense statements describe earlier source; the current reconciliation above
and the top checkpoint take precedence.

The deployment-free no-release history drafts are integrated into the working
source, with source authority/refusal and missing-reference facade corrections
being finalized before root compilation. TEMP draft artifacts remain under
`/private/tmp/kegari-no-release-{history,fixture,inspection,root}-draft` as review
inputs, not current verification evidence. The origin-owner/approval audit keeps
the established observed-publication boundary under SPEC §8.6; requiring later
decision/closure success would reject valid acceptance after a lost response.
No broader transitive-closure claim is made.

The separate unintegrated review-round provenance draft remains under
`/private/tmp/kegari-review-round-draft`. It now includes common completed-round
capability/plan producers and continuation preparation, but atomic Control/session
accounting and scheduling are not wired and no Cargo verification exists.

The next owned-target historical no-release draft is under
`/private/tmp/kegari-owned-target-history-draft`; it carries actual target proof/
challenge/closed receipt ancestry through the baseline and complete accepted
vector. Independent static review found no material defect; an authentic target
fixture and signed legacy roundtrip are still required. External deployment proof
needs a distinct actual read-only observation producer, described under
`/private/tmp/kegari-external-baseline-design`; no owned deployment dispatch is
invented. None of these next-slice source drafts are integrated into this run.

- Native evaluation composition remains an implementation obligation. The
  checked-in binary exposes only Plan/Inspect/Compare; no concrete
  `LiveAuthority` implementation or native suite executor is present. The 28
  fixtures have legacy synthetic contexts rather than declared complete native
  contexts. Production `authorized_context` includes canonical issues, child
  projections, native observations, scoped evidence and work identity, all of
  which must be declared and matched without normalizing away fields. Several
  scorers also expect synthetic state paths (for example coordination children
  and repeated-step counts) absent from `RuntimeCoordinator::observed_state`.
  Required next composition must establish those facts from actual verified
  coordination/continuation receipts, install declared corrections through
  ordinary intake, and use genuine prior-session proofs for recovery variants.
  A fixture JSON or authority-reference string must not mint these capabilities.
  Existing owned preparation, `AdmittedTrial`, `finish_trial` and grading APIs
  supply reusable pieces; their presence is not evidence that a native suite
  can execute. Keep all 56 slots unqualified until this composition and the
  separately authorized live/human checks are complete.
  Independent inspection also found a grading-contract mismatch: shipped
  review instructions require a structured verdict/findings/links claim, while
  `ScopedWorker::observe` looks for an `AgentSummary` under `claim.summary` and
  current review graders expect summary/review-state fields. Migrate the
  versioned graders to the authenticated structured claim, retaining independent
  source/effect/publication checks; a verdict must not become authoritative
  completion. Coordination uses a production substrate without a repository,
  whereas the existing owned-clone fixture binder requires seeded repository
  bytes. These need explicit separate fixture paths. A read-only independent
  review is the proposed first complete native composition case.

- Exact-source failed child review retry now has producer, consumer and component
  tests through repeated transient failures and clean publication. Review
  findings/repair across mixed grant budgets and cold lineage remain to be
  completed; old phase records and caps must stay visible.
  Independent design review confirmed the next correction: new ordinary and
  granted review admissions need an authenticated, versioned round identity
  scoped to their complete review PhaseKey and exact reviewed source. Transient
  retries retain that identity while consuming a session attempt; Findings pairs
  its required repair with the same round. Final-round repairs remain bounded by
  the original implementation attempts, while another review is rejected beyond
  the review ceiling. Existing scalar histories must not be silently reinterpreted
  or have counters reduced. Completed source-advanced reviews must retain their
  exact completion/claim/receipt lineage for recurring Findings and transient
  retry; persisted references alone cannot recreate execution authority.
  Proposed integration seam (not implemented): a separate typed
  `ReviewRoundReference` on preparation and run admissions, atomically persisted
  by the existing Control admission transactions, with a sealed
  `VerifiedReviewRoundPlan` produced from actual completed child/source/review
  assignment capabilities. Initial, transient-retry, paired-repair and
  after-repair causes bind exact operations, whole receipts and source/claim
  digests. `LaunchRequest` carries that opaque plan; preparation and final
  admission recheck current grant, inventory, assignment, contract and capacities.
  Keep `RunReference`'s session budget independent and require preparation/model
  references to agree. New ordinary and granted review producers must both emit
  round provenance, while unmarked legacy chains remain conservative.
- Cold recovery cannot recreate original `WorkerContract`/`RunCompletion` from
  current digests. First persist a bounded signed recovery record at the genuine
  unfinished producer, then reconstruct a separate obligation capability only
  after fresh native authority, real owned Git/source and closed receipt/journal
  checks. The first review-findings path must drop all in-memory proof state in
  its positive restart test. Legacy/missing/local-check-only/uncertain records
  remain blocked; no synthetic confined execution is acceptable.
- Provider resume now has protected submitted-input/history provenance, guarded
  metadata recovery, actual source/family proofs, typed admission and a native
  preparation consumer for the narrow known transient terminal-failure case.
  Automatic same-process runtime selection and metadata/source acquisition are
  wired and locally checked; native qualification is still required. Repeated
  resumed-thread prefix reconciliation, corrected/review
  sessions, configured before/after hooks and active/unresolved turns require
  separate work. Preserve actual joined completion identity; no cold JSON may
  construct it. Shared-directory enrollment is tested, and every member's cleanup
  and retained redirect must remain valid through actual cleanup.
- Chained integration recovery now verifies bounded signed admission/supersede/
  publication lineage, rejects cycles and proves current-main Git ancestry while
  deduplicating original contributions. The actual A/B/C Git/Control/native-wire
  fixture retains A exactly once through a held deployment-only B and corrected
  C after restart; it still blocks at missing genuine integrated evidence.
  Carried acceptance remains a producer/integration gap, not only a historical
  reader gap: all concrete runtime release paths currently pass `carried: None`.
  The pure evidence constructor and negative tests do not supply an actual
  producer or durable native publication. The next slice needs original and
  current verified parent results, the unchanged ordered patch proof, actual
  fresh production validation and a sealed known-released baseline lineage.
  Its signed publication must retain the original human-reviewed token/time and
  both parent/vector references while the plan names the newer machine result.
  History/completion then needs tagged direct-or-carried reference inventories;
  no JSON record may recreate current carry authority.
- Exact original target proof recovery uses immutable authenticated identity
  and complete historical observations separately from current readiness. The
  current increment implements terminal transitive completion projection from
  actually successful, retired descendants and the authenticated chain, preserving
  original failed outcomes and unrelated holds. Focused and prior library checks
  pass; its latest nonempty reload chain is being checked before the final suite.
- Released-parent dependency propagation is a separate production gap:
  `DependencyOutcomes::released` has no callers and accepts a raw attempt, while
  `effective_evidence` merges only fresh delivery snapshots. Independent design
  review recommends a separate cache of sealed release completions. Generalize
  `VerifiedReleaseCompletion` from two-or-more attempts to one-or-more; ordinary
  successful retired releases need the same complete authenticated inventory but
  clear no ancestor holds. Insert dependency credit only from that opaque proof,
  using the queue's exact generation/outcome digest. Install outside existing
  Control/release locks, and revalidate immutable references before consumption.
  Missing/replaced references revoke only that exact credit with diagnostics.
  Keep current runtime/workspace/account binding and actual observation gates.
  SPEC §5.2 requires B pinned to released A1 to remain satisfied after A2 is
  authorized; A1 cannot satisfy a dependency pinned to A2. Historical release
  facts therefore must not expire with current delivery snapshots or require
  production to remain at the historical revision. Native missing/inaccessible
  prerequisites still block. Tests need ordinary completion, C without failed
  ancestor credit, A1/A2 pins, reference deletion/restoration, cold recovery after
  production moves, config removal/remapping, failed observations and unrelated
  credit preservation. This design is not implemented in the current increment.
- Operator history still needs a typed Overview projection for successful
  releases: current release inspection enriches Show details, while the only
  `HistoryRow` producer feeds no-release-required outcomes. Ordinary excluded or
  inaccessible parents are retained and remain resolvable. Authenticated release
  history absent from the current parent projection needs a separate read-only
  canonical resolver; it must never expand scheduling or mutation targets.
  Next-slice design (not implemented): inspection identity is workspace UUID plus
  full `IssueKey`; each attempt retains exact recorded `WorkIdentity`. Return a
  sorted typed vector rather than a JSON map keyed only by issue. Merge it into
  `RuntimeInspection`, never `RuntimeProjection.parents`, retained scheduling
  refs or dependency outcomes. Overview appends/filter/sorts history outside the
  current-parent loop; Show combines exact current/historical identities and
  rejects ambiguity. An authenticated removed workspace UUID may be selected
  only by this read-only resolver. Missing legacy name/title/key/URL stays unknown;
  Open requires an actually observed URL. General workspace and approve/release/
  correct resolution remains current-only. Existing release proof validation
  requires a current repository contract; do not weaken it for removed-workspace
  display. Those records may expose authenticated recorded outcomes plus explicit
  diagnostics, not recreated success/readiness. Only existing terminal completion
  validation for the exact work/generation may suppress that generation's active
  display. `DependencyOutcomes::released` currently has no production caller;
  this separate delivery integration gap cannot be filled with a UI predicate.
  Tests must cover removed workspace/root, cross-workspace ambiguity, later
  corrections, missing actual completion/owner/step refs, missing/unsafe URL,
  unchanged mutation targeting and unchanged receipt/permit counts.
- Same-grant transient failed reviews mint fresh per-attempt capabilities
  preserving assignment/source, original caps, backoff and contiguous history.
  The current Findings-to-repair and later source-advanced review implementation
  rechecks retained grant lineage during advisory, preparation, admission and
  publication; focused checks pass and stale/deleted/forked grants cannot authorize
  repair. Recurring Findings and transient failures after changed-source review
  still need exact paired-round provenance. Validation retry follows separately.


## Requirement-to-evidence checklist

Status key: **not run** is not a pass; **implemented** requires tests/review before **verified**. Detailed deterministic cases will be linked as they are executed. The complete normative contract remains SPEC; this grouped ledger does not waive any clause.

| Contract / retained requirements | Planned evidence | Current status |
| --- | --- | --- |
| §§1–2: product boundary, identities, authority, persistence; REQ-14/31/42/45/46/52/54 | Build, canonical identities, role/broker rejection, durability/ownership | Primitives implemented; 12 storage/control tests pass; process registration implemented; native qualification blocked |
| §§3.1–3.4: strict config, credentials, workflow/reload; REQ-34/35/40/41/50/51/57; H01/H02 | Config/workflow/secret/reload tests, installed asset assembly | 28 focused tests pass including context-composition review fixes; native Keychain implemented but unrun; serialized setup integration in flight |
| §§3.5–4: preparation, provider reads, authorization/scope/review; REQ-01–04/29/37–39/43/44/62 | Paginated provider fixtures, event provenance and scope revocation | Offline provider/reconciliation/decision fixtures pass in the frozen checkpoint; real provider provenance/compatibility qualification remains unrun |
| §5: lifecycle/dependencies/corrections; REQ-07–09/19/22–24/30 | Correction races, retained child sets, dependency generations | Local correction/scope primitives tested; provider reconciliation tested; full workflow loop in flight |
| §6: scheduling/bounds/claims; REQ-12/21/32/35; V0-T14 | Fair account/workspace admission, attempts, exact review-round ancestry and pause races | Core round Control tests40/40 and child continuation9/9 passed; current full-suite run pending; cold/corrected/repeated and retried-validation recovery incomplete |
| §7: isolation/Codex/harness; REQ-31/32/34/42/54; H01/H02 | Pinned-schema protocol fixtures, OS sandbox probes, manifest/discovery | Pinned protocol, active corrections and shared worker/evaluation path tested locally; actual containment canaries blocked and live discovery unverified |
| §8: local/hosted/review/staging/evidence/acceptance; REQ-05/06/10/11/19–21/25–28/43/44/53 | Exact-revision evidence, attestation, Git fixtures, stale tokens/video | Native provider adapters and separate gate progression integrated and locally tested; complete live workflow/media qualification outstanding |
| §9: candidates/carry/queues/release/recovery; REQ-13/15–18/27/28/30 | A/B isolation, patch bytes, partial effects/holds, correction races | Committed owned-target immutable/scoped history passed721/0/2; actual anchor race tested; external production observation and carried acceptance remain incomplete |
| §10: complete CLI/control/overview/TLS; REQ-49–51/55–57 | CLI/API observable tests, peer identities, snapshot pagination | CLI/control/dispatcher/native executable factory wired; deterministic CLI/API/transport checks pass; native service qualification unrun |
| §11: intent/cancellation/recovery/cleanup/logs/handover; REQ-31/33/36/40/41/52/58–62 | Crash boundaries, restart/stop/force, preservation and redaction | Offline control/process/recovery/cleanup fixtures pass in the frozen checkpoint; complete historical and native lifecycle qualification remains incomplete |
| §12.1 SC-01–SC-26, each retained scenario | Observable deterministic + required live profiles | Partial primitive coverage; no complete end-to-end scenario claimed |
| §12.2 V0-T01–V0-T37, each complete adversarial case | Deterministic process/provider/Git/transport failure fixtures | Partial focused coverage; full rows remain unverified |
| §12.4 V0-E01–V0-E08, every variant repeated per profile; H03 | Actual integrated harness trials, isolated graders, all results | Blocked live authority; 56 declared trial slots and deterministic graders delivered; actual trials not run |
| §§12.3–12.4 independent review and human calibration | Independent code/behavior review; human qualitative review | Multiple bounded independent reviews and corrected regressions recorded; full conformance review/human calibration incomplete |
| §§13–15 traceability/licenses | Source ID coverage, no spec changes or license regression | Baseline verified; final audit not run |
| REQ-05/33/40/41/45–51/53 graphical/distribution portions | Dashboard, wrapper, frontend, Homebrew, other OS packaging | Deferred exactly as SPEC §§1/13 |

## Validation and evidence

- **Native recovery/history checkpoint:** complete suite 683 passed, zero failed,
  two native ignores across 38 groups. Library 190/190 in 255.01s includes both
  corrected/chained successor fixtures. Final strict all-target Clippy passed in
  17.76s; deployment 19/19 and terminal target 1/1 focused reruns passed before
  the full run. Formatting, inventory and diff checks passed. Final 564-input
  digest `sha256:c438a22a3ea57152fcf48f92ae9023c1fb753cec9f18c621d875da9dac159c58` was
  unchanged during and after validation. The current report and JSON above
  retain exact commands, log hashes, source versions and remaining limitations.
- **Retained failures in this increment:** initial compilation used field access
  as a method; first full invocation stopped after 187 library passes/3 failures
  because target-proof validation rejected actual active Prepared admissions and
  a fixture raced unstable refresh. Exact live permit checks and stable fixture
  readback corrected these while keeping retired historical gates strict. A later
  deployment run passed 18/19; its second-target fixture now supplies complete
  native facts while retaining its missing-proof assertion. Strict Clippy found
  argument-count/enum-size issues; grouped stage inputs and boxed payloads fixed
  them without suppression. Final full evidence passed on the corrected source.
- **Independent review corrections:** automatic review retry now requires minimal
  typed transient provenance bound to the full terminal content digest; saved
  contract identity is independent of fresh current authority. Actual joined
  metadata task evidence controls delayed reservation release and recovery errors
  are visible. Exact release plan/resume permit scope, reconciled dispatch effect
  identity, interrupted index repair and read-only recovery from absent or valid
  old current pointers are enforced. Corrupt/dangling pointers, ambiguous proofs,
  unknown exits and cold Prepared receipts remain blocked. Bounded peer reviews
  found no remaining material defect in these paths; complete conformance and
  live/human qualification remain outstanding.

- **Active session-resume admission component:** root added `RunResumeReference`
  and serialized same-phase admission binding the exact latest predecessor,
  recovery/assembly/directory digests and consumed review rounds. Original account,
  profile and minimum phase caps, one-second spacing, no duplicate recovery,
  failed terminal/exited/reconciled predecessor and a mandatory fresh proof gate
  are required. These references grant no provider/directory capability. Typed
  marker `session_resume` is excluded from preparation/continuation/release
  provenance. All three actual-Control component regressions passed in 2.33s,
  log `/private/tmp/kegari-v0-session-resume-control.log`; their terminal observation
  is explicitly synthetic and cannot qualify native resume. Independent bounded
  review found no confirmed admission bypass; full provider history and native
  failure classification remain separate opaque proof requirements. That initial
  run emitted an unrelated in-flight unused-method warning. Subsequent runtime
  proof consumption and family enrollment are described below; no new frozen
  all-target checkpoint is claimed.
- **Directory-family cleanup work in progress:** initial protected family creation
  binds actual root/repository device+inode, exact member source/work/workflow and
  complete registration inventory; foreign aliases and incomplete enrollments fail
  closed. All-member receipt/process/hook checks and final registration/family/
  observation predicates now protect cleanup; retained tombstones cover every
  member. Existing 14 tests passed, then three new regressions passed with the full
  17-test target in 23.90s (`/private/tmp/kegari-v0-directory-family-regressions-rerun.log`).
  An initial compile failed because three fixtures called the no-argument
  constructor with `false`; that was corrected without weakening assertions
  (`...-directory-family-regressions.log`). Independent review found unlocked
  process/retention read-modify-write, a legacy final-gate missing registration
  digest, and cleanup's stale initial hook-ledger copy. Registration updates now
  serialize with Control/cleanup and each other. The real protected worker ledger
  is registered by immutable directory identity and reread, including later
  sessions/hooks; native before_remove uses that same ledger. Missing/replaced
  registered ledgers never fall back to the old copy. Five additional integration
  regressions passed with the complete 22-test target
  in 26.61s (`/private/tmp/kegari-v0-housekeeping-family-final-reattachment.log`).
  This includes removed canonical-reference refusal, changed-location denial
  without writes and exact original reattachment. Initial concurrency fixture
  setup used the host Git path and failed strict pin ownership; it now uses the
  actual pinned fixture binary. No host rule changed. The first combined compile
  exposed private sibling helper visibility and a peer fixture closure lifetime; owners corrected these.
  Opaque unchanged-source/family proofs and typed serialized enrollment are now
  implemented. Four actual host/source tests pass in 37.11s and two actual family
  enrollment tests in 34.16s (`/private/tmp/kegari-v0-git-reuse-host-final.log`,
  `...-git-reuse-family-tests-4.log`). They use actual scoped Git and Control, with
  explicitly synthetic terminal metadata and no joined-runtime cache. They cover
  exact/replayed enrollment, ordinary-admission denial, inherited creation, unknown
  ledger refusal and source changes invalidating both replay and enrolled proofs.
  The shared-family cleanup/tombstone positive passed 1/1 in 51.38s
  (`/private/tmp/kegari-v0-git-reuse-family-cleanup.log`): stale tokens fail
  before receipts both while the successor is active and after retirement; a fresh
  plan retains exact source and marks both operation tombstones removed. Strict
  library/tests Clippy passed in 9.32s (`...-git-reuse-family-final-clippy-2.log`). Native scheduler
  selection is not connected; a full native resumed run is not claimed. Root owns
  the family/hook implementation;
  delivery owns source-proof/family component tests and new housekeeping regressions
  during this phase.
- **Exact-source review retry increment:** actual Git/Control/native-wire component
  passed 1/1 in 28.79s, all continuation tests 15/15 in 46.47s and delivery tests
  21/21 in 17.39s. Logs are `/private/tmp/kegari-review-retry-final-test.log`,
  `...-all-cycles.log` and `...-delivery-tests.log`. Independent producer/runtime/
  publication review found no confirmed defect. The initial fixture omitted the
  native grant's required supporting link; it was corrected rather than weakening
  publication. Full native joined-runtime success remains unverified; no fake
  joined-cache entry or confined execution proof was added.
- **Metadata recovery increment:** typed hook outcomes and actual input provenance,
  opaque full-history recovery, finite distinct metadata admission and guarded
  initialize/account/config/thread-read wrapper are implemented. Worker/protocol
  tests passed 68/68 (37+31), Control metadata tests 2/2, plus full history and
  handshake scenarios; current-cap/fencing/owned disposable-process timeout tests
  passed. Logs use `/private/tmp/kegari-v0-recovery-*` and
  `/private/tmp/kegari-v0-resume-metadata-tests.log`. Root's bounded read found no
  confirmed defect. No live Codex/model was launched. Metadata recovery itself grants no model
  permit; the subsequent sealed bridge described below consumes actual retained
  completion, exact instructions, provider proof and directory enrollment.
- **Native resume consumer in progress:** `runtime_resume.rs` rechecks the actual
  retained completion Arc, exact metadata receipt, source-family proof, native
  authority/grants/provider holds, shared capacity, minimum original caps, failure
  backoff and consumed review rounds under the common final admission gate.
  `runtime_workers::prepare_resumed` performs real assembly, enrollment, new-bound
  execution/services, canonical inherited hook ledger and sealed resume options;
  it never reconstructs the original completion from JSON. Independent bounded
  review found no confirmed new authority defect. The first compile had an extra
  `RunServices.control` field; corrected to existing `WorkerServices.control`.
  A mistaken Control filter selected zero tests and is not counted; the corrected
  `control::session_resume::tests` command passed all three in 2.35s, log
  `/private/tmp/kegari-v0-resume-control-three-current.log`. Source/filesystem proof
  and family replay checks were strengthened after review. Automatic selection,
  repeated resumed-session prefix recovery, configured before/after hook support,
  cold recovery and genuine native qualification remain unfinished. Strict
  library/tests Clippy passed in 15.39s (`/private/tmp/kegari-v0-resume-bridge-clippy.log`).
  A root all-target run encountered in-flight fixture HTTP type errors, corrected
  by the fixture owner; no new frozen all-target checkpoint or commit is claimed.
  The private resumed-wire component also passed 1/1, including fresh history
  readback/current prompt/new-session accounting and history/policy refusals
  (`/private/tmp/kegari-v0-resume-wire-bridge-tests.log`). It cannot mint native
  joined or admitted-directory capabilities. A final review found pre-first-turn
  resumed thread usage could be misattributed to the new operation; the owner is
  fixing baseline-only handling with explicit prior/unattributed usage evidence.
  This narrow correction must pass before the source freeze. Integrated review
  also found source bytes could change after session binding but before outbound
  thread/resume or first-turn start: existing protocol gates checked only current
  authority. The owner is retaining the enrolled source verifier through those
  serialized startup gates, with a no-outbound-frame regression, and releasing
  the initial source constraint only after actual first-turn dispatch so later
  legitimate source edits remain possible.
- **Chained integration work:** an actual A/B/C Git/control/native-wire fixture is
  in progress and stops at missing genuine integrated evidence. Independent review
  found unchanged deployment-free repositories omitted from recovery's complete
  accepted baseline inventory (owner fixed), and historical successful deployment
  reads incorrectly coupled to the current target index/current main (pending
  immutable history route). Transitive queue eligibility is restricted to verified
  signed lineage and an ephemeral view; durable ancestor holds are preserved.
  The actual chain completed every assertion in 198.15s, including original A
  contribution exactly once and C integrated-gate refusal, but asynchronous poll
  teardown raced fixture deletion and logged a background panic. That run is
  retained as noisy development evidence, not the final clean result. The owner
  is adding explicit abort/join and rerunning both successor fixtures. An earlier
  large unboxed test future overflowed the normal test-thread stack; the fixture
  now boxes stage futures, with no host/thread-stack setting change. Full chain
  recovery still lacks overwritten historical target proof lookup and terminal
  transitive hold completion projection; neither is claimed complete.
  The next combined successor rerun failed after concurrent Cargo relinking
  replaced the running test executable, so the actual macOS process-path probe
  returned absent; an observation poll also saw `unstable_observation` during
  signed publication. The fixture now retries only that bounded read failure,
  without treating it as a valid projection. Final test runs must use an exclusive
  Cargo window after all sources settle, preserving the strict process probe.
  Log `/private/tmp/kegari-v0-lineage-runtime-final.log` retains both failures.
- **First frozen focused verification:** all 561 code/test/instruction/protocol/
  fixture/Cargo inputs had `sha256:c8f5d7d5b934fec93da762706ab4ce4c9ed90f82c43a2b85a61c549466f9614a`.
  Protocol/recovery tests passed 2/2 in 0.04s and actual source/family/startup-gate
  tests passed 6/6 in 49.99s, sequentially with no competing Cargo relink. Logs
  `/private/tmp/kegari-v0-worker-release-recovery-protocol.log` and
  `...-families.log`; initial source inventory retained as `...-source-v1.json`.
  Before full validation, root found that delayed usage for a historical turn
  could still be charged after a new turn started: the pinned notification has a
  required turn ID, but accounting discarded it. This narrow per-turn attribution
  correction is saved; the first source hash is historical, not final. The sealed
  resume origin now retains the bounded historical turn-ID set and rejects reused
  IDs on start, acknowledgement and completion, preserving policy signals first.
  Unknown-turn usage remains explicitly unattributed even after a new turn starts.
  The updated focused protocol target passed 2/2 in 0.04s
  (`/private/tmp/kegari-v0-worker-release-recovery-protocol-v2.log`).
  The full-suite base 561-input hash was
  `sha256:49b29e829fa6b366f7661222929201d139a6bd211aee55f748fea64cb0099dc4`.
  The exclusive full suite passed **668 tests, zero failures, two native ignores**
  across 38 groups; library tests passed 184/184 in 246.61s, including both
  successor fixtures without teardown errors. Input hashes remained unchanged.
  Log `/private/tmp/kegari-v0-worker-release-recovery-full.log` has SHA-256
  `430eaeee0b66d27b7c5639b961c87369593da6c733e677b5bc6aac20a85a5a81`;
  the source inventory is retained as `...-source-v2.json`. The following two
  read-only review findings were corrected and verified separately below.
  The native evaluator also now rejects a resumed admission before binding because
  its current TrialOperation cannot carry the sealed resume mode. No fabricated
  capability or tautological guard test was added; relevant trial regressions
  passed in both the full base suite and final affected checks.
- **Final read-only review findings (resolved and checked):** while the frozen
  suite ran, Codex found a successful turn acknowledgement could reach a failing
  mutation-receipt write before persisting its actual policy signal. The saved
  fix preserves same-request/thread policy before acknowledgement bookkeeping,
  with a real duplex response and rejecting observation gate regression. Delivery
  also found that the first new-turn cumulative counter may contain unobserved
  prior usage: full history readback has no authenticated final token snapshot.
  The first resumed known-turn counter now establishes an explicitly unqualified
  boundary and retains ambiguous increases; only later known-turn deltas are
  assigned. Both corrections passed 122 affected checks on final source and
  independent rereview. Strict all-target Clippy passed in 18.66s. The complete
  base suite and final two-code-file delta have separate recorded identities.
- **Post-review fixture failure retained:** the first corrected-source protocol
  run passed 1/2 and failed the new acknowledgement-ordering fixture. Its parent
  Tokio test used the default single-thread runtime; production guarded dispatch
  correctly rejected it before transmission because it requires a multithread
  host. The fixture's uncertainty assertion hid that precise prerequisite error.
  The fixture was corrected to the actual supported runtime and reports
  scenario/error context without weakening dispatch. Its final rerun passed 2/2
  in 0.05s, followed by all other affected checks. Log
  `/private/tmp/kegari-v0-worker-release-recovery-protocol-final.log` and source
  `sha256:069b48cc37d2b8e63fa15d8388881b2dbd5eb1a4ed6a392344b84d8b23a094a1`
  are retained as a failed intermediate state; final affected checks then passed.
- **Validation process ownership:** all root-owned focused sessions above have
  completed. A read-only `ps` inventory was denied by the sandbox (`operation not
  permitted`); no escalation or alternate process-enumeration route was attempted.
  Use the tracked tool session completions and agents' explicit process handoffs.
  The full suite, failed and successful focused runs, final affected tests, lint
  and current evaluation-plan build all completed. Final sources remain frozen
  until root commits the reviewed checkpoint. No Cargo process remains owned by
  root or a delegated agent.

- **Earlier retry-checkpoint implementation history:** read-only review found that explicit retry
  skipped every completed phase, including an exhausted genuine repair obligation.
  The fix must retain the actual unfinished proof and bind a new explicit native
  retry grant; completed work cannot become generically retryable. A full runtime
  successor fixture is also in progress. Actual integrated-check production proofs
  remain unavailable behind the previously denied native sandbox/descendant
  prerequisites; the team will not repeat those probes or fabricate confined
  execution. Reachable actual Git/runtime assembly and fail-closed preservation
  remain independently testable.
- **Retry publication/finalization corrections:** independent review found that
  the real selector searched again for an ungranted phase after observing its
  own native grant. The postpublication path now verifies the exact signed current
  comment and unchanged prior-admission map. It does not let a stale cached
  publication-before-ACK error override complete fresh evidence; provider/session
  holds still gate this path. A second actual-control regression exposed two
  finalization races: the runtime can already consume the grant, and its unknown
  effect then blocks a generic metadata acknowledgement. The final predicate
  permits only the exact granted phase's admissions/effects. A narrow local
  completion path verifies exact command/publication lineage, records only fixed
  acknowledgement metadata and leaves successor holds intact. The first new test
  failed on generic overlap; the next failed on mismatched provider versus work
  target identity. Both were corrected; the exact positive/wrong-budget regression
  passes (`/private/tmp/kegari-v0-retry-start-race-native-target.log`). Original
  failures remain in `...-retry-start-race-exact.log` and
  `...-retry-start-race-local-ack.log`. The initial partial-name command with
  `--exact` selected zero tests and is not counted. All 14 preexisting decisions
  had passed earlier. Independent review then found the earlier preparation
  interval also needed exact provenance. A new typed durable preparation reference
  binds worker operation, phase/budget, workspace and account to its canonical
  preparation admission; it grants no authority by itself. The actual runtime
  records it, and finalization recognizes only matching preparation/model work.
  All 15 decisions and 25 evaluation integration tests pass, including the
  preparation/model × exact/wrong-budget race regression
  (`/private/tmp/kegari-v0-retry-evaluation-review-regressions.log`). Root review
  then found that the advisory schedule could become stale before preparation
  admission. The worker agent is adding the exact current retry/continuation
  planning predicate inside serialized preparation admission without lock reentry.
  The exact current attempt, caps, trigger and spacing must still match. Actual
  native grant deletion/edit tests pass and leave no preparation receipt, permit
  or effect (`/private/tmp/kegari-v0-retry-preparation-native-tests.log`, 1 test,
  44.39s). Previously admitted preparation still drains under retained authority.
  The genuine fixture's postconsumption assertion was initially misplaced before
  consumption; it is now after the actual repair admission and passes. Earlier
  focused results did not establish that specific timing assertion.
- **Explicit retry of genuine unfinished work:** producer/consumer now retain
  the actual unfinished proof and bind its original completed implementation,
  failed checks/reviews, original admission map, exact current native grant and
  fresh first admission. Ordinary completed work is still not generically
  retryable. Two actual delivery repair cycles and four selector tests pass;
  source edits/deleted grants and consumed proofs are rejected. Logs:
  `/private/tmp/kegari-v0-explicit-retry-cycle-tests.log` and
  `/private/tmp/kegari-v0-explicit-retry-selector-tests.log`. Mixed phase budgets,
  cross-correction and cold proof recovery remain incomplete. These focused runs
  precede the latest root finalization changes and will be covered again by the
  combined checkpoint.
- **Reachable runtime successor fixture:** one expanded production-path fixture
  passed in 164.38s, plus 11 shared release adapter and 13 correction regressions.
  Actual command publications/admissions, signed correction routing, retained
  predecessor recovery, original/corrected Git candidate assembly and complete
  vectors are verified. Historical parent/check/retained-plan seed inputs are
  explicitly synthetic. Missing genuine integrated execution blocks progress,
  preserves the unknown hold and produces zero PR/merge/production dispatch.
  This does not establish full partial-merge/multi-target supersede, carried
  acceptance, deployment-only execution or chained integrations. Logs:
  `/private/tmp/kegari-v0-runtime-successor-fixture.log`,
  `/private/tmp/kegari-v0-successor-release-regressions.log`, and
  `/private/tmp/kegari-v0-successor-correction-regressions.log`. Focused strict
  Clippy passed. A one-off read-only process-status diagnostic was sandbox-denied;
  it was not retried or escalated. No denied native probe was repeated.
- **Resume prerequisites:** actual instruction assemblies now retain exact
  compatibility provenance; only typed operation/attempt metadata can normalize,
  while rendered workflow and all effective instructions remain exact. Provider
  journals retain ordered complete terminal-turn fingerprints and interrupted
  known turn IDs. Three assembly and 29 protocol tests pass. These are
  prerequisites, not a working resumed provider session: metadata-only recovery,
  reused owned source/cleanup and atomic new-admission composition remain undone.
  Independent review found malformed provider `itemsView` values could normalize
  to full history and a late/duplicate policy notification could lose intervention
  provenance. The adapter now rejects invalid present history views, classifies
  actual same-thread policy intervention before ordinary history errors, and
  durably checkpoints that hold on either request/pump error path. Protocol 31,
  assembly 3, control/storage 24 and release-control 5 tests pass, including legacy
  record serialization and actual retired preparation rejection as release origin.
  The release adapter's 11 tests and strict lib/test Clippy also pass. Logs:
  `/private/tmp/kegari-v0-resume-review-regressions.log`,
  `/private/tmp/kegari-v0-release-preparation-isolation.log`,
  `/private/tmp/kegari-v0-release-typed-admission-regressions.log`, and
  `/private/tmp/kegari-v0-resume-review-clippy.log`.
- **Owned evaluator placement in progress:** source preparation now binds its
  scratch token to one anticipated operation; binding consumes the actual admitted
  worker contract and compares exact seed/clone bytes, modes, full context,
  workflow, declared scope and scorer boundaries. Binding grants no live authority
  and cannot occur after worker start. Library compilation passed; the first
  new focused test compilation stopped on missing imports in a concurrently
  extracted shared delivery fixture, before placement assertions. A subsequent compile hit an in-flight release
  seed helper; both failures are retained in the original/rerun logs. Once those
  sources compiled, all five placement tests passed against actual disposable
  Git repositories (`/private/tmp/kegari-v0-owned-fixture-tests-current.log`).
  The evaluator integration suite passed all 25 tests, including a new private
  grading executable rejection (`/private/tmp/kegari-v0-owned-fixture-integration.log`).
  Source compilation is `/private/tmp/kegari-v0-owned-fixture-check.log`.
  The earlier strict lint attempts stopped on concurrently changing retry/release
  helpers; later agent strict lib/test checks passed. A complete frozen check of
  all targets with the newest root changes remains pending. A later independent
  review found private host instruction discovery and undeclared scratch input
  paths. Actual auth/workflow/supplied/discovered instruction paths now exclude
  grader/report roots, and current declarations require truly empty scratch at
  binding and before trial hooks. All seven placement tests pass, including an
  actual assembled global-guidance canary and scratch contamination regressions
  (`/private/tmp/kegari-v0-owned-fixture-review-fixes.log`). Root then extended
  private instruction checks to reject hardlinked inputs, with an actual private
  rubric/global guidance alias regression (`...-owned-fixture-private-links.log`).
  Independent rereview
  found no remaining material issue in those paths or exact preparation retry
  lineage. These are bounded code/test reviews, not native qualification.

- **Continuation checkpoint admission:** the host now records the run and its
  unfinished-work reference in one serialized admission. It requires the exact
  latest reconciled/exited predecessor, an actual same-work failure admission,
  one-second spacing, unused trigger/material-progress digests, retained minimum
  attempt/turn/review limits and monotone consumed review rounds. Four new
  actual-journal tests pass, including concurrent predecessor consumption,
  callback refusal without an admission, restart retention and unknown-effect
  preservation. The existing control/storage suite also passes (24 tests).
  Logs: `/private/tmp/kegari-v0-continuation-control.log` and
  `/private/tmp/kegari-v0-continuation-control-storage.log`. Initial compilation
  exposed a module-visibility error and two concurrently edited fixture
  constructors; both were corrected before these runs. Producer/consumer
  integration, independent review and combined checks remain in progress.
  Independent worker-agent review found no material defect in that atomic seam.
  A new positive producer fixture subsequently exposed two integration defects:
  completed validator/reviewer phases need a distinct verified changed-source
  trigger after repair, and raw delivery task payloads were replacing the full
  native instruction context. The team is correcting both without resetting
  phase budgets or weakening assembly. Root added the distinct atomic
  `SourceAdvancedReference` path and authored its journal regression; the next
  test runs stopped at concurrently changing release enum matches and the new
  assignment envelope method before assertions. A subsequent coherent run passed
  all five control continuation/follow-up tests, including the final-gate check
  that a newer implementation admission invalidates an older source trigger
  (`/private/tmp/kegari-v0-source-followup-control.log`). Broader pipeline and
  full-checkpoint validation subsequently passed as recorded above.
- **Continuation checkpoint evaluation corrections:** the outer runner now limits the whole
  executor (including preparation/admission) to the lesser of the declared trial
  duration and remaining suite time. Its new cancellation/retained-report
  regression passed with the evaluation suite (23 tests, log
  `/private/tmp/kegari-v0-continuation-evaluation.log`). A declared native context
  binding is now implemented and library compilation passed: versioned sorted
  typed slots come from the actual assembly identities, and all remaining native
  context must match exactly. The declaration is included in the fixture hash
  and private preparation seal. Two matcher tests now pass, and the evaluation
  suite passes all 24 tests including production assembly/file provenance
  (`/private/tmp/kegari-v0-native-context-tests.log` and
  `/private/tmp/kegari-v0-native-context-evaluation.log`). The positive assembly
  test first failed on an unsupported top-level `native_context`; production and
  fixture context now place it inside the documented `evidence` object. A separate
  positive producer fixture found the public generation `authorization` digest
  collided with secret-field rejection; the team is adding a typed public
  provenance projection while retaining that rejection. Full
  suite migration, owned-clone binding, live authority/CLI composition and actual
  host-result projections remain engineering work. This is not live evidence.
  Independent review then identified two additional fidelity defects: the native
  trial path omitted actual phase versus declared variant equality, and fixture
  bytes could change between the initial source hash and later copying/decoding.
  The shared phase preflight and substitution regression are now implemented;
  immutable bounded fixture capture is also implemented. Six capture tests and
  all 24 evaluation integration tests pass on that source, with strict selected
  lint and formatting. Capture verifies one descriptor's bounded bytes/modes,
  directory entry/depth limits and the final reset against the captured inventory.
  Changed-and-restored source, missing native binding, wrong copied bytes/mode
  and extra files have regressions. Logs are
  `/private/tmp/kegari-v0-fixture-capture-tests.log`,
  `/private/tmp/kegari-v0-evaluation-capture-integration.log` and
  `/private/tmp/kegari-v0-fixture-capture-clippy.log`.
- **Current no-release implementation and review:** already-production completion
  now has a whole-parent immutable signed intent, a real finite metadata admission,
  current native parent/acceptance references, complete current production source
  and target observations, broker publication and exact native comment/receipt
  readback. Current proofs enter dependency and release queue caches; history/show
  metadata is separate. An owned task retires only its joined publication while
  preserving unknown provider holds. The shared metadata planner gives known-not-sent
  attempts the original finite cap/backoff and never replays unknown effects. Three
  planner regressions pass. Independent review found stale no-release outcome/link
  retention after acceptance replacement and a provider-target/work-key mismatch;
  both are corrected. The actual positive runtime fixture passes. Narrow explicit
  publication/admission reconciliation is implemented but its authorized-decision
  failure/recovery regression is still pending. Negative native record/baseline
  fixtures and combined verification are in progress. Earlier compilation stopped
  on two new test-fixture argument types; the fixture owner corrected them.
- **Executable/recovery checkpoint candidates:** the concrete delivery decision
  implementation now handles queue, resume, abandon, resolve, supersede and exact
  operation reconciliation. Main/CLI/local CLI tests pass 3/24/10. Eighteen release
  unit scenarios pass, including actual adapter source/PR/merge and old owned-merge
  production recovery without relabeling current main. The historical runtime
  fallback is wired but a complete positive runtime fixture is outstanding.
  Independent review found no new material bypass in bounded successor-intent
  consumption. A separate review found nonempty `RepositoryId` map keys could not
  serialize to JSON; two retained release/preparation maps now use sorted tuple
  arrays with bounds, identity and duplicate/order checks. Their actual signed
  private-directory roundtrip regression passes. The first roundtrip fixture used
  an uncanonicalized temporary path and failed before assertions; the canonical
  owned path passed. Deployment-only successor representation remains future work.
- **Correction/import checkpoint candidates:** affected protocol (28), worker
  (34) and evaluation (22) suites pass; correction-filter (15), corrected claim (2)
  and ACK race (1) tests pass separately. Protocol ACK observation preserves its
  exact original effect during a new correction, and classification waiting uses
  the original deadline with source/effect gates closed. Imported Git sources have
  17 passing parser/acquisition/retention/clone cases, including changed/removed
  receipts, changed binary/cache bytes, STOPPED missing-cache reads with no network
  or creation, full fork identity and exact retained per-delta root selection.
  The queued manual-release identity regression passes. No live provider was used.
  All registered source edits are frozen for a fresh full run; targeted lint
  results do not yet establish a combined checkpoint. The 525 source/fixture/Cargo
  inputs have digest `sha256:d8ecbd165a7320a864934c4eea40451e0ed1d80ebdd78d334d08f3f3ea2146e9`.
  Full tests passed (577/0/2, 36 groups, Cargo exit 0) in
  `/private/tmp/kegari-v0-correction-recovery-full.log`; strict all-target Clippy passed (exit 0) with evidence in
  `/private/tmp/kegari-v0-correction-recovery-clippy.log`. Full formatting, contract
  inventory (363/136) and diff whitespace checks also passed.
  Source hashes are `/private/tmp/kegari-v0-correction-recovery-source.json`.
  The evaluation plan CLI reports 56 declared slots, suite digest
  `sha256:v1:e59fa4a3e35486210374a57511a54033e6a1a334a7a728be78453d83b57647d5`,
  and `dispatch_authorized: false`; no live trial was launched. The first manual
  invocation used an unsupported `--suite` option and returned usage/exit 2; the
  documented positional manifest invocation then passed. The full suite exited
  successfully and the exact source hash matched afterward. All owned check
  processes have finished; no uncertain live side effect was created.

- **Post-checkpoint recovery integration:** new release resumes use a distinct
  actual admission that binds the immutable original plan, exact predecessor,
  acknowledged decision authorization and monotone ordinal. A serialized check
  rejects active/unknown ownership, stale acceptance, paused intent, reused
  commands and forked predecessors. Four control regressions pass. Recording
  local recovery intent can coexist with unknown effects but neither clears
  their holds nor grants a permit. The normal control/storage (24) and decision
  (13) suites pass after this change. An additional pending-correction recovery
  authorization regression passes, as does the release agent's actual stopped
  DecisionManager-to-abandon completion regression. The closed completion path
  rejects an authorization receipt presented as a completed recovery action.
  Operation reconciliation has the same closed metadata completion rule: it
  requires the exact actual Reconciled target and can finish while STOPPED
  without changing a different runtime-wide Sent hold. Its new regression also
  rejects an unknown target and altered result; all four control cases pass.
  The first new unit-test compile
  failed on a moved fixture string; cloning that test value fixed it without
  changing assertions. Step dispatch now carries exact attempt/prerequisite
  ordinals (release agent: 9 adapter and 4 backend checks pass). Correction
  publisher regressions verify exact signed predecessor chains and edited-source
  races (2 pass); live steering integration remains in flight. A disposable
  protocol-v2 probe used real local Git to fetch/verify nine objects and preserve
  dirty source/index/config/refs; it exercised no remote service or credentials.
  Bounded acquisition implementation and actual runtime wiring remain in flight.
- **Post-checkpoint evaluator fidelity review:** the shared admitted-worker path
  validated its actual contract and permissions but did not repeat direct
  assembly's exact declared task-context check. The correction applies the same
  check to both paths, seals prepared fixture input/prior-state/baseline identity,
  and compares the complete supplied variant with the suite declaration before
  live admission. Independent review found nested hint fields could still enter
  the host-added workspace/operation context; complete canonical context equality
  now rejects them. Scope grading now counts nested `.git` paths instead of
  silently excluding them. The targeted evaluation integration suite passes 22
  tests and the two evaluation unit tests pass. Suite version 2 and grader
  `kegari-observable-v2` explicitly version these changed rules; all 28 scorer
  digests are updated, while earlier results remain historical and require
  regrading for comparison. The versioned evaluation/report suites pass 22/9.
  The first unit rerun stopped at two
  in-flight release compilation errors, then passed after the release agent's
  corrections. The normative coverage ledger now records partial evaluation
  implementation/deterministic evidence separately from unrun actual-agent and
  human evidence. This does not solve the
  separately outstanding symbolic-fixture-to-native-context composition and
  authorizes no live trial.
- **Current affected recovery rerun:** control/storage (24), decisions (14) and
  evaluation (22) pass together, including the shared correction-loop code at
  that build. An earlier run stopped before assertions on in-flight correction
  execution-binding visibility and broker methods; the coherent-source rerun
  passed after the owning agents completed those interfaces. These targeted
  results do not replace the frozen 523-test full-crate checkpoint. Independent
  review found no new material bypass in either closed recovery-completion path.
- **Current independent recovery/correction findings:** root review found that
  reconciling the original release admission could select newer resumed steps,
  and resume could retire a healthy between-step plan before proving a held or
  terminal state. The release agent accepted both findings and is adding scope
  and ownership regressions. Correction review found an installed epoch's
  pending-classification error could abort an active worker on the next comment,
  and freshness checks on an already received protocol response could turn it
  into an unresolved acknowledgement. The worker agent is separating bounded
  waiting/observation from permission to send new effects; ordinary effect gates
  must remain closed throughout pending classification. These findings remain
  unresolved until their affected tests pass.
- **Remote object acquisition evidence:** the delivery agent reports 11 passing
  offline acquisition/parser tests, including actual protocol-v2 upload-pack,
  strict pinned index-pack, binary/mode/symlink preservation, missing ancestry,
  expansion/depth bounds, changed refs/authority, cache tampering, fork PR identity
  and two-root squash/delta verification. The correction-aware execution bridge
  and 18 execution integration tests pass. Native staging and parent assembly
  now use admitted acquisition; worker wiring and read-only restart discovery
  remain in flight. No remote server or credential was used by these tests.
- **Frozen progression verification passed:** the full crate run completed with
  523 passed, zero failed and two native qualifications ignored (35 groups,
  Cargo exit 0). Strict all-target Clippy, formatting, inventory and diff checks
  passed. All 497 source/test/fixture inputs retain digest
  `sha256:60831f1db54e9807d560ae3d9a452a8d9a6d59301f409f7e1bb093f6610f8ac1`.
  The [durable check record](validation/progression-checks.json) includes group
  counts and the log digest. Raw local evidence is in
  `/private/tmp/kegari-v0-progression-full.log` and
  `/private/tmp/kegari-v0-progression-clippy.log`; input hashes are in
  `/private/tmp/kegari-v0-progression-source.json`.
  The built evaluation CLI reports all 56 declared slots with dispatch disabled.
  An initial manual invocation used nonexistent `suite.json` and correctly
  returned `io_error`; the actual `evaluations/manifest.json` invocation exited 0.
  No trial was launched. The final release callback rejects changed acceptance
  token/digest/time without an admission receipt. Failed provider-read invalidation
  shares admission serialization. Known terminal phase failures retire only
  proven finished local work, preserve the error/original operation, and grant no
  retry; unknown effects remain held. Release regressions cover receipt failure,
  restart/retirement, corrected-token preparations, global acceptance ordering
  and native evidence replacement. This milestone remains separate from the
  previous control/worker composition checkpoint because it adds independently
  reviewable provider delivery, process ownership and recovery behavior. No
  history rewrite or consolidation is planned.
- **Current full-run failure and review:** the first post-checkpoint full suite
  stopped in deployment after 9/10 cases passed: `verify_existing_target`
  compared a refreshed observation timestamp as if it were source identity.
  The correction normalizes only that timestamp and retains exact provider,
  repository, source/tree, artifact and target identity; deployment (10),
  provider (28) and release-wire (8) checks then passed. A fresh full run passed
  503 tests with two native ignores (35 completed test groups, Cargo exit 0),
  and strict all-target Clippy, formatting and inventory passed for that
  pre-review snapshot. Subsequent corrections require affected checks again.
  Independent review also found impossible zero-turn successful agent archives,
  missing input/permission/evidence-class comparison conditions, and historical
  reconstructed grader-version substitution. Those report fixes pass 8 archive
  and 20 evaluation tests, with independent rereview confirming the changes.
  Delivery review found the designated E2E candidate-preparation
  cycle, parent generation replacement deadlock, pre-admission launch retry
  deadlock and admitted-phase draining gaps. Their focused corrections are
  being implemented; no completed parent/runtime scenario is claimed.
- **Current review regression results:** cleanup's previous final metadata check
  could miss same-inode, same-size content edits or mode changes. Descriptor
  hashing outside the control lock and exact descriptor/path/time verification
  at unlink now preserve modified files; 2 unit tests cover four mutation cases
  and positive unlink, and all 14 housekeeping integration tests plus strict
  selected Clippy pass. The first fixture run failed on `/var` versus `/private`
  path canonicalization before reaching the regression and was corrected.
  Capacity preflight now checks real occupying receipts before preparation while
  retaining final atomic worker admission. An observed admission callback binds
  the actual serialized control generation. Control/storage (24), scheduling
  (7), evaluation (20) and archive (9) tests pass together. Archive grades also
  require every declared predicate exactly once; missing historical timestamps
  and build provenance remain unavailable rather than invented.
  The first scheduling rerun stopped at an in-flight missing parent E2E helper;
  later cleanup builds also stopped at a missing snapshot-release wrapper and
  release-backend borrow/serialization errors. These were compile failures, not
  test results; coherent-source reruns are reported separately. Further review
  found final new-worker admission could reuse drain-only authority after filter
  exclusion, and quota observations could race serialized admission. Both fixes
  pass the six current runtime unit tests, including two new regressions using
  actual ControlHost admission and deterministic ordering channels.
- **Read-only evaluation report tooling:** `kegari-v0-evaluation plan/inspect/compare`
  now provides executable local inspection and version-aware comparison commands.
  The affected evaluation suite (20) and new report/archive/actual-CLI tests (6)
  pass. Missing started results remain interrupted/unknown; altered criteria are
  incomparable; later passes do not resolve earlier failures. Report writes now
  use owner-only descriptor-relative exclusive creation, and replacement/link/
  corruption cases are retained as regressions. Initial compile exposed a private
  directory-revalidation visibility error, corrected with crate-only access.
  This tool does not launch live trials or satisfy human review; concrete live
  fixture/authority composition is still outstanding. Strict lint is in flight.
- **Harness fidelity review correction:** live evaluation previously bypassed the
  guarded provider startup path and projected the original Git commit as a
  filesystem candidate digest. It now requires the actual scoped worker's opaque
  journaled launch capability and request gate. Observations hash actual files,
  retain each check's original candidate, and verify private retained artifact
  bytes. The evaluation suite passed 20 tests; the affected worker suite passed
  32 after adding stale-check and corrupted-archive regressions. The first new
  stale-check regression incorrectly expected a dirty-source rerun to succeed;
  the production exact-revision gate rejected it with `dirty_execution`. The
  regression now explicitly verifies that rejection and continued ineligibility.
  The current shared-launch worker/evaluation/host-process rerun passes 54 tests
  (33/20/1). An outer-suite deadline could previously drop the trial before its
  explicit cancellation block; a new guard fences the admitted operation before
  provider-handle destruction. Its actual-control/drop-order regression passes
  (1), without claiming an actual provider or native containment run.
  Guarded live-run composition still requires affected checks and a concrete
  authorized evaluation runner; no actual trial was launched.
- **Current integration review corrections:** protected worker outcomes now require
  the latest completed protocol turn, exact bound contract/manifest, contiguous
  journal and matching correction watermark. Starting or failing a later turn
  invalidates an older completed claim (4 claim regressions pass). Delivery and
  dependency caches recheck their 30-second evidence age, complete current provider
  projection, native source digest, required-child set, corrections and current
  monotone permission fences on every read. Acceptance attaches to the actual
  sealed snapshot without refreshing its observation age (4 runtime tests pass).
- **Finite authorization preparation:** metadata admissions now atomically retain
  original attempt bounds independently of model sessions. A next publication ID
  requires actual local retirement, a proven never-sent effect, retryable failure
  and backoff; unknown writes retain the original request. The 2 planner tests and
  22 control/storage tests pass, including restart/cap-change cases. The real-wire
  authorization fixture passes all 9 scenarios: exact publication, lost reply,
  STOPPED recovery, missing/edited/duplicate native records, owner loss/revocation,
  and one preflight retry with exactly one mutation. The latest 23-test
  control/storage run also verifies that reconciliation sees an intervening stop
  under the control lock, preserves a rejected unknown receipt, and resolves
  only a fresh proof while STOPPED. The first new regression failed to compile
  because IntentRecord has no PartialEq; it now compares the intended fields.
  Read-only native recovery
  never retransmits a missing result.
- **Delegated latest increments:** correction publication and prefix/final recovery
  pass 10 tests; deployment baseline tests pass 10; authenticated Linear asset
  association/readback is included in 28 provider tests. Latest selected
  configuration/authority/reconciliation suites pass 65. Host-local metadata
  permits and process journals bind actual registered control ownership separately
  from worker leases; selected host/process tests pass, but native containment and
  strict installed-Git prerequisites remain unresolved. These overlapping targeted
  results are not summed into a new full-suite count. The executable factory,
  production phase graph and final integrated checks are still in flight.
- Baseline `git status --porcelain=v2` and `git diff --cached --stat`: empty. `shasum -a 256 crates/kegari-v0/SPEC.md`: approved digest. Entire SPEC read before implementation.
- `rustc --version`, `cargo --version`, `cargo fmt --version`, `cargo clippy --version`, `git --version`, `codex --version`, app-server/schema help inspected. CLI emits a PATH-alias write warning under sandbox; no host changes attempted.
- Current official [Codex app-server documentation](https://learn.chatgpt.com/docs/app-server) retrieved for protocol context. Installed generated schema and actual bounded tests, not current docs alone, will decide supported-profile compatibility.
- `cargo check --locked -p kegari-v0`: passed for registered foundation modules. `cargo test --locked -p kegari-v0 --lib --test authority_scheduler --test control_storage --test config_harness --test codex_protocol --test snapshots --test git_delivery`: 87 passed, zero failed/ignored; all use the explicit environment above. Strict Clippy for those targets passed after refactoring initial lint findings. Formatting passed after sorting module declarations. Upcoming source changes require affected checks again.
- [Local foundation report](validation/LOCAL-FOUNDATIONS.md) records findings, fixes, commands and limits. [Contract coverage inventory](validation/coverage.json) covers 363 normative prose/table blocks and all 136 named REQ/SC/V0-T/V0-E/V0-H identifiers. `tools/contract_inventory.py --check` passes; inventory is not conformance proof.
- Independent review produced and corrected mutation-rejection replay, recovered-work duplication, unresolved-target retry, ambiguous correction, stale turn completion, scope/definition conflation, YAML type coercion, release-plan omissions, extra selected deltas, unverified carry and lost failed-step history. Their regressions remain. Further review found omission of mandatory context in a plain-text workflow and scratch/source overlap in assembly: both are now corrected, with the affected config/harness suite rerun (28 passes), bringing current total to 89.
- Further protocol review found missing list/operation pagination continuations, weak optional-filter typing and unchecked GET bodies. All three were corrected; the 8 API tests include full 501-child projection traversal, invalid filters, duplicate query fields and oversized GET bodies. The 2 secret/storage tests use a clearly identified in-memory secret fixture: native macOS Keychain calls compile but have **not** been invoked or qualified.
- Build provenance now includes a deterministic source/asset/dependency-lock digest in instruction manifests and evaluation reports. Raw private TOML reads/writes retain the descriptor-relative ownership, symlink and durability checks. The affected storage/control and instruction suites were rerun (12 and 28 passes).
- Platform's first native run failed because `kern.bootsessionuuid` is denied by the outer sandbox (`Operation not permitted`, confirmed by the read-only system query). No bypass was attempted. Deterministic registration tests now inject a labeled process observer through the production ownership code: 4 pass. Two native ownership/process-group cases remain explicitly ignored/blocked, not passes. Actual Keychain, service installation and process-supervision qualification have not run.
- The transport increment passes 10 tests, including an actual Unix socket request with OS peer identity and a disposable-CA rustls handshake. Certificate/identity/revocation/reload checks are covered locally. The configuration suite now has 29 passes after requiring stable numeric Tailscale UserProfile IDs. These do not qualify a real Tailscale enrollment or deployed certificate.
- Runtime wiring review found two additional platform gaps: an unresolved pre-handshake startup had no safe recovery path, and the registration object could outlive its control lock. Pending launches now retain their original launcher identity and require complete process/lock inspection before replacement; registration and control share the live lock descriptor. The platform fixture suite now passes 5 cases with 2 native cases still blocked. Three inherited-channel tests pass for bounded framing, cancelled partial reads and stale/contradictory lifecycle state. The actual child supervisor still requires native qualification and the executable host loop is not yet wired.
- **Corrected delivery integration finding:** the original review binding included the future staging squash SHA, making review-before-merge impossible. Review evidence now uses a separate pre-integration digest; staged artifacts retain their full identity. Root reran all 28 actual-Git/evidence fixtures successfully, including a second-squash/same-tree regression, replacement mapping, authenticated partial-production recovery and no-release-required accounting. Complete provider-backed delivery remains in progress.
- **Corrected local-execution integration finding:** a worker check initially held the control mutex while waiting for the process. `begin_effect` now persists a Sent receipt and launches/registers the owned process under the mutex, then releases it for the wait; `complete_effect` rejects evidence after fencing/revocation. The worker regression observes responsive status during a blocked check, refuses cancellation before durable force-stop, then cancels the owned handle and rejects its late success. Root control regression separately verifies the retained receipt and cleared local hold.
- **Corrected actor-binding integration finding:** retaining only operator display ID/roles lost the original Unix/Tailscale principal during asynchronous preflight. Actors now retain the verified transport principal; mutation receipt digests bind it and current mapping is rechecked inside serialized configuration/lifecycle gates. The dispatcher regression rejects a cached actor after principal reassignment.
- The shared dispatcher has 4 passing tests through authenticated API composition: STOPPED inspection creates no receipts/work; start remains reconciling until admission actually opens; a reassigned principal cannot reuse its old role; service-stop reservation fences subsequent start. Native service persistence/installation and actual process supervision are still being implemented, not qualified.
- Root validation: `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --test app --locked` (4 pass); affected API/control run (10/15 pass), prior platform/channel/connection run (5 pass +2 blocked /3/2 pass). An initial new test failed to compile because a deliberately panicking launch callback needed an explicit generic type; corrected without changing its assertion. Current strict full lint remains pending while delivery/configuration work is in flight.
- Root checkpoint validation: the frozen Git/evidence/release files pass 28 `git_delivery` tests, selected-file `rustfmt --check`, and `git diff --check`. This increment adds canonical signed-publication framing, pre-merge review identity, explicit replacement contribution inventories, authenticated production-integration recovery, no-release-required proofs and read-only manual-release eligibility. It is kept separate from the earlier nonadjacent core checkpoint because it is a new independently reviewable contract correction with intervening provider/harness commits; no history rewrite or consolidation is needed.
- Dispatcher/recovery corrections now pass 6 application, 17 control/storage and 15 reconciliation tests: background configuration work survives HTTP timeout, active work cannot be prematurely reconciled, snapshot continuations preserve original metadata, and recovery retains required-child identities while re-reading signed provider authority. The initial observation cache remains explicitly pending, and failed refreshes cannot authorize admission.
- The bounded artifact archive reader uses exact `zip` 9.1.0 with the Rust deflate backend, locked with its dependencies. Package metadata and primary crate documentation were checked; archives are bounded and parsed in memory, never extracted into an execution directory.
- **Corrected provider responsiveness and cancellation finding:** provider preflight
  and response waits no longer retain the control mutex. A unique durable
  Prepared reservation precedes asynchronous reads; current gates run before
  reads and before serialized Sent/owned-task dispatch. Actual network tasks
  survive caller aborts, cannot be prematurely reconciled or service-stopped,
  and preserve unknown receipts. The two new offline wire regressions pass as
  part of 23 provider tests. One first run failed only because the new test
  asserted `status_id` instead of the adapter's established `record_id`; the
  assertion was corrected to the real receipt schema, retaining all behavior
  checks. Decision callbacks were also changed to prepare complete observations
  asynchronously and use cache-only final checks; observation publication now
  serializes with dispatch.
- **Canonical ownership and active reload corrections:** canonical issue/PR
  claims now remain stable across generations and filter/profile edits. Durable
  phase identity still binds exact work generations and budgets. Atomic run
  admission counts actual active receipts under the control lock, including
  global, account, workspace and normalized-state limits; the concurrent
  capacity regression passes. Opaque admitted settings snapshots retain original
  workflow/profile settings while re-reading current permissions and provider
  facts. The 21 reconciliation and 17 configuration tests include ordinary reload,
  invalid future workflow, actor/path/access revocation, known reductions in an
  otherwise invalid proposal, persistence/restart, and unknown-effect holds.
  Independent-review drain and final runtime integration checks remain in flight.
- **Dispatcher permission regression:** retained cached transport roles cannot
  survive disabling HTTPS or a known monotone role reduction. Both tests pass
  through the real dispatcher; unchanged inspect authority remains usable.
- **Owned-process finding and retained failure:** leader exit alone did not prove
  process-group descendants exited. The new owned-group helper reserves the
  unreaped leader while observing/terminating remaining members. Darwin quick
  exit returned ESRCH from getpgid and EPERM for a terminal-only group, initially
  breaking ten decision fixtures and the bounded Git process test. The helper
  now captures creation provenance and performs complete read-only native group
  inspection; its three actual local process fixtures and bounded Git unit pass.
  Shared affected suites are being rerun. No security-policy change or sandbox
  bypass was made; escaped sessions and denied/incomplete observations remain
  explicit uncertainty.
- **Material native process-containment finding (open):** the new finite
  escape fixture demonstrates that a descendant can call setsid, close inherited
  streams and continue writing after the original group exits. Existing Seatbelt
  configuration has no verified escape denial. Native group/stream completion
  therefore no longer grants the stronger all-processes-exited/check capability;
  `process_tree_unverified` retains the Sent receipt, permit and owned reference.
  This is an incomplete implementation guarantee, not merely an unrun test.
  Built-in Codex shell is now disabled for every role; declared commands remain
  scoped host tools. A real process-containment/ownership mechanism and supervised
  native qualification remain required. No sandbox bypass or host change was made.
- **Descendant implementation research (2026-10-25):** local macOS 27.0.1 SDK
  and Apple XNU source rule out recursive kqueue NOTE_TRACK/NOTE_CHILD: unsupported
  since 10.5 and rejected with ENOTSUP. The installed SDK documents the macOS 27
  Endpoint Security descendants client, but it requires an Apple-granted signing
  entitlement. No entitlement application, signing change or client activation
  was attempted. This is a possible engineering route, not validated containment;
  exact event-loss, identity, cancellation and exit guarantees still need design
  and adversarial qualification. See [native host notes](docs/native-host.md).
- Runtime authority, actual worker-handle collection, retained admission settings,
  signed pending outcomes and durable attempt planning are being composed in
  `src/runtime.rs`, `src/runtime_schedule.rs` and `src/runtime_workers.rs`. The
  worker helper uses separate preparation and model admissions. Explicit
  disposable test-resource write grants and no-repository read-only child
  contexts are being integrated. Worker completion remains a pending claim.
  Full scheduling/phase progression/continuation/correction/recovery and the
  executable entrypoint remain incomplete. Abrupt-host orphan recovery and exact
  remote Git object acquisition into owned caches are newly identified gaps;
  configured source checkouts are never assumed magically up to date.
- Real-agent evaluations: not run. Real integrations: not run. Human calibration: not run. No provider mutations, Keychain changes or unknown remote outcomes exist for this build.
- Runtime decision composition now delegates actual human commands through
  `DecisionManager`, stores only sealed acceptance/correction/retry proofs, and
  requires a concrete release/recovery backend. Retry selection rejects ambiguous
  phases, incomplete admissions and unresolved effects. Restart recovery checks
  exact current signed native comments against publication receipts and complete
  prior admission digests. Its newest recovery regression is awaiting an affected
  rerun; the initial three selection regressions passed.
- Review found that correction/retry generations could bypass provider intervention
  if only the latest exact-phase receipt were checked. New account/work guards
  scan historical receipts, and the worker checkpoints authoritative session state
  before continuing. Runtime inserts intervention/input holds under the same gate
  as new admissions and recovers bound protected journals. Later successful exits,
  profile edits and new retry budgets cannot clear them. Six scheduling tests and
  nine then-current library tests passed; the latest recovery/cache additions need
  their affected rerun. No supported provider-resolution adapter exists yet.
- Additional review fixes: cached acceptance now compares the exact evidence token
  within a generation; edited/deleted acceptance/retry/correction publications are
  re-read before proof insertion; a shared pre-dispatch correction fence closes
  stale normal and active gates until intake or actual `not_sent` reconciliation.
  Agent-reported latest decision/reconciliation/cleanup run: 13/24/11 passes.
- Process journaling now preserves Starting references, actual spawned process
  identity and an inherited OS guard. Three deterministic journal fixtures pass.
  Group exit still does not prove full descendant exit; durable identity records
  never authorize signalling a PID reconstructed from disk. Worker injection and
  crash recovery integration remain in progress.
- Full Clippy and the latest root focused rerun initially stopped at shared schema
  migration compile errors (`HostedProofExpectation.context_digest`, then the
  parent evidence fixture's `ProductionBaseline.target_artifacts`). These are
  retained integration failures, not test passes. Target-specific deployed artifact
  provenance is being implemented; no single artifact is selected arbitrarily for
  multiple deployment targets. SPEC SHA and `git diff --check` still pass.
- The complete registered crate subsequently passed 411 offline tests with two
  explicit native ignores, plus strict all-target Clippy and the coverage inventory.
  Independent review then found a retry grant could survive a deleted publication
  until status inspection refreshed the grant cache. Planning, selection and the
  final serialized admission now revalidate the exact current native grant and
  reject superseded budgets; cached recovery errors also block retry selection.
  The affected 11 library/13 decision/6 schedule/28 worker tests passed again.
  Unregistered development drafts were moved out of `src`, because the build
  provenance hashes all source-directory files. The pending executable is at
  `/private/tmp/kegari-v0-main.rs.pending`; no coordinator has been activated.
- **Post-checkpoint integration:** the authorization publisher now rechecks the
  actual orchestration owner before admission and immediately before dispatch;
  its provider-wire regression passes all five scenarios, including owner loss
  at final dispatch with zero POSTs. The guarded process/source increment passes
  27 protocol, 6 process-journal, 28 worker and 1 provider-source tests, plus
  selected strict Clippy (agent-reported). Real Git candidate/execution fixtures
  pass 20 delivery and 18 execution tests; 13 decision and 11 housekeeping tests
  also pass after adopting the strict fixture executable identity.
- **Executable prerequisite:** strict pinning correctly rejects the installed
  Homebrew Git path because an ancestor is group-writable. Production policy was
  preserved. Disposable tests copy the exact installed Git bytes into a private
  fixture directory and verify their digest; this is not an installation or a
  qualification of the original path. Actual native boot identity remains denied;
  fixture probes identify their simulated boot UUID explicitly.
- **Runtime recovery review (correction in progress):** joined worker results now
  retain sealed check capabilities before receipt retirement. A failed retirement
  cannot discard the result or skip other completed workers. Empty process
  registries cannot upgrade a joined-but-uncertain process tree to confirmed exit.
  Pending correction proposals now retain a signed historical authority envelope;
  recovery checks actual admission, contract, completed turn and journal output.
  These new paths compile; focused adversarial tests remain in flight.

## Blockers and bounded follow-up dependencies

| Dependency / affected requirements | Observed evidence, permitted work, smallest next action |
| --- | --- |
| Live agent trials and account compatibility; §7, E01–08, §12.3.3 | Not attempted: no explicit accounts, finite live plan or execution authorization. Prepare cases and production runner locally; later approve named account/profile, isolated fixtures, repetition/turn/time bounds and handling of uncertain outcomes. No paid fallback. |
| Disposable Linear/GitHub integration; §§4/8/9/12 | Not attempted: remote repository writes and deployment remain prohibited, no approved disposable targets. Prepare exact account/target/action/evidence/recovery plan; explicit scoped authorization must amend applicable prohibitions before remote test actions. |
| Tailscale/media/background/locked-session/handover; §§7/8.5/10.3/11 | Required utilities not on PATH; no installation attempted. Implement adapters and deterministic fixtures; later identify installed supported paths or operator-provisioned tools/certificates and approve bounded host testing. |
| Actual macOS execution boundary; §7.1, V0-T12, E07 | Installed Codex schema generation and credential-free strict named-permission parsing succeeded. A nested sandbox probe exited 71 (`sandbox_apply: Operation not permitted`) under the existing outer sandbox. No bypass/unsandboxed retry. Continue deterministic boundary tests; actual canaries need a permitted supervised execution environment before qualification. |
| Native execution descendants; §§7.1/11, V0-T12/23/E07 | An ordinary finite owned fixture proves setsid descendants can outlive group/EOF completion. Native result promotion now fails closed with `process_tree_unverified`; implement verifiable containment/descendant ownership before native checks can pass, then qualify in an allowed supervised environment. This is an engineering gap, not permission to use group exit as full proof. |
| Native process ownership; §§6.1/11, V0-T07–09/23 | Read-only macOS boot-identity query returns `Operation not permitted` in this sandbox. Production ownership discovery fails closed; two retained native tests are explicitly blocked. Injected deterministic process fixtures verify registration/intent/lease logic only. A permitted supervised environment is required for actual process birth/boot identity, termination and restart qualification. |
| Trusted native Git executable; §§7/8/9 | The installed Homebrew Git has a group-writable ancestor and fails the unchanged executable-identity policy. Exact-byte private copies are used only inside disposable tests. Production setup needs an operator-provisioned supported executable path satisfying the owner/mode/ancestor checks; no host permissions or installation have been changed. |
| GitHub-hosted runner provenance; §§8.2/9.4 | Job labels do not prove runner type. Signed OIDC verifier now binds fixed issuer/JWKS, audience, exact job/run/attempt/workflow and time/replay guards. The trigger SHA is distinct from the source actually checked out: the reviewed pinned proof workflow must verify the tested checkout and derive the audience accordingly. Adapter fixtures pass; proof workflow preparation/collection and actual hosted route remain unqualified. |
| Human qualitative review/calibration; §12.4.3 | Operator explicitly unavailable; not attempted. Prepare representative evidence across every qualitative rubric and all material failures, then request review only when supervision is explicit. |

These block only dependent validation, not local implementation. No missing prerequisite is permission to weaken the contract.

## Milestones and recovery notes

- Frozen continuation/successor/evaluation checkpoint: 613 passing tests, zero
  failures, two native ignores, strict lint/fmt/inventory/diff checks passed.
  Actual test and Clippy sessions exited zero; all 534 before/after input hashes
  match. The read-only evaluator reports 56 slots with dispatch disabled.
  Independent review corrections and remaining gaps are recorded in
  `validation/LOCAL-CONTINUATION-SUCCESSOR.md`. No live activity occurred.
- Local checkpoint `90ce6d4631dc605a43ba1aa4033c839832b9cf95` —
  `feat(kegari-v0): connect corrections and delivery recovery` records native
  executable composition, active correction transitions, bounded remote object
  acquisition, explicit release recovery and no-release completion. Frozen
  verification: 577 passed, two native ignored; strict lint/fmt/inventory passed.
  Signed off by the configured human committer with `Assisted-by: Codex`.
  No commits were consolidated or pushed; the worktree and source hash were
  verified immediately after committing.

- Local checkpoint `6b5a304e5898ab4ae87969ae41085c90b06e73d2` —
  `feat(kegari-v0): add guarded delivery progression` records guarded process and
  cleanup ownership, exact child/parent/release adapters, correction publication,
  runtime progression, instruction asset version 2, and evaluation archive tooling.
  Frozen verification: 523 passed, 2 native ignored, strict lint/fmt/inventory
  passed. Signed off by the configured human committer with `Assisted-by: Codex`.
  No commits were consolidated or rewritten; clean tree was verified immediately
  after commit. Later local work above is intentionally uncommitted while in flight.
- Initial inspection complete; clean scaffold contains only `fn main() {}`. No existing runtime to preserve or activate.
- Initial dependencies fetched successfully into `/private/tmp/kegari-v0-cargo-cache`; exact versions are locked. Generated Codex schema probe data is under `target/codex-schema`; selected fixtures are in `protocol/`. No model turn was launched.
- One formatter raced creation of the toolchain pin and attempted rustup sync; the sandbox rejected its temporary-file write before installation. Repeated with the existing verified stable alias successfully. No host config changed.
- Local checkpoint `24c7d4e` — `feat(kegari-v0): add durable control and instruction foundations` records strict config/workflow/assets, authority/correction/scheduling, private storage/control and snapshot primitives plus focused tests and the initial coverage ledger. No commits were consolidated or rewritten. Coordinator alone stages/commits; all source remains task-owned and initial index was empty.
- Local checkpoint `06c8786` — `feat(kegari-v0): add the pinned Codex execution adapter` records the installed-schema profile, bounded stdio adapter, explicit filesystem/tool settings, deterministic fixtures and source attribution. Generated schemas retain applicable Codex Apache license/notice; the installed alpha's exact public source commit is not established. No binary is redistributed. No commits were consolidated or rewritten.
- Local checkpoint `55712e1a6ba1de8e17d9cbf95ee166c7347a5d58` — `feat(kegari-v0): enforce isolated delivery and evidence gates` follows 18 passing focused tests, including the atomic lock-reconciliation regression. No commits were consolidated or rewritten. Later replacement/production accounting changes remain uncommitted pending review correction.
- Local checkpoint `20a932b6727c93dd5af77964efa22db66583b1ca` — `feat(kegari-v0): add scoped provider adapters and hosted proof verification` records 18 passing offline wire/proof/receipt tests and strict Clippy. Provider HTTP implementations and fixed-issuer signed proof verification are committed; executable integration and real provider compatibility remain incomplete. Separate checkpoints preserve the independently reviewable adapter, harness and delivery contracts; no history rewrite or consolidation was performed.
- Local checkpoint `92b5cd42b9acf88884006086cfb11f2d87b1f032` — `feat(kegari-v0): add version-bound agent evaluation fixtures and runner` records 28 variants/56 bounded trial slots, 20 passing deterministic runner/grader tests, full tool/mode provenance review corrections and build-bound instruction manifests. No actual-agent trials or human calibration ran. No history rewrite or consolidation occurred.
- Local checkpoint `ac79bdb7002143fb4324a895de7b4a29cc2096c8` —
  `feat(kegari-v0): track replacements and bind delivery provenance` follows 28
  passing Git/evidence fixtures and selected formatting/diff checks. The initial
  same-change commit `4a689e284c68662ae7754e3f0af2363418b79d0f` was immediately
  amended only to wrap its body at 80 characters. Recovery reference
  `refs/recovery/kegari-v0/4a689e2` was created first; its file tree and final HEAD
  were verified identical. No other commits were consolidated or rewritten.
- Local checkpoint `e9d16ba16b58ec58227178e80630e12e6bd258b5` —
  `feat(kegari-v0): compose scoped control and worker services` records the real
  provider/control/CLI/worker components, 411 passing offline tests and two
  explicitly ignored native qualifications, strict all-target Clippy and the
  subsequent affected retry-publication review correction. No commits were
  consolidated or rewritten. This remains an incomplete application checkpoint.
- Subsequent source/process/deployment/runtime changes remain in flight. All test
  processes are bounded local Cargo tests; no lasting service has been installed
  or started. Git fixtures are owned temporary repositories. No project branch,
  worktree, push, deployment or coordinator activation occurred.

Local checkpoint boundary: this increment remains separate from 946a15f because
it introduces new historical dependency and inspection consumers, whereas the
previous checkpoint also contains independent worker/driver ownership changes.
Keeping the validated consumer increment separate makes both changes reviewable;
no published history is rewritten and no remote action is authorized.


### Held paired recovery provider bridge

Root prepared the one-file metadata provider bridge at
`/private/tmp/kegari-paired-recovery-provider-draft`, manifest
c03351f2e5424784cc4c9462f0fd428d4b5b386b495338cbc834a1d90fbb753a.
It adds a mandatory-callback paired entry point over the unchanged original
metadata protections and binds exact round/recovery provenance into paired scope.
It depends on Config's typed Control layer and a real native candidate/caller;
syntax-only, not installed/compiled. It does not create model/source/ownership
capabilities or solve repeated resumed failures. Root owns future native integration.
