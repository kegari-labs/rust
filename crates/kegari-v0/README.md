# Kegari v0

Kegari v0 is the scoped Rust delivery coordinator defined by [SPEC 1.5](SPEC.md).
Implementation and qualification remain in progress. The current build is not
qualified for operational coordination. See [PROGRESS.md](PROGRESS.md)
for the actual checkpoint, failed or blocked checks, and current work.
[MILESTONES.md](MILESTONES.md) defines version targets, immediate 0.2.0 correction
packages, completion gates and progress reporting. The six-hour [goal](GOAL.md)
uses the fixed timestamps and current state in PROGRESS.md; reading or editing
these documents does not start or restart its clock.

**Redesign in progress:** Codex owns repository execution;
Kegari owns orchestration and acceptance. The current pass connects native Codex
tools to production workflows and deletes the superseded repository harness,
preserving host machinery still needed for Codex, Git and control processes.
Feature implementation uses minimum sufficient validation; comprehensive tests
and qualification follow later. The package descriptions below describe the
current alpha.10 source, not completion of this migration.

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
registry. Shared process-journal mechanics currently live in `kegari-execution`.
This backend retains host journal admission, control permits, recovery policy,
registration, services and application supervision. Retain actual Codex/Git/control
callers while deleting the custom repository executor. The CLI has no direct
process dependency.

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

Current isolation-derived permission adapters await the Codex-owned execution
migration; their old environment contracts are not forward requirements. Backend
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

The custom repository executor formerly split between
[`kegari-execution`](../kegari-execution/README.md), `execution_host` and worker
adapters has been removed. Do not restore its command renderer, sandbox
generation, environment inventory or per-command admission/journals.
Codex app-server supplies native repository execution and supported terminal
lifecycle; Kegari retains operation/session authority and result acceptance.
Shared journal/process mechanisms with actual Codex, Git and control callers stay.
Old evidence retains its original identities and must not be treated as Codex
execution proof. Shared `Lease` data remains in `kegari_types::control`.

## Build and deterministic checks

Use the pinned Rust 1.99.1 toolchain and cached dependencies. Follow the
[feature-first development policy](../../docs/development.md#feature-first-development-policy):
compile affected code and use the smallest relevant existing check, rather than
running the complete workspace suite after each increment.

```sh
cargo check --offline --locked -p kegari-v0
cargo build --offline --locked -p kegari-cli --bin kegari-v0
python3 crates/kegari-v0/tools/contract_inventory.py --check
```

These are examples for applicable changes, not a mandatory per-edit sequence.
Comprehensive workspace tests, Clippy and authorized native/provider/actual-agent
qualification follow working product features in milestone 0.7. Existing native
qualification tests remain ignored in ordinary runs. Historical supervised checks
and failed attempts retain their original source/profile scope in the
[progress record](PROGRESS.md); they do not require a new custom-executor
qualification project before the Codex-backed implementation can proceed.

## Versioned prerelease executable

The archived stabilization checkpoint is `0.2.0-alpha.10`, built before CLI
extraction. Its final release suite passed 1,260 tests, with
three native qualification tests ignored. See the artifact links, checksums and
retained results in the [stabilization report](validation/LOCAL-DISCOVERY-ADMISSION.md).
The current [seventeen-crate workspace](../../docs/workspace.md) retains the same
version and executable name, with a source-dependent build identity. Its archived
alpha.10 bytes and historical evidence remain untouched; no new release was
packaged by the extractions. The duplicated repository executor has been removed;
Codex-native workflow integration is in progress. Native, provider, actual-agent
and human qualification remain incomplete.

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
version `8` and persisted
schema versions are independent of the application's prerelease version.

`--version` reports the application version and source/profile build digest.
Record the executable's SHA-256 separately: the build digest is not its file
checksum or a complete record of compiler settings. Release and debug profiles
have different build identities. The build output is a development artifact;
operational use requires a separately authorized immutable installation and
qualification under SPEC §11.7. Building does not install, activate or replace a
running coordinator. The SPEC 1.5 migration removed the custom executor and
updated embedded tool guidance with explicit persisted and instruction identities.
Current implementation and remaining limitations are recorded in PROGRESS.md.

Discovered validation, independent adequacy review, protected artifact archives
and evidence acceptance remain workflow responsibilities. Complete paging and
Codex-result integration are still required; historical evidence does not prove
those new production paths. The [archive report](validation/LOCAL-VALIDATION-ARCHIVES.md)
and [stabilization report](validation/LOCAL-DISCOVERY-ADMISSION.md) preserve prior
results and source identities. Earlier native failures and component checks remain
in the [historical progress record](PROGRESS.md), not the current implementation
queue. Archived binaries are unchanged.

## First launch

Running `./kegari-v0` without a command requests the local runtime's `overview`;
it does not open a setup wizard or start a service. Before setup, alpha.4 and later report
`target_unavailable: no local runtime is configured` and directs you to
`kegari-v0 setup --help`. Alpha.2's earlier message was
`no registered local runtime; use explicit local setup`; both mean that no local
runtime has been configured yet. Use `./kegari-v0 --help` or
`./kegari-v0 setup --help` to inspect the available commands without setup.

Alpha.10 is a stabilization checkpoint with the old repository-command path still blocked;
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
inventories are not operator prerequisites under SPEC 1.5. The remaining legacy
custom-tool interfaces await migration and deletion; the current source does not yet
deliver that complete command-discovery path. Supported Codex permission/capability
checks and actual CI/review/deployment facts must precede their dependent gates.
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
requirements. SPEC 1.5 retains V0-E09 Codex-led discovery variants;
their implementation and qualification remain open. Actual-agent trials, real provider integrations, native service and
isolation checks, and required human calibration are separate evidence classes.
Supervised, bounded native component results are recorded above. Other live steps
still require separate authorization and evidence.
