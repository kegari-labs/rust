# Kegari v0

Kegari v0 is the scoped Rust delivery coordinator defined by [SPEC 1.3](SPEC.md).
Implementation and qualification remain in progress. The current build is not
qualified to coordinate the lasting implementation. See [PROGRESS.md](PROGRESS.md)
for the actual checkpoint, failed or blocked checks, and current work.

## Build and deterministic checks

Run from the Rust workspace root with the pinned Rust 1.99.1 toolchain:

```sh
cargo build --locked -p kegari-v0
cargo test --locked -p kegari-v0
cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
```

The build conversation uses an already installed `stable` alias verified to be
that exact toolchain and an isolated Cargo cache; those local environment values
are recorded in [PROGRESS.md](PROGRESS.md). A successful build or fixture test
does not establish native isolation, live provider compatibility or human review.
The two explicitly ignored native tests remain blocked, not passed.

## Versioned prerelease executable

The application is versioned as `0.2.0-alpha.1`; the package and executable remain
`kegari-v0`. This prerelease does not complete the 0.2.0 containment milestone.
Repository command execution still refuses `process_tree_unverified`, and native,
provider, actual-agent and human qualification remain incomplete.

Build the optimized executable from the workspace root:

```sh
cargo build --locked --release -p kegari-v0 --bin kegari-v0
./target/release/kegari-v0 --version
./target/release/kegari-v0 --help
./target/release/kegari-v0 __build-info
./target/release/kegari-v0 __handover-info
```

These inspection commands do not set up or start the coordinator. The hidden
metadata commands are exact internal forms, useful here for checking the build;
they are not an installation or handover procedure. `__handover-info` validates
the embedded instruction catalog before reporting its version and asset hashes.
The six role assets and starter workflow are embedded in the executable, so they
do not require a companion checkout. Instruction asset version `3` and persisted
schema versions are independent of the application's prerelease version.

`--version` reports the application version and source/profile build digest.
Record the executable's SHA-256 separately: the build digest is not its file
checksum or a complete record of compiler settings. Release and debug profiles
have different build identities. The build output is a development artifact;
operational use requires a separately authorized immutable installation and
qualification under SPEC §11.7. Building does not install, activate or replace a
running coordinator. See [prerelease evidence](validation/LOCAL-PRERELEASE.md)
for the exact artifact and checks performed.

## Operator and repository preparation

Read the [configuration contract](docs/configuration.md),
[CLI command guide](docs/cli.md), [native host lifecycle](docs/native-host.md),
and [control transport](docs/transport.md) before installation or activation.
The [starter WORKFLOW.md](examples/WORKFLOW.md) documents the target-specific
commands, tools, resources and acceptance inputs that must be supplied. Preserve
the target repository's existing instructions; runtime-owned guidance is shipped
in [instructions/](instructions/) and bound to the installed binary.

An operator must supply canonical account/workspace/repository identities,
protected executable and credential locations, explicit scoped actors/resources,
exact CI/review/deployment contracts, and qualified native isolation. Missing
facts stay visible as blockers. Setup, service installation, live evaluation,
provider mutation and deployment require their own operational authorization;
none is performed by the build/test commands above.

## Runtime and evidence design

- [Scheduling and recovery](docs/runtime-driver.md),
  [worker preparation](docs/runtime-workers.md),
  [owned worker lifecycle](docs/worker-run.md), and
  [process recovery](docs/process-recovery.md).
- [Provider adapters](docs/providers.md),
  [correction publication](docs/corrections.md),
  [delivery evidence](docs/delivery.md),
  [deployment observations](docs/deployment.md), and
  [operator decisions](docs/decisions.md).
- [Execution boundaries](docs/execution.md),
  [configuration transactions](docs/configuration.md), and
  [preservation and cleanup](docs/housekeeping.md).

Local tests, hosted CI, Codex review, staging, integrated evidence, human
acceptance and release remain separate gates. A worker summary, successful exit,
tracker state or signed stale record cannot substitute for them. Unknown effects
retain their original receipts and are observed before any further action.

## Qualification evidence

The [progression report](validation/LOCAL-PROGRESSION.md) records the current
523-test offline checkpoint. The earlier
[integration report](validation/LOCAL-INTEGRATION.md) preserves its historical
411-test result. Newer work and stale evidence are identified in
[PROGRESS.md](PROGRESS.md). The [coverage inventory](validation/coverage.json)
indexes normative requirements; it is not a conformance certificate.

The [evaluation guide](docs/evaluations.md) describes the versioned V0-E01–08
fixtures, finite trial bounds, protected graders, provenance and human-review
requirements. Actual-agent trials, real provider integrations, native service and
isolation checks, and required human calibration are separate evidence classes.
The current unattended build has not run or authorized those live steps.
