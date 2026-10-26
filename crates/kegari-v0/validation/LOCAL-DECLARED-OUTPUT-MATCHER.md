# Local declared-output matcher evidence

The final source passed 1001 tests, with zero failures and the same two native ignores across 39 full-suite targets. The full command took 756.594 seconds. This is local component evidence for declared output capture and positive byte matching; output remains `Partial` / `CoverageNotQualified` and does not establish a negative disclosure result or E07 qualification.

The executed collector verified 14 final-source records and three preserved source1 records against their actual commands, logs, before/after source snapshots and compiler identities. Tested source: `sha256:c4a8b71e52ef8b4c93523766e8293b6d7a1883e0804fa3700cb4a8458bcd32af` (706 inputs). The machine report contains exact argv, working directory, stable toolchain/private Cargo cache settings, timings, hashes, full target/test membership and the unchanged source map. Focused counts are not added to the full-suite total.

| Record | Exit | Seconds |
| --- | ---: | ---: |
| check-1 | 0 | 33.139 |
| clippy-1 | 0 | 31.987 |
| evaluation-1 | 101 | 59.066 |
| clippy-2 | 0 | 18.724 |
| evaluation-2 | 0 | 18.782 |
| protocol-2 | 0 | 0.161 |
| storage-2 | 0 | 0.149 |
| runtime-2 | 0 | 1.903 |
| worker-2 | 0 | 1.914 |
| owned-return-2 | 0 | 0.255 |
| worker-return-2 | 0 | 0.172 |
| compatibility-2 | 0 | 75.508 |
| fmt-2 | 0 | 1.638 |
| full-2 | 0 | 756.594 |
| plan-2 | 0 | 41.609 |
| inventory-2 | 0 | 0.041 |
| diff-2 | 0 | 0.018 |

The final suite preserves every preceding E07 test and adds exactly the 17 named tests in the authored inventory. Each new test passed once in its required evaluation or protocol filter and once in the full suite. The seven selected integration targets passed 109 tests without filtering. The source1 all-target check passed; final-source all-target strict Clippy compiled the corrected source and passed. There was no redundant source2 `cargo check` command.

Source1 evaluation failed 82/1 at `original_deadline_and_shared_budget_refuse_without_output_reset`: initialization had already charged 128 bytes, so whole 64 KiB reservations left 65,408 bytes. The test-only correction exhausts that actual remainder and asserts refusal before the event and after finalization. The original log, failed membership, exact before/after file and reviewed patch remain pinned. No production budget, deadline or assertion was weakened. Source1 check and strict Clippy passes remain historical, and no source1 full-suite run is claimed.

The first collector execution failed after application checks had completed because it compared the successful CLI envelope (`{data,error,ok}`) directly with a Plan body. That collection failure is separate from application tests: exit 1 in 0.164 seconds, original source/toolchain unchanged, raw log SHA `df11230ce8163753ec70515d572431a6ec2240d5d23f099187531806fcc9908d`. It reached no output-directory creation. Collector v2 validates exact `ok=true`, `error=null` and object `data`, compares the entire envelope to the prior verified raw Plan, then compares `data` with prior machine evidence. The original collector, failure record/log and narrow correction are retained; no application rerun or source change was needed for this correction.

Static review also preserved MA-1 (unbound/established callback labels split the same explicit thread/turn/item stream), fixed by exact nonempty tuple identity with isolated invalid IDs. MR1/MR2 corrected canonical temporary roots and the retained failed-member/error-record oracle before installation. These are review findings, not run results. Exact independent reviews and installed-source provenance are referenced in the machine report.

The tests exercise the four declared marker forms, UTF-8 key/value and root-array/string projection, cross-chunk and per-stream concatenation, original protected-file creation/readback, refusal on substituted own anchors, shared capture budget and original deadline, safe public labels, archive/grader-error retention, policy parsing and actual duplex callback ordering. Synthetic protocol/session data and plain reducers are explicitly components; they do not instantiate the production live enrollment. A matching original protected piece can preserve a positive despite unrelated gaps, while its own replaced anchor still refuses authentication.

The disabled Plan is exactly equal to the preceding Plan: 28 variants, 56 slots, `dispatch_authorized=false`; it took 41.609 seconds. All 349 evaluation assets are unchanged, including the source recipe, v3 scorers, fixture/context data and profile bindings. The preceding actual 20 Python source-data tests are referenced with their raw-log hash and unchanged input identity; they were not rerun for this increment.

Production source review covers actual protocol/feedback hook wiring and private owner enrollment. This local validation does not prove actual registered native matching, native containment, joined termination, complete original-reader or broker membership, remote publication truth, output-wide secrecy of unchanged legacy labels, a negative canary witness, scorer qualification, human acceptance or a completed E07 exercise. The two native tests retain their exact prior ignored names and reasons. No model, provider, account, source-preparation or native trial was launched by the collector.

The worker focus retained uncertainty evidence at `/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpzJW7y2`. This report claims no cleanup. The collector writes only a new verified TEMP bundle; coverage remains a separate narrowly scoped append-only review and this report does not mark requirements complete.
