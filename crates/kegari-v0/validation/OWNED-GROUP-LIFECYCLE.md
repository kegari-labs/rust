# Owned-group lifecycle and alpha.3 artifact

Alpha.3 binds process-journal completion to an opaque exit observation from the
original owned child and process group. Git, provider and command owners retain
that observation across persistence errors. A foreign exit, changed durable row,
surviving inherited guard or unresolved output stream cannot grant completion.
The supported foreground profile and registered-service integration remain
incomplete; repository commands still refuse `process_tree_unverified`.

The final source passed **1,025 release-profile tests**, with zero failures and
two explicitly ignored native tests across 39 test groups. All-target clippy with
warnings denied, formatting, the contract inventory and a separate release binary
build also passed. Five direct inspections of the staged executable passed.
These are local development checks, not operational qualification.

| Area | Implemented and checked | Limit |
| --- | --- | --- |
| Owned exit evidence | Original group/child binding; single reaping token retained across journal retries; foreign-token and valid-envelope substitution rejection | Group evidence does not cover escaped descendants |
| Group observation | Two bounded member inventories and repeated birth identities; changed, denied, saturated or disappearing observations refuse closure | Conservative observations, not an atomic kernel snapshot |
| Guard ownership | Inherited exclusive lock still excludes observers; shared readers coexist; observed surviving holder permanently retains uncertainty | Later descriptor release does not establish the holder's process exit |
| Process recovery | New schema-2 verified/unverified states; schema-1 serialization and uncertainty preserved | Historical evidence grants no live authority or unfinished-operation retirement |
| Git cancellation/output | Bounded nonblocking drain; unresolved streams latch uncertainty; cancellation rereads it after closure | Actual remote operations were not exercised |
| Handover metadata | Schema 2 explicitly requires process-journal schema 2; missing/old support rejected before mutation | Public handover, recovery and qualification workflow remain unfinished |
| First launch | Missing registration points to setup help; header says `Target: local`; no implicit setup or service start | Guided configuration generation is still missing |

Independent reviews found and drove corrections to the same-group fork race,
the Git output/cancellation race, internal observer-lock contention and the
incorrect inference that a previously surviving holder's descriptor release
proved exit. Regressions exercise those cases. Final independent source review
reported no additional material issue. First-launch review confirmed the actual
setup/service/orchestration sequence; current documentation now names alpha.3.

Commands run from the Rust workspace root with
`RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

```sh
cargo test --locked --release -p kegari-v0 --no-fail-fast -- --test-threads=2 --nocapture
cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
cargo build --locked --release -p kegari-v0 --bin kegari-v0 --message-format=json
```

Rust/Cargo are 1.99.1, host `aarch64-apple-darwin`, LLVM 23.1.1. The inventory
still contains 503 contract clauses and 136 named requirements; no coverage was
promoted by this build. Machine identities, commands, log hashes, review and
limitations are in [owned-group-lifecycle-checks.json](owned-group-lifecycle-checks.json).

Artifact: [kegari-v0 0.2.0-alpha.3](/Users/lgr/Development/com.kegari/com.kegari.rust/target/prereleases/kegari-v0-0.2.0-alpha.3-aarch64-apple-darwin-f0435ae248dc/kegari-v0).
The 40,409,312-byte executable has mode `0555` and SHA-256
`f0435ae248dc5da792927bf326c9addce8f3b75c343512fd60177a729aef5dee`.
Its independently reproduced source/profile build digest is
`sha256:afe0b54c45d229cabee79ac64730db16d749d749bbceb5d7bc475bf04591b1d3`.
Adjacent `artifact.json` records every build input; `test-inputs.json` records
the retained test/evaluation file hashes. Required licenses/notices accompany it.
The alpha.2 staged artifact is unchanged.

The staged executable passed `--help`, `--version`, `setup --help`, `__build-info`
and `__handover-info`, each from an empty disposable cwd, with minimal PATH,
closed stdin and a 15-second timeout. No files appeared in that cwd. Embedded
runtime version and all six instruction hashes match source; instruction asset
version remains 3. Process-journal and handover metadata schemas are now 2;
configuration, receipt and instruction schemas are otherwise unchanged.

The full suite used Cargo-generated test artifacts before the separate final
release build. Only the five inspections above ran against the staged binary;
no binary-equivalence or complete reproducible-build claim is made. All source
inputs remained unchanged between the final suite and the staged build.

Earlier outcomes are retained in the machine record and adjacent validation logs:

- An initial broad debug run was interrupted with 13 observed failing test lines
  and no complete test-group summary. One isolated case later passed. The causes
  of all those debug failures were not established; that run is not passing
  evidence and the complete debug suite was not rerun.
- The first completed intermediate release suite had 1,014 passes, four failures
  and two ignored tests. Three assertions still expected the legacy recovery
  state; they now require schema-2 evidence without claiming all-descendant exit.
  The fourth was a continuation fixture's timing assumption; it now respects the
  actual returned retry deadline while retaining the existing bound/assertion.
- The corrected intermediate release suite passed 1,023 tests. The final 1,025
  result additionally covers shared observation locks and stronger durable-row
  substitution. Interrupted build/test attempts are recorded as interrupted.

No real setup, Keychain initialization, service installation/start, coordinator
activation, native qualification, live provider/agent evaluation or human
qualification was performed. The executable and raw logs remain local development
artifacts under Cargo `target`, removable by cleanup. This is not an installed
runtime or completion of milestone 0.2.0. Detached/unregistered descendants remain
outside the approved managed-workload guarantee; full coverage is required before
qualification for untrusted external context such as public PR material.
