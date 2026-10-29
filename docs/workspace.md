# Workspace overview

Kegari is a Cargo workspace of **17 crates**, sharing version
**`0.2.0-alpha.10`**. The table describes the current source layout. The approved
[SPEC 1.5](../crates/kegari-v0/SPEC.md) changes the target architecture to
**Kegari orchestrating Codex-owned repository execution**; the source migration is active. The ordinary admitted child path uses native
Codex results and the custom repository runner has been removed. Remaining
features and live qualification are tracked in PROGRESS; historical extraction
verification is not SPEC 1.5 qualification.
Each crate links to its detailed README.

## Approved direction

Codex app-server owns repository inspection, editing, command execution, sandbox
enforcement and supported terminal management. Kegari supplies task context and
role-scoped permissions, schedules and admits work, observes results, and controls
provider mutations, reviews, acceptance and releases. Deterministic Git assembly
and Kegari's own process lifecycle remain host responsibilities.

The migration must **delete** replaced repository execution tools, sandbox
rendering, environment/resource inventories, per-command bookkeeping, obsolete
configuration and tests, and unused dependencies. The old harness must not remain
behind flags, wrappers or fallback paths. Shared process/journal mechanisms still
used by Codex launch, Git and control operations remain until their actual users
no longer need them. Further crate extraction is not a prerequisite.

The existing process arrangement is retained: the control host runs the runtime
driver; the orchestration child owns the runtime lock and exchanges lease,
heartbeat and poll messages. Exclusive runtime ownership and lease-based
admission fencing remain required. Crate names do not require moving the driver
between processes during this migration.

The [six-hour goal](../crates/kegari-v0/GOAL.md) is **active**, with fixed UTC
timestamps in PROGRESS. Follow the
[milestones](../crates/kegari-v0/MILESTONES.md) and
[current progress](../crates/kegari-v0/PROGRESS.md); implementation, focused
validation and full qualification are separate states. Feature work uses minimum
sufficient validation; comprehensive testing follows in the stabilization phase.

## Crates

| Area | Crate | Responsibility |
| --- | --- | --- |
| Application/control | [`kegari-cli`](../crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| Application/control | [`kegari-controller`](../crates/kegari-controller/README.md) | Control-host composition, dispatch, supervision and orchestration channel. |
| Application/control | [`kegari-control-api`](../crates/kegari-control-api/README.md) | Authenticated transport, request rules, operator identities and snapshot operations. |
| Application/control | [`kegari-v0`](../crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |
| Workflow/execution | [`kegari-context`](../crates/kegari-context/README.md) | Workflow documents, instruction assembly, embedded assets and resume compatibility. |
| Workflow/execution | [`kegari-scheduler`](../crates/kegari-scheduler/README.md) | Synchronous scheduling, capacity allocation, attempt accounting and retry timing. |
| Migration/host mechanics | [`kegari-execution`](../crates/kegari-execution/README.md) | Filesystem role/scope data and shared host process-journal mechanics used by Codex/Git/control; the custom repository runner has been removed. |
| Integrations | [`kegari-codex`](../crates/kegari-codex/README.md) | Codex protocol client, session recovery, observations, stream transport and pinned schemas. |
| Integrations | [`kegari-providers`](../crates/kegari-providers/README.md) | Typed GitHub/Linear network adapters and records. |
| Integrations | [`kegari-git`](../crates/kegari-git/README.md) | Git records, naming rules, validation and canonical digest calculations. |
| Foundations | [`kegari-types`](../crates/kegari-types/README.md) | Shared command, response, error, snapshot, run, lease and artifact data types. |
| Foundations | [`kegari-config`](../crates/kegari-config/README.md) | Configuration models, TOML parsing, validation and path resolution. |
| Foundations | [`kegari-digest`](../crates/kegari-digest/README.md) | Shared byte hashing and canonical JSON identities. |
| Foundations | [`kegari-storage`](../crates/kegari-storage/README.md) | Protected filesystem access, durable records and lock primitives. |
| Foundations | [`kegari-process`](../crates/kegari-process/README.md) | Guarded process spawning, owned process groups and exit evidence. |
| Foundations | [`kegari-platform`](../crates/kegari-platform/README.md) | OS identity, executable inspection and native host helpers. |
| Foundations | [`kegari-secrets`](../crates/kegari-secrets/README.md) | Secret values, Keychain storage and generic signed-record primitives. |

## Current implementation boundaries

- **Executables and processes:** `kegari-cli` builds the main `kegari-v0`
  executable; the backend package builds `kegari-v0-evaluation`. Crates organize
  code and dependencies, not separate operating-system processes.
- **Remaining backend:** CLI and controller temporarily depend on `kegari-v0`,
  which retains application admission, configuration transactions, runtime
  integration and delivery policy. The backend depends on control API, but not
  on CLI or controller.
- **Protocol and native integration:** Codex protocol/session handling belongs
  to `kegari-codex`; native launch and protected recovery remain in the backend.
  `kegari-git` owns records and deterministic rules; actual Git operations remain
  backend-owned.
- **Execution and authority:** Codex app-server is the approved repository
  execution layer. The custom repository command/resource implementation and its adapters have
  been removed; source-result and lifecycle integration remains in progress. Application permission to admit
  sessions and accept results stays in Kegari, as do host process mechanisms with
  actual Codex/Git/control callers. This source inventory is not a recipe to retain
  or extend the superseded executor.

See the [developer guide](development.md) for detailed package boundaries and
build instructions, or the [CLI reference](cli.md) for application commands.
