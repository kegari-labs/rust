# Workspace overview

Kegari is a Cargo workspace of **17 crates**, sharing version
**`0.2.0-alpha.10`**. The table describes the current source layout, including
ongoing extraction work; it does not imply that every extraction is complete or
that the application is operationally qualified. Each crate links to its detailed
README.

## Crates

| Area | Crate | Responsibility |
| --- | --- | --- |
| Application/control | [`kegari-cli`](../crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| Application/control | [`kegari-controller`](../crates/kegari-controller/README.md) | Control-host composition, dispatch, supervision and orchestration channel. |
| Application/control | [`kegari-control-api`](../crates/kegari-control-api/README.md) | Authenticated transport, request rules, operator identities and snapshot operations. |
| Application/control | [`kegari-v0`](../crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |
| Workflow/execution | [`kegari-context`](../crates/kegari-context/README.md) | Workflow documents, instruction assembly, embedded assets and resume compatibility. |
| Workflow/execution | [`kegari-scheduler`](../crates/kegari-scheduler/README.md) | Synchronous scheduling, capacity allocation, attempt accounting and retry timing. |
| Workflow/execution | [`kegari-execution`](../crates/kegari-execution/README.md) | Managed environments, isolation, native command lifecycle and process-journal mechanics. |
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

## Key boundaries

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
- **Execution and authority:** `kegari-execution` handles managed environments
  and command/process-journal mechanics. Application permission to execute work
  and acceptance of its results remain backend decisions.

See the [developer guide](development.md) for detailed package boundaries and
build instructions, or the [CLI reference](cli.md) for application commands.
