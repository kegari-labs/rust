# Developing Kegari

This guide describes the Rust workspace, package boundaries and local verification.
For the product introduction and preview instructions, see the [README](../README.md).

## Feature-first development policy

The approved [SPEC 1.5](../crates/kegari-v0/SPEC.md) makes Codex app-server the
repository execution layer and Kegari its orchestrator. The
[implementation queue](../crates/kegari-v0/MILESTONES.md) begins with one complete
Codex-backed production flow, deletion of the superseded harness, then remaining
user-facing features. The native production path is being integrated; the custom repository executor
has been removed. PROGRESS.md records the latest completed checks and remaining work.

- Agents **MUST prioritize working features** and use the smallest appropriate
  compile check, focused test or smoke exercise needed to proceed.
- Fix observed failures attributable to the change. Add focused coverage only
  where needed to establish changed behavior or reproduce a concrete defect.
- Agents **MUST NOT routinely run complete workspace suites, exhaustive failure
  matrices, repeated independent reviews or broad conformance audits** during
  feature implementation. Comprehensive regression, end-to-end coverage and
  qualification belong to the later stabilization phase.
- Deferred comprehensive testing **MUST NOT block dependency-ready feature
  implementation**, including later milestones. Keep implementation, focused
  validation and qualification status distinct; do not claim release readiness.
- Delete obsolete architecture-specific production code, configuration,
  instructions, tests and dependencies as their callers migrate. Record the
  rationale briefly. Do not keep the discarded execution harness behind flags,
  wrappers or fallback paths; preserve applicable tests and shared host machinery.
- Keep checkpoints concise. No speculative framework, further crate extraction
  or validation infrastructure is a prerequisite to delivering the next feature.

This development policy does not waive the checks and acceptance rules Kegari
must enforce for its target projects. Native/live-operation restrictions remain
in force. The [goal](../crates/kegari-v0/GOAL.md) retains its original six-hour
deadline across continuations; consult [PROGRESS.md](../crates/kegari-v0/PROGRESS.md)
for the current state and absolute timestamps. The final twenty minutes are
reserved for integration, build and handoff. Reading documentation does not
activate a goal, restart its clock or authorize restricted operations.

## Workspace packages

See the [workspace overview](workspace.md) for the crate index and a concise
summary of responsibilities.

## Current package boundaries

Both CLI and backend depend directly on `kegari-config` and `kegari-types`;
`kegari-config` depends on `kegari-types`. The CLI also temporarily depends on the
backend; the backend does not depend on the CLI. Shared types and configuration
models are imported directly from their owning crates, without backend
compatibility reexports. Configuration transactions, workflow integration,
current-operator authorization and exit-code mapping remain in the backend.
Command validation and snapshot operations now belong to `kegari-control-api`. Shared hashing belongs to `kegari-digest`.

Provider adapters depend on config/types/digest; the backend retains the mutation
broker, receipts and evidence admission. Neither new crate depends on the backend.

CLI and backend also depend directly on `kegari-storage`, which depends on
`kegari-types` and `kegari-digest`. Domain persistence policy remains in the backend.

The backend also depends directly on `kegari-process`, which owns the single
spawn/retirement interlock and owned process-group handling. It depends on
storage/types without a backend dependency; the CLI has no direct process
dependency. Process-journal mechanics belong to `kegari-execution`; application
authorization and supervision remain in the backend.

CLI and backend import native host helpers directly from `kegari-platform`, which
depends on process/digest/types without a backend dependency. Registration,
runtime ownership, services and application build metadata remain in the backend.

CLI and backend import secret values directly from `kegari-secrets`, which also
owns Keychain access, explicit reference resolution and generic attestation
signing. It depends on types/digest without a backend or provider dependency.
Provider credential conversion, authorized imports, publication framing and
delivery-evidence policy remain in the backend.

The controller temporarily depends on the backend and control API. The backend
depends on control API, with no reverse dependency on controller, including in
tests. Shared runtime contracts and command-policy helpers remain below the
controller in `kegari_v0::{runtime_host, control_command}`. Registration, durable
admission, configuration transactions and workflow/delivery policy remain
backend-owned. Repository execution now uses Codex-native observations; remaining integration is tracked in PROGRESS.md. The control host runs the runtime driver; the orchestration child
owns the runtime lock and exchanges lease, heartbeat and poll messages. SPEC 1.5
retains that arrangement and its exclusive ownership/admission fencing; moving
the driver to the child is not part of the execution migration.

The backend depends directly on `kegari-codex` for protocol/session ownership,
request sequencing, deadlines, protocol recovery and observations. Its production
workspace dependencies are shared types and digest; disposable storage fixtures
are test-only. It has no backend dependency. Native launch and owned cleanup stay
in `kegari_v0::codex_host`; protected retained sessions, receipt lineage and
execution admission remain backend-owned. Shared run values are imported from
`kegari_types::run`, while receipt parsing remains backend policy.

The backend depends directly on `kegari-context` for workflow documents,
instruction assembly, embedded assets and sealed instruction-resume compatibility.
Context depends on config/digest/types without a backend, process or Codex
dependency. Backend callers supply the existing application build identity;
configuration workflow selection, protected artifact handling and execution
admission remain backend-owned. `ArtifactProof` is descriptive shared data in
`kegari_types::artifact`; conversion to a backend evidence reference uses
`kegari_v0::execution::artifact_reference`.

The backend imports synchronous scheduling and attempt-accounting interfaces from
`kegari-scheduler`, whose only production workspace dependency is `kegari-types`.
Configuration-derived limits, admission callbacks, permits, receipts and runtime
orchestration remain in the backend. The scheduler performs no I/O or process
management; CLI and controller need no direct dependency. Its existing `Attempts`
helper does not replace the backend's receipt-based attempt accounting.

The backend imports Git records and deterministic rules directly from
`kegari-git`, whose production workspace dependencies are `kegari-types` and
`kegari-digest`. This first pass retains actual Git operations, journal admission,
operation requests and privately constructed proofs in
`kegari_v0::git`. The three existing pure validation helpers are public for those
retained callers; no compatibility reexports or unchecked proof constructors are
added. CLI and controller need no direct Git dependency. The extraction does not
complete the Git adapter or change its pinned executable version.

All packages currently use workspace version `0.2.0-alpha.10`; the executable
name and process roles are unchanged. Migration-specific persisted and instruction
identities are recorded in PROGRESS.md; older extraction reports describe their
original schemas. The original alpha.10 archive stays intact. The next coherent Codex-backed
candidate is reserved as `0.2.0-alpha.11`; do not bump versions for individual
edits or checks.

The custom repository executor in `kegari-execution` and backend adapters has
been removed. Do not restore its environment construction, per-command journal
factories or native qualification sequence. The new path uses
Codex app-server's supported repository tools, sandbox, permissions and lifecycle
observations; selected-version/capability checks replace the old blanket refusal.

Kegari retains operation/session admission and workflow result acceptance.
Preserve shared process/journal code with actual Codex, Git and control callers,
including the single spawn/retirement interlock, actual child ownership and
conservative recovery. Shared lease data remains in `kegari_types::control`.
Do not reconstruct the deleted executor around the Codex event stream.

Runtime-loaded instruction assets change with their tool/caller migration and
explicit asset identities. Old evidence retains its original meaning; an
unsupported retained record must refuse safely rather than silently becoming new
Codex execution evidence. See the [instruction guide](../crates/kegari-context/instructions/README.md).

## Build and verification

The commands below assume the pinned toolchain and dependencies are already
cached. For a first build with dependency downloads, follow the root README.

Select the affected package and an existing focused test. For example, changes
to the Codex protocol crate can use these commands when appropriate; replace the
package/filter for other changes rather than running every example by default:

```sh
cargo check --offline --locked -p kegari-codex
cargo test --offline --locked -p kegari-codex --test protocol
```

For the migration's first production flow, use one simulated Codex-transport
integration case and one missing/stale-result refusal. Installed Codex and live
account tests remain separately authorized. Fixtures do not establish native or
live compatibility.

Build and inspect the application at a coherent handoff, when time permits:

```sh
cargo build --release --offline --locked -p kegari-cli --bin kegari-v0
```

Use the existing minimal-environment, state-free help/version/build/handover
inspections for that binary. A build failure is a reported blocker, not a reason
to expand testing into a new project.

The following workspace-wide checks are a **later stabilization reference**, not
the default after each feature. `--workspace` includes all seventeen packages:

```sh
cargo test --release --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

See [current progress](../crates/kegari-v0/PROGRESS.md) and
[version milestones](../crates/kegari-v0/MILESTONES.md). The previous goal was
cancelled; the replacement run has its own fixed timestamps in PROGRESS.md.
Native execution integration and operational qualification remain incomplete.
