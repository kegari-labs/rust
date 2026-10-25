# Local owned-target no-release history validation

Status: current deterministic and component checks passed **721 tests,
zero failures and 2 native ignores across 39 groups** in 727.621s.
Current source10 is `sha256:b65dcbbcc2a32882362d2a4c73a50841e8fd811a96043ef6bd4e273c69a16214` (593 inputs).
The scoped-history isolation finding and subsequent serialization/shared-status
findings described below are corrected and exercised by full10. Native, live-agent,
real-provider and human qualification remain incomplete; the Goal is not complete.

## Scope and provenance

This increment extends immutable no-release history to ordinary production
baselines observed through actual owned target deployment proofs. It retains the
original proof, challenge, immutable locators and closed admission/dispatch
receipt ancestry. Historical recovery rereads those exact local references;
later main or target movement and mutable target indexes do not replace history.
The seal retains target resources without taking ownership, clearing holds or
consuming release spacing. Optional new fields omit empty values so existing
signed deployment-free schema-1 records retain their bytes and digest.

The first fixture uses actual owned disposable Git, Control admission/retirement,
offline native/OIDC transport, target proof, ordinary baseline, isolated unchanged
candidate vector and signed publication/seal paths. Initial deployment authority,
parent integrated-check facts and human acceptance intent remain explicit
synthetic upstream inputs. No native confined execution is fabricated. Its new
delivery facade shares RuntimeCoordinator/ControlHost; it is not a process restart.
Resumed ancestry and differing multiple-target artifact coverage passed their
focused checks and the complete suite. Resumed assertions consume the existing actual twice-resumed
terminal deployment fixture and test the ordinary baseline reference collector;
they do not claim a second no-release publication path.
External production observations without owned proof remain a separate producer.

## Validation record

Root exclusively owns serial Cargo. The local runner records commands, installed
Rust/Cargo 1.99.1 identity observations, before/after source snapshots and hashed
logs under `/private/tmp/kegari-v0-owned-target-history-<label>`. Failed results
remain retained; labels are never reused.

| Label | Check | Result |
| --- | --- | --- |
| check-1 | `cargo check --locked -p kegari-v0 --all-targets` | Passed, 19.152s, at 586 inputs, `sha256:7ba2685a510544a279d23735ce94c021d8e1eb39ef18f14709158f7a1ebd010b` |
| no-release-1 | Complete focused no-release suite | Passed: 11/11, 96.872s wall, unchanged 586 inputs; includes owned-target and legacy serialization regressions |

| deployment-1 | Deployment integration suite | Passed: 19/19, 31.998s wall, unchanged 586 inputs |

| check-2 | All-target check after resumed/multiple-target coverage | Passed, 11.453s, at 589 inputs, `sha256:27767cc39c1c2548c6acd74fa4620dd22bc1f5148ba66caa04539ea3e902316d` |

| no-release-2 | Focused suite with distinct target artifacts | Passed: 12/12, 80.405s wall, unchanged 589 inputs |

| chain-2 | Chained terminal recovery with ordinary baseline ancestry checks | Passed: 1/1, 210.158s wall, unchanged 589 inputs |

| clippy-2 | Strict all-target Clippy | Passed, 19.801s, unchanged 589 inputs |

| fmt-2 | Formatting check | Passed, 1.206s |
| inventory-2 | Contract inventory with auxiliary hashes | Passed: 363 blocks/136 named requirements, 0.043s |
| diff-2 | Diff whitespace check | Passed, 0.015s |
| plan-2 | Dispatch-disabled evaluation plan | Passed: 56 slots, dispatch_authorized=false, 25.159s |

| full-2 | Complete unfiltered crate suite | Passed: 719/0/2 across 39 groups, 707.458s wall, unchanged 589 inputs |

Independent review of the producer, original fixture and additional resumed/multiple-target coverage found no remaining material
authority issue within the bounded scope. A review tightened the later-native
baseline refusal to its exact error rather than accepting any error. All-target compilation and focused no-release tests passed. Deployment integration also passed. Additional resumed/multiple-target coverage
and complete-suite validation passed at the recorded 589-input snapshot. Native containment,
actual-agent and real-provider qualification and required human calibration are
not established by these local checks.

## Review finding after source2 (corrected in source10)

Read-only review after the focused passes confirmed a scoped-history isolation
bug: `NoReleaseQueueHistory::require_available` and queue-wide completion
validation reject unrelated release resources when an authenticated known seal
loses one required reference. SPEC §9.6 requires unrelated disjoint work to
continue. This is separate from unsafe/unauthenticated enumeration, which must
remain globally unavailable. Scoped authenticated quarantine is in TEMP design;
no correction or regression result is claimed yet. Full suite 2 passed at unchanged inputs
and cannot verify this currently missing behavior. Failed completion credit and
its affected resource protection must remain denied during the correction.

## Scoped-history correction, intermediate source window 3

Integrated 21 reviewed files with exact base/draft hash verification. All-target
check 3 passed in 18.321s at 592 inputs,
`sha256:64def15d6b2cbc4baf06b2cbd52ae1a003d713b77d0cdf28e96aca6012c104f8`.
Focused no-release 3 failed (10 passed/3 failed, 96.916s): changed underlying
missing-seal message, pending-bootstrap fixture comparison, and extra read-admission
receipts for an already blocked parent. Root overlay 4 corrects these while keeping
exact diagnostics and receipt assertions. Ordinary release 3 failed (0/1, 98.707s) at a remaining old global-restore assertion; exact affected refusal replaces that expectation. Known schema replacement remains denied, while legacy compatibility is tested with the actual cold reader explicitly given no prior anchor. Two new independently confirmed gaps remain:
quarantines need retained outer-envelope readers for pre-sync tamper detection,
and completion commits need their known-anchor check inside Control serialization.
Separate overlays and actual fixture regressions are in progress; no validation
claim covers those uninstalled corrections.

Source window 4 integrated nine reviewed overlay files after all base/draft hashes
matched, at 592 inputs:
`sha256:b648c2671a06d5e26beef2ca5fba1c5087d9cd6b5a4d5f88e5e29a7b81eef710`.
Check 4 failed in 10.871s at the new helper’s missing IssueKey import; the type is now fully qualified. Check5 passed in 17.163s at unchanged 592 inputs, `sha256:b23893dd1bc099f0a4478386e0daccb456befdb157efa9f59c97835d3facb694`. Independent rereview confirms the retained-reader and atomic
identity fixes structurally; actual regression execution remains pending.

Root serial focused validation now runs no-release5, ordinary5 and chain5, stopping on the first failure. No passing result is claimed in advance.

Focused no-release5 failed (12/1, 94.2s) on actual symlink classification:
`io_error` instead of `unsafe_storage`. Ordinary5/chain5 did not start. Source6
corrects the protected no-follow file/directory error classification and adds
actual symlink assertions to the existing storage regression. At 592 inputs,
`sha256:c777e75a6bdb151bdb3f15f0ae644ed5ef86485d7a820033bc1693589b8adc87`,
root now serially runs storage6, no-release6, ordinary6 and chain6; it stops on
any failure. All earlier failures remain retained.

Storage6 passed 1/1 in 26.780s and no-release6 passed 13/13 in 74.796s at unchanged source6. The serial runner proceeds to ordinary6 and chain6; neither result is assumed.

Ordinary6 passed 1/1 in 185.286s at unchanged source6; chain6 remains running. A new review-confirmed inventory-versus-commit anchor registration race remains: inventory writers must share the final Control boundary with commits. Separate TEMP correction/caller review is in progress; source6 passing tests do not validate that pending correction.

Chain6 failed 0/1 in 179.458s on its old release_hold-only expectation after injecting a malformed completion. The TEMP correction asserts actual record_corrupt, its persistence after deletion, and a successful complete scan before hold recovery. No Cargo is active. Remaining serialization fixes include immediate authenticated-anchor learning, shared fatal status and a single Control gate for no-release inventory/cache/queue installation.

## Serialized history source window 7

Integrated eleven reviewed source6 files after exact base/draft hash checks.
Current592 inputs are `sha256:becb7553c5303f1c680ee16876b9e8b0056694f9d74b389472e6cc828019845f`.
First authenticated observations and all map/queue installation now share Control
with commits; prefix identities survive later inventory failure. Shared fatal
status and late origin-aware storage errors close fresh facades. Protective shared
NR scopes plus final deny-only checks cover new facades and the same parent-loop
window without granting anchor-only credit. Bounded independent review found no
additional concrete lock/authority issue.

Check7 passed in13.426s at unchanged inputs/toolchain. Root serial focused no-release7,
ordinary7 and chain7 are running, stopping on failure. The actual first-hydrate/lost
seal regression now also denies cached current fallback before sync. A distinct
full interleaving fixture remains TEMP work; no concurrent race test result is
claimed. Prior failures remain retained above and in their invocation artifacts.

Source7 no-release7 failed13/1 in89.923s on the obsolete assertion that immediate
producer anchor registration leaves the shared map empty. Source8 (592inputs,
`sha256:89952415ddaa3b788241f133e1e4e00539e9cf44ec6b9891a065ec10d675e3e2`)
corrected that assertion and kept the exact direct known-seal replacement message.
No-release8 failed13/1 in74.221s on the old successful-restore expectation after
a global NR failure. Inspection then found restore also copied that NR failure
into the independent ordinary inventory. Both later ordinary/chain invocations
did not start.

Source9 (593inputs, `sha256:c18612b627226ad0a2fdc93d573a71929488357b5411bc12c2b4e76733fe3a92`)
uses a structurally tagged restore-error origin, preserves existing independent
ordinary failure state, and adds exact category/recovery assertions. It also
integrates a genuine cold-cache inventory/commit interleaving fixture with bounded
per-facade hooks, no raw map clearing and no proof factory. Five files integrated
with base/draft checks and a clean nonoverlapping release.rs merge. Root now runs
check9/no-release9/ordinary9/chain9 serially, stopping on failure. No result is
assumed; all prior failures and snapshots remain retained.

Check9 passed16.926s; no-release9 passed14/14 in92.698s at unchanged source9/toolchain. Ordinary9 now runs the actual first-reader race in its genuine completed-release fixture; chain9 has not started.

Ordinary9 passed1/1 in185.531s at unchanged source9. Its actual first-reader/commit interleaving, exact removed-seal refusal, unchanged provider/Control inventories and restored original recovery all executed successfully. Chain9 is running.

Chain9 passed1/1 in208.274s at unchanged source9. All focused source9 checks passed; session89118 closed. Root session39790 now runs strictClippy9/fmt9/inventory9/diff9/dispatch-disabledplan9/full9 and recollection, stopping on failure. No full-suite result is assumed.

Clippy9 failed13.081s on map-entry style and item placement after the test module.
No later strict check/full9 ran. Source10 changes only those lint issues, with
593 inputs `sha256:b65dcbbcc2a32882362d2a4c73a50841e8fd811a96043ef6bd4e273c69a16214`.
Root session52897 runs strictClippy10/fmt10/inventory10/diff10/dispatch-disabled
plan10/full10 and recollection serially, stopping on failure. All focused source9
checks remain prior exact-source evidence; the full source10 invocation will
exercise their scenarios again, so no additional focused repetition is needed
for the lint-only edits.

Source10 strictClippy10 passed21.359s; fmt10 passed1.206s; inventory10 passed363blocks/136namedrequirements in0.041s with auxiliaryhashes; diff10 passed0.020s; plan10 passed26.081s,56slots/dispatch_authorized=false. Full10 is running in session52897. No live evaluations ran.

## Final source10 result and bounded evidence

Full10 passed 721/0/2 across 39 complete unfiltered groups in
727.621s, at unchanged 593 inputs/toolchain. StrictClippy10, fmt10,
inventory10 (363 blocks/136 named requirements), diff10 and plan10 passed at the
same input digest. Plan10 has56 slots with dispatch_authorized=false. The machine
report verifies 36 invocation artifacts, including earlier failures,
and rejects changed logs, snapshots, compiler identities or missing final checks.
Session52897 and evidence collection completed; no result is inferred for any
unrun or unclosed earlier invocation.

The corrected behavior preserves the original authenticated completion and its
full scope, permits unrelated valid history, and denies failed exact credit and
affected resources. Unsafe/untrusted outer evidence remains globally unavailable.
First reads/anchor learning, writes and full installation share Control. Fresh
facades inherit protective anchors and failure state without gaining success;
separate ordinary and no-release failure categories cannot clear or contaminate
each other. Independent bounded producer/consumer/lock review and the actual
first-reader/commit interleaving found no remaining concrete issue in this scope.

The first-reader race reconstructs genuine retained records in a fresh runtime
cache sharing its registered ControlHost, pauses the actual inventory path before
registration, and starts a genuine delayed completion commit. It checks serialized
exclusion, original identity retention, missing-seal refusal, unchanged provider/
Control effects and recovery of original bytes. It is not full process-restart,
native containment or live-provider evidence. Existing explicit synthetic upstream
check/acceptance inputs remain labeled; two required native tests remain ignored.
