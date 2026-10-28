# Kegari v0

Kegari v0 is the scoped Rust delivery coordinator defined by [SPEC 1.4](SPEC.md).
Implementation and qualification remain in progress. The current build is not
qualified to coordinate the lasting implementation. See [PROGRESS.md](PROGRESS.md)
for the actual checkpoint, failed or blocked checks, and current work.
[MILESTONES.md](MILESTONES.md) defines version targets, immediate 0.2.0 correction
packages, completion gates and progress reporting. Goal mode is cancelled; work proceeds through individual planning and execution prompts.

The [`kegari-cli` package](../kegari-cli/README.md) now owns the public CLI, control
client and `kegari-v0` executable. This package retains the backend library and
`kegari-v0-evaluation` inspection tool. Both CLI and backend import shared command,
response, error, snapshot, run and artifact data directly from
[`kegari-types`](../kegari-types/README.md); there are no backend compatibility
reexports. The backend retains current-operator authorization, exit-code policy and domain
state. Authenticated transport, command validation/classification, operator
identity and snapshot operations now belong to
[`kegari-control-api`](../kegari-control-api/README.md). Control-host composition,
dispatch, supervision and the child control channel belong to
[`kegari-controller`](../kegari-controller/README.md).

Import shared runtime traits/dependencies from `kegari_v0::runtime_host`, and
command identity/current-actor helpers from `kegari_v0::control_command`. The old
backend `api`, `app`, `host`, `supervisor`, `orchestration`, `transport` and
`snapshot` modules and `control::{Actor, Principal}` paths are removed without
compatibility reexports. Identity lives at `kegari_control_api::identity`.
Controller depends on this backend; the backend has no controller dependency.
Registration, durable admission, configuration transactions and the runtime
factory/engine remain here. The control-host process still executes the driver;
the child still handles the runtime lock and lease/heartbeat/poll channel.

Protected filesystem access, durable record encoding and locking now belong to
[`kegari-storage`](../kegari-storage/README.md), imported as `kegari_storage`.
Domain records and recovery policy remain here.

Guarded spawning and owned process-group handling now belong to
[`kegari-process`](../kegari-process/README.md). Import the moved interfaces from
`kegari_process::{spawn, group}`; there are no backend compatibility reexports.
The process crate owns the single spawn/retirement interlock and descriptor
registry. `kegari-execution` owns environment policy, process-journal mechanics
and native command cleanup. This backend retains journal admission, control
permits, recovery policy, registration, services and application supervision. The CLI has no direct process dependency.

OS account/host identity, executable inspection, process identity/probes,
`OwnedChild`, free-space queries and browser launch now belong to
[`kegari-platform`](../kegari-platform/README.md), imported directly from
`kegari_platform` by CLI and backend. The existing backend `platform` module
retains registration, owner records, runtime locks, service planning/lifecycle,
handover, `BuildInfo` and installed-build verification. It does not reexport the
moved interfaces. Private native probing remains with the platform helpers;
application authorization and build identity remain here.

Secret values, Keychain access, explicit reference resolution and generic
attestation signing now belong to
[`kegari-secrets`](../kegari-secrets/README.md). CLI and backend import `Secret`,
`SecretStore`, `Keychain`, `resolve_reference`, `AttestationKey` and `Signed<T>`
directly from `kegari_secrets`; the backend provides no compatibility reexports.
The removed `Secret::credential()` method becomes the backend-only
`connections::provider_credential(&Secret)` helper. Publication framing, evidence
validation, credential-import authorization and configuration transactions remain
here. Keychain names, signed-record formats and errors are unchanged.

Configuration models, TOML parsing, validation and path resolution now belong to
[`kegari-config`](../kegari-config/README.md), imported directly as `kegari_config`.
The backend's `config` module retains `config_revision(&Config)`,
`load_workflow(&Workspace)`, `load_workflows(&Config)`, `ConfigReload` and
`inspect_reload`; it does not reexport configuration types. Configuration
transactions and provider verification remain in `configuration`, while shared
canonical hashing now belongs to `kegari-digest`. See the configuration crate's
[Rust migration notes](../kegari-config/README.md#rust-consumer-migration).

GitHub/Linear transport, records and fixed typed network operations belong to
[`kegari-providers`](../kegari-providers/README.md). This backend retains
`providers::ProviderBroker`, `ProviderMutation`, hosted-proof verification/replay
and receipt-bound Linear asset admission. Typed writes are called only through
the existing durable reservation and final authorization sequence. Import moved
adapter types directly from `kegari_providers`; import hash functions directly
from [`kegari-digest`](../kegari-digest/README.md), without compatibility reexports.

Codex protocol data, `RpcClient`, `SessionState`, request/tool gates, history,
protocol recovery, observations, stream transport and pinned schemas now belong
to [`kegari-codex`](../kegari-codex/README.md). Import them directly from
`kegari_codex`; the old backend `codex` module is removed without compatibility
reexports. Native launch, permission derivation, executable verification,
connection probing and owned cancellation/cleanup live in `codex_host`.
`NativeCodexClient` owns the actual child and lends its protocol client through
`client_mut()`. Native methods formerly on `RpcClient` are now `codex_host` free
functions. Client timing fields have read-only accessors.

`IsolationSessionScope` delegates revalidation to `ValidatedIsolation`;
native host APIs still require that concrete type. Backend
`worker_run::RetainedResumeSession` remains an opaque protected-journal wrapper
around ordinary `kegari_codex::ResumeCandidate` data. Receipt lineage, joined
recovery and authorized resume stay here. Protocol observations and transport EOF
supply no native authority. `RunReference` and `PhaseKey` are shared data in
`kegari_types::run`; receipt parsing uses
`control::run_reference_from_receipt(&Receipt)` and validation stays private to
Control. See the [final extraction report](validation/CODEX-CLIENT-EXTRACTION.md).

Workflow models, parsing, bounded rendering, instruction assembly, embedded assets
and sealed resume compatibility now belong to
[`kegari-context`](../kegari-context/README.md). Import the moved interfaces from
`kegari_context::{workflow, instructions}`; the old backend modules are removed
without compatibility reexports. Production assembly supplies `crate::BUILD_ID`
through `AssemblyInput::runtime_build`, retaining the single application identity.
Configuration workflow loading and required-check cross-validation remain here,
alongside execution, protected archives and admission authority.
`ArtifactProof` is descriptive data in `kegari_types::artifact`; the former
`ArtifactProof::reference()` method becomes `execution::artifact_reference(&proof)`.
See the [context extraction report](validation/CONTEXT-EXTRACTION.md).

Synchronous scheduling, capacity/priority selection, attempt-accounting helpers
and retry timing now belong to [`kegari-scheduler`](../kegari-scheduler/README.md).
Import these interfaces directly from `kegari_scheduler`; the backend `scheduler`
module is removed without compatibility reexports. The runtime driver retains
configuration-to-limit conversion, durable admission, receipts, recovery and its
existing locking/scheduler replacement sequence. The scheduler's `Attempts` helper
is preserved without introducing new production use. The distinct private release
attempt record stays here. See the
[scheduler extraction report](validation/SCHEDULER-EXTRACTION.md).

Git records, naming rules, pure validation and canonical digest calculations now
belong to [`kegari-git`](../kegari-git/README.md). Import the fourteen moved types,
constants and helpers directly from `kegari_git`; this backend provides no
compatibility reexports. The existing `validate_repo_path`,
`CommitIdentity::validate` and `SourceCommitMetadata::validate` checks are public
for retained backend callers without changing their behavior.

This is the first Git extraction pass. `git::Git`, its operation request structs,
process scopes, journal admission, gates and cancellation remain here, together with clone
ownership, protected recovery and privately constructed candidate/integration
proofs. Branch construction and delta ordering still perform their existing Git
commands. No unchecked constructors, runner abstraction or new execution authority
are introduced. The pinned Git version, serialized records and digest inputs are
unchanged.

Managed environments, isolation, native commands, outputs, cancellation and joined
cleanup belong to [`kegari-execution`](../kegari-execution/README.md). Import those
interfaces from `kegari_execution` and process records/guards from its `journal`
module. Backend `execution_host` now contains only worker/invocation/bundle
adapters, while `process_records::ProcessJournal` is the application admission
facade over the concrete journal store. The store provides checked resource
ownership; the facade retains sealed host and worker admission. Application
execution/evidence, receipts, reservations and recovery policy remain here.
`Lease` is shared data in `kegari_types::control`. There are no compatibility
reexports or changes to command prerequisites.

Production environment construction and managed-domain validation explicitly
supply `crate::BUILD_ID`. Validated isolation retains this expectation privately
through revalidation and cwd derivation; persisted formats are unchanged. Codex
protocol calls borrow `codex_host::IsolationSessionScope`, whose constructor does
no validation or I/O. Execution infrastructure ownership is complete; application
admission and recovery remain backend-owned, with no qualification claim.

## Build and deterministic checks

Run from the Rust workspace root with the pinned Rust 1.99.1 toolchain. Workspace
test and lint commands cover all seventeen packages:

```sh
cargo build --offline --locked -p kegari-cli --bin kegari-v0
cargo test --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
```

The build conversation uses an already installed `stable` alias verified to be
that exact toolchain and an isolated Cargo cache; those local environment values
are recorded in [PROGRESS.md](PROGRESS.md). A successful build or fixture test
does not establish native isolation, live provider compatibility or human review.
Native tests remain explicitly ignored in ordinary suite runs. The separately
approved [NQ-01A identity check](validation/NATIVE-IDENTITY.md) passed once.
[NQ-02C](validation/LOADER-BOOTSTRAP.md) passed three fixed foreground cases after
the loader correction; the earlier NQ-02A/B failures remain recorded. The separate
NQ-01B check and broader native execution/service qualification remain pending;
new requests must follow the generic backend and milestone plan.

## Versioned prerelease executable

The archived stabilization checkpoint is `0.2.0-alpha.10`, built before CLI
extraction. Its final release suite passed 1,260 tests, with
three native qualification tests ignored. See the artifact links, checksums and
retained results in the [stabilization report](validation/LOCAL-DISCOVERY-ADMISSION.md). The Cargo-specific
execution design has been removed. The source now separates `kegari-cli`,
`kegari-types`, `kegari-config`, `kegari-digest`, `kegari-providers`,
`kegari-storage`, `kegari-process`, `kegari-platform`, `kegari-secrets`,
`kegari-control-api`, `kegari-controller`, `kegari-codex`, `kegari-context`,
`kegari-scheduler` and `kegari-git`
from the remaining backend while retaining the
same workspace version and executable name. Its build identity changes with the refactor; the archived alpha.10 bytes
and historical evidence remain untouched. This extraction does not package a
new prerelease, and native execution remains blocked.
Repository commands still refuse missing profiles (`process_tree_unverified`) or
unqualified declared profiles (`managed_profile_unqualified`), and native,
provider, actual-agent and human qualification remain incomplete.

Build the optimized executable from the workspace root:

```sh
cargo build --offline --locked --release -p kegari-cli --bin kegari-v0
./target/release/kegari-v0 --version
./target/release/kegari-v0 --help
./target/release/kegari-v0 __build-info
./target/release/kegari-v0 __handover-info
```

These inspection commands do not set up or start the coordinator. The hidden
metadata commands are exact internal forms, useful here for checking the build;
they are not an installation or handover procedure. `__handover-info` validates
the embedded instruction catalog before reporting its version and asset hashes.
The runtime guidance, generic default workflow and optional starter are embedded
in the executable, so they do not require a companion checkout. Instruction asset
version `7` and persisted
schema versions are independent of the application's prerelease version.

`--version` reports the application version and source/profile build digest.
Record the executable's SHA-256 separately: the build digest is not its file
checksum or a complete record of compiler settings. Release and debug profiles
have different build identities. The build output is a development artifact;
operational use requires a separately authorized immutable installation and
qualification under SPEC §11.7. Building does not install, activate or replace a
running coordinator. The rejected offline Cargo execution adapter has been removed.
Generic execution and agent-led discovery now follow SPEC 1.4 and MILESTONES.md.
Discovered child and integrated validation require independent adequacy review.
Compact publications resolve complete private archives, including failed-attempt
artifacts. Review receives the original history and bounded artifact contents;
inputs exceeding the full 1 MiB prompt limit remain blocked until paging exists.
See [the archive evidence](validation/LOCAL-VALIDATION-ARCHIVES.md) for exact local
checks and retained limits. These records do not confer native qualification.
The historical [alpha.8 resource binding](validation/MANAGED-RESOURCES.md)
records bounded SDK/library observations and explicit mappings, with 1,117
passing tests for that source. It does not establish SPEC 1.4 conformance.
The [alpha.7 loader correction](validation/LOADER-BOOTSTRAP.md)
records the exact staged executable, 97 passing affected local tests and three
passing native foreground cases. The qualification prerequisite remains enforced.
The [alpha.6 managed check increment](validation/MANAGED-CHECK-BUNDLES.md)
records finite registered services and the historical 1,097-test full suite. The earlier
[alpha.5 command supervision evidence](validation/MANAGED-COMMAND-SUPERVISION.md)
remains historical. The
[alpha.4 setup/profile report](validation/MANAGED-SETUP.md),
[alpha.3 report](validation/OWNED-GROUP-LIFECYCLE.md),
[alpha.2 report](validation/LOCAL-SERVICE-SHUTDOWN.md) and
[alpha.1 report](validation/LOCAL-PRERELEASE.md) remain historical evidence.

## First launch

Running `./kegari-v0` without a command requests the local runtime's `overview`;
it does not open a setup wizard or start a service. Before setup, alpha.4 and later report
`target_unavailable: no local runtime is configured` and directs you to
`kegari-v0 setup --help`. Alpha.2's earlier message was
`no registered local runtime; use explicit local setup`; both mean that no local
runtime has been configured yet. Use `./kegari-v0 --help` or
`./kegari-v0 setup --help` to inspect the available commands without setup.

Alpha.10 is a stabilization checkpoint with native execution still blocked;
no current build is qualified for coordinator activation. The intended [first-install sequence](docs/cli.md#first-install-and-local-status)
requires the supported profile and native/provider prerequisites to be established
first, then a final protected, versioned binary and explicit runtime configuration.
Interactive initial local `setup` now derives host/account identity, generates the
runtime UUID and asks for operator identity, concurrency, execution root and roles.
It displays a validated initial TOML document with no provider/workspace scope
and submits it only after `create`. Provider connections, verification and exact
workspace policy remain subsequent explicit configuration steps. `setup --from`
continues to accept complete TOML; existing/remote setup uses the document editor.
The starter `WORKFLOW.md` supplies repository guidance, not runtime configuration.

Setup, service installation and control-host start are separate actions. They do
not qualify the prerelease or implicitly start orchestration. Missing native,
provider and execution prerequisites remain blockers.

## Operator and repository preparation

Read the [configuration contract](docs/configuration.md),
[CLI command guide](docs/cli.md), [native host lifecycle](docs/native-host.md),
and [control transport](docs/transport.md) before installation or activation.
The [starter WORKFLOW.md](../kegari-context/examples/WORKFLOW.md) is optional guidance. Omitting a
workflow selects the installed generic workflow; selecting an invalid file still
fails. Agents retrieve project commands, technology boundaries and validation
needs from accessible repository instructions, documentation, configuration and
scripts. Empty repositories begin from the authorized task and actual observed
state. Preserve existing repository guidance; runtime-owned instructions are
shipped in [instructions/](../kegari-context/instructions/) and bound to the installed binary.

An operator must supply canonical account/workspace/repository identities,
the runtime's protected executable and credential locations, explicit scoped
actors/resources and release policy. Project command lists and tool/environment
inventories are not operator prerequisites under SPEC 1.4. The remaining legacy
catalog/check interfaces are being replaced; the current source does not yet
deliver that complete command-discovery path. Native isolation and actual
CI/review/deployment facts must be established before their dependent gates.
Repository `ci`, `production_ci`, `no_change_validation` and `deployment` may be
omitted during discovery; missing policy blocks dependent delivery. Only explicit
`deployment = "none"` declares deployment-free behavior. Run `config check` after
setup to see repository prerequisites: `configuration_admission: verified` can
coexist with `status: blocked` for unresolved delivery, and operational qualification
remains unverified.
Setup, service installation, live evaluation,
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

The [progression report](validation/LOCAL-PROGRESSION.md) records the historical
523-test offline checkpoint. The earlier
[integration report](validation/LOCAL-INTEGRATION.md) preserves its historical
411-test result. Newer work and stale evidence are identified in
[PROGRESS.md](PROGRESS.md). The [coverage inventory](validation/coverage.json)
indexes normative requirements; it is not a conformance certificate.

The [evaluation guide](docs/evaluations.md) describes the existing V0-E01–08
fixtures, finite trial bounds, protected graders, provenance and human-review
requirements. SPEC 1.4 additionally requires V0-E09 generic-discovery variants;
their implementation and qualification remain open. Actual-agent trials, real provider integrations, native service and
isolation checks, and required human calibration are separate evidence classes.
Supervised, bounded native component results are recorded above. Other live steps
still require separate authorization and evidence.
