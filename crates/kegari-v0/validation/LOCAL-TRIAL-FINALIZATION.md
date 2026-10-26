# Evaluation terminal-finalization evidence

The change passed **100 focused tests across six groups**, all-target compilation,
strict Clippy, formatting, the contract inventory and the dispatch-disabled
evaluation Plan. No full crate suite was run for this increment. The preceding
paired-recovery checkpoint's 798-test result retains its own source identity.

All 629 build inputs remained unchanged at
`sha256:95f47600a9a67c9b8f7399eb526a4949caa3f4ed08081b788478de3aa9ce554a`.
[Machine evidence](trial-finalization-checks.json) contains all ten invocation
results, exact source hashes, commands, log hashes and before/after observed
Rust/Cargo 1.99.1 identities. The approved SPEC 1.2 and inventory inputs stayed
unchanged. All commands ran in the Rust workspace with `RUSTUP_TOOLCHAIN=stable`
and `CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`.

| Command | Result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass, 18.135s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass, 18.987s |
| `cargo test --locked -p kegari-v0 --test evaluation --test evaluation_reports --test worker -- --nocapture` | 78 passed |
| `cargo test --locked -p kegari-v0 --lib evaluation:: -- --nocapture` | 19 passed |
| `cargo test --locked -p kegari-v0 --lib worker_run:: -- --nocapture` | 2 passed |
| `cargo test --locked -p kegari-v0 --lib worker::trial_checkpoint_tests:: -- --nocapture` | 1 passed |
| `cargo fmt --check -p kegari-v0` | Pass |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 planned trials; dispatch unauthorized |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | 496 clauses; 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

A failed, interrupted or prelaunch-blocked trial can now finalize through its
original admitted lifecycle after actual owned exit. A private single-use
execution observation binds the whole broker checkpoint and pending request,
actual hook result and execution outcome independently of the rubric. Completed
execution still requires a current completed checkpoint. Runtime installation
retains the exact registered contract Arc and operation, and Control retirement
still refuses unresolved effects. Reports cannot create completion authority.

Timeouts retain the actual callback future during bounded cancellation drain.
The configured after lifecycle is attempted once after a started before lifecycle,
including failures; its real result is journaled before final capability creation.
A callback that does not drain, a missing hook result, unknown process ownership
or a mismatched checkpoint prevents finalization. Pending requests and unknown
effects remain recorded. No report or grade can promote failed execution to
completed execution, or erase a hook failure.

Independent static review by a second agent and root inspection found no further
concrete defect in the four installed files. Their exact bases and draft hashes
were verified before installation. The new tests cover execution classification,
whole checkpoint and pending-method substitution, and actual Control admission
with explicitly synthetic failing hooks. The hook test confirms one after call,
wrong-operation refusal, retained failure and no terminal capability or retirement.
These tests do not manufacture provider, native process or joined-run authority.

The concrete one-trial caller remains separate unfinished TEMP work. Actual
failed-provider, timeout with owned descendants, same-trial completion and
runtime retirement positives remain unrun. Two required native probes, repeated
actual-agent evaluations and human calibration remain incomplete. No live trial,
remote mutation, denied probe, installation or deployment was performed. This
checkpoint does not qualify the application for operation; the Goal is incomplete.
