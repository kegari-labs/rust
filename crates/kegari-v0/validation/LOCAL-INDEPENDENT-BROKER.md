# Local same-owner broker diagnostic evidence

Summary: Exact broker source1 records verified; historical transport failures remain preserved. Local component/integration evidence only, not native qualification. Final deterministic suite: 954 passed,
0 failed, 2 ignored across 39 groups in 785.104s.
Tested source: `sha256:897b56c30c0a32c2ce82724c500e1897a44b09dfe7cbc85486d3b053ac5ea6a5`. These totals come only from this broker full run;
focused and historical counts are never added.

This increment retains an explicitly declared IndependentReview worker's actual
broker-call UUID membership, original timestamped journal anchors, exact returned
context/source value digests and provider-normalized feedback request identity.
A single original Control gate reads known same-owner receipts at collection;
it does not discover arbitrary effects, grant authority or turn an ACK into
remote-content readback. Ordinary workers remain opted out. Report diagnostics
contain bounded references/digests, with raw permitted diagnostics in protected
storage. Protocol and optional broker records share one nonrenewing 8 MiB
allowance; this does not bound every mandatory worker journal write.

Seven new component functions cover: exact same-write anchors against first-read
mutation/removal/restoration; shared-budget refusal before a write; actual local
ProviderBroker/Control normalization and Pending/ACK-only/NotSent classification;
unknown outcome after an owned task exit retaining the original hold; exact
same-descriptor source bytes plus traversal and escaped-JSON oversize refusal;
unavailable diagnostic archive/legacy preservation; and four internally
well-shaped context/tool/source/permission contradictions against the report.
The archive fixture remains explicitly synthetic and Blocked, not a capability.
The three extended tests retain ordinary opt-out and scorer-load/grade-error
evidence. Four existing controls and all sixteen transport/receipt regressions,
plus both prior failed tests, must pass in exact full targets.

Static review B1 found successful-read observation ran before the public encoded
Value bound; v2 moves that unchanged bound before the callback. Its actual
20,000-NUL source case must reject without a success fact while preserving the
same-descriptor mutation case. B2 found internally consistent broker identities
were insufficiently compared with the actual report manifest; v2 adds exact
context/tools and optional authority source/permission comparisons. The budget
test name now describes monotone reservation and pre-write refusal, not an
unperformed uncertain-write fault. These were static findings, not fabricated
executed test failures; both immutable reviews and corrections remain retained.

## Preserved prior evidence

The prior committed transport report records 947 passed, 0 failed and 2 native
ignores across 39 groups in 800.907s at source e908c28aada5577ce254bcf2a78e2eb5e5c5c9b8e1950b4d01bccd20ce7e9c65.
It preserves source1's strict-Clippy failure and source2's actual full result:
943 passed, 2 failed, 2 ignored, 39 groups, 782.758s, exit 101. The failures were
`runtime_delivery::child::continuation_tests::exact_validation_retry_uses_native_grant_and_new_checks_without_join_fiction`
and `runtime_delivery::release::successor_fixture::chained_runtime_retains_merged_contribution_through_deployment_only_predecessor`.
Neither exact original interleaving was observed; receipt serialization and the
bounded synthetic publication interval address concrete reachable races without
claiming a proven retrospective cause. Original raw logs, corrections and reviews
remain in the prior immutable evidence, which is pinned and rechecked here.
These counts are history, not additions to this increment's totals.

## Evidence scope and remaining gaps

The recorded set has thirteen broker invocations: eleven serial validation steps
and final inventory/diff checks, each bound to the same 694-input source and
exact toolchain. All affected compatibility targets run whole; the full raw log
supplies target identities, cases and totals. The exact two native platform
ignores remain explicit. Disabled Plan preserves 28 variants, 56 declared slots,
unchanged profiles and dispatch_authorized=false; it launches no evaluation.

Two existing partial report/provenance clauses receive only narrow component
appends. No row/status is promoted; prior evidence and all other rows remain
unchanged. No universal read/effect/output completeness, canary non-disclosure,
claim acceptance, E07 attack behavior or scorer/native qualification is claimed.
No Complete state is added. Missing or legacy data is unknown, never zero activity.
A collector-enabled cancellation barrier, actual effect-return/finish-write-fault
registered capture, independently read pending-claim content and genuine native
worker positive remain unexercised. Scorers, fixtures, instructions and permission
or qualification predicates are unchanged.

Recorded commands and immutable raw logs:

| Recorded run | Exit | Seconds | Command | Log SHA-256 |
| --- | ---: | ---: | --- | --- |
| check-1 | 0 | 23.988 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo check --locked -p kegari-v0 --all-targets` | `1479b719c6ad2c5894a3e4bc4dc1d5176d001f50e37979a390bd66c36cfb8fc7` |
| clippy-1 | 0 | 24.922 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | `cd083d1e0f01820763311aaa5da14ca9c203b905fc9f7befe98c5475ea780636` |
| broker-1 | 0 | 41.504 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib worker::broker_observation::tests::` | `6d6c6f38634a4e90f256dc154fe0e90a78f87f828a1f670f8861483e9cffcccc` |
| control-1 | 0 | 12.196 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib control::` | `de4de09d2cc0be08680b6305db46005ff561e6ac17ebdadd6810e2f99d59f491` |
| runtime-1 | 0 | 1.759 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib runtime::` | `9b4f38ce1dd9bd28e3679085eae028e2199eaa20b199d988a597a66e406b1b29` |
| evaluation-1 | 0 | 1.951 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib evaluation::` | `9c0357439da52058ead5518e9fa36c42c06e0f221efe01c588db3416847718f4` |
| protocol-1 | 0 | 0.165 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --lib codex::observation::` | `95e0a35f5e02db00b7afbf8d22ffd54da4524e1f9843eb4dddd6af098512c2ef` |
| compatibility-1 | 0 | 93.884 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --no-fail-fast --test control_preparation --test control_storage --test storage_lock --test providers --test worker --test evaluation --test evaluation_reports --test runtime_schedule --test authority_scheduler --test codex_protocol` | `c6a6264dd0839aab4a0afe3a9e4ad45ca2aa97b726cf04e2b57bffdb92767e53` |
| fmt-1 | 0 | 1.529 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo fmt --all --check` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| full-1 | 0 | 785.104 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0 --no-fail-fast` | `b7dc6fddbefd8ee736c067fa23064d4b7da1013c54ab3be6c0a0cb8a82ffcf74` |
| plan-1 | 0 | 32.297 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | `cd37e234e922354b41eb8d4dd8b2572312bf3771db78c1a8abbbb6316b65a063` |
| inventory-1 | 0 | 0.045 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache python3 crates/kegari-v0/tools/contract_inventory.py --check` | `de843922ff0517de68c07be40f91edfad2f63da24be0a5586d6f429bd7d943fc` |
| diff-1 | 0 | 0.023 | `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache git diff --check` | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
