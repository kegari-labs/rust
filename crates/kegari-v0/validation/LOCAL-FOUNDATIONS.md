# Local foundation validation

This report concerns an implementation increment, not Kegari v0 readiness. The executable entry point, provider-backed orchestration, complete CLI/control transports, platform supervision and evaluation runner are still being implemented. No production runtime has been installed or activated.

## Build environment

The repository pins Rust 1.99.1 and exact direct dependencies, with transitive versions in Cargo.lock. The inspected host's existing `stable` alias resolves to `rustc 1.99.1 (58cf8f9c3 2026-10-13)`. Development checks use that alias explicitly to avoid installing or changing a toolchain on the operator's host:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --locked -p kegari-v0
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --locked --workspace --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=stable cargo fmt --all -- --check
/usr/bin/python3 crates/kegari-v0/tools/contract_inventory.py --check
```

The explicit stable override is valid only after checking that it is the pinned version. Other operators should use the pinned toolchain through their normal approved setup. The temporary Cargo cache is owned build data, not runtime state. No host installation was attempted.

## Evidence classes and limits

- Deterministic tests use owned local directories, process/protocol fixtures and disposable Git repositories. They establish the particular assertions present in each test; a test mentioning a V0-T number does not establish every clause of that row.
- Generated app-server schemas are evidence about the installed `codex-cli 0.164.0-alpha.11.1` protocol. They do not establish authenticated model, filesystem or provider behavior.
- Actual-agent evaluations, Linear/GitHub integration, locked-session operation, Tailscale/TLS, video upload/playback and human calibration have **not run**. There is no readiness or successful remote-operation claim.

## Independent findings and corrections

| Finding | Correction / evidence | State |
| --- | --- | --- |
| A failed pre-send mutation gate left a Prepared receipt; replay could appear accepted | Persist a terminal `not_sent` rejection and return the same error on replay, including after restart; `rejected_mutation_replay_retains_the_error_across_restart` | Regression passed |
| Recovery forgot prior active/uncertain ownership; a new ID could repeat work | Reconstruct target holds from unresolved receipts, enforce in admission and broker; `recovered_unfinished_work_cannot_be_admitted_under_new_owner`, `mutation_is_serialized_with_pause_and_unknown_replay_sends_nothing` | Regressions passed; platform process ownership still to implement |
| Reclassification to ambiguous failed to reinsert the pending gate | Explicitly suspend readiness for ambiguous current input | Regression passed |
| Old turn completion could clear/consume a newer turn's correction | Require the current turn ID before mutating completion ownership | Regression passed |
| Child definitions and write boundaries shared one digest map | Separate child definition and scope digests so an authorized in-scope correction is distinct from expansion; `correction_definition_change_is_distinct_from_scope_expansion` | Regression passed |
| YAML scalar coercion accepted non-string command arguments | Preserve scalar types before typed validation | Config/harness regression passed |
| Git/delivery review found incomplete plan coverage, unaccounted candidate deltas, weak carry verification and discarded recovery history | Exact repository/target/step and child contribution sets, non-deserializable verified carry/full production checks, retained failed-step history | Four regressions passed |
| A valid literal workflow could omit required task/context fields | Append mandatory normalized context as a distinct layer regardless of template interpolation; preserve its digest | Production-path regression passed |
| Assembly accepted scratch inside repository source | Reject overlap in both directions, in addition to the execution-boundary checks | Regression passed |
| Conflicting recovered release acquisition partly removed existing resource locks | Validate every conflict before changing lock ownership | Regression passed |

The initial combined strict Clippy run failed on collapsible conditions, large enum size and excessive argument lists. These were refactored and strict Clippy subsequently passed for the foundation targets; no lint suppressions or weakened checks hid the findings. This failed run remains part of the milestone record. The first combined run passed 87 tests; two context-assembly regressions and one atomic lock-reconciliation regression bring the foundation suite to 90. Later provider/evaluation work is outside these results.

## Qualification blockers

The current outer sandbox rejected a Codex macOS sandbox probe with `sandbox-exec: sandbox_apply: Operation not permitted` (exit 71). No unsandboxed retry or host security change was made. Named filesystem permission configuration was accepted by the installed strict startup parser in an owned credential-free Codex home, but actual enforcement canaries remain unverified. See `../docs/protocol.md` for the exact profile and probe details.

Tailscale, ffmpeg and ffprobe were not found on PATH during initial inspection. No live account/model execution, account authentication, persistent installation, remote mutation, deployment or human review was attempted. Their bounded plans and explicit prerequisites must be prepared before supervised qualification.

## Contract inventory

The initial `coverage.json` selector enumerated363 blocks through §12 and136 named requirements. Its earlier claim to include all normative prose was incorrect: it omitted descriptive mandatory content, required fenced forms and §§13–14. The [inventory correction](LOCAL-CONTRACT-INVENTORY.md) retains every original evidence entry while adding133 unassessed content blocks. Historical consistency checks remain valid for their exact selector; they do not establish complete normative coverage. The grouped progress ledger remains in `../PROGRESS.md`.
