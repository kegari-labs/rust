# Kegari Rust workspace

| Package | Responsibility |
| --- | --- |
| [`kegari-types`](crates/kegari-types/README.md) | Shared command, response, error, snapshot and run data types. |
| [`kegari-config`](crates/kegari-config/README.md) | Configuration models, TOML parsing, validation and path resolution. |
| [`kegari-digest`](crates/kegari-digest/README.md) | Shared byte hashing and canonical JSON identities. |
| [`kegari-providers`](crates/kegari-providers/README.md) | Typed GitHub/Linear network adapters and records. |
| [`kegari-storage`](crates/kegari-storage/README.md) | Protected filesystem access, durable records and lock primitives. |
| [`kegari-process`](crates/kegari-process/README.md) | Guarded process spawning, owned process groups and exit evidence. |
| [`kegari-platform`](crates/kegari-platform/README.md) | OS identity, executable inspection and native host helpers. |
| [`kegari-secrets`](crates/kegari-secrets/README.md) | Secret values, Keychain storage and generic signed-record primitives. |
| [`kegari-control-api`](crates/kegari-control-api/README.md) | Authenticated transport, request rules, operator identities and snapshot operations. |
| [`kegari-codex`](crates/kegari-codex/README.md) | Codex protocol client, session recovery, observations, stream transport and pinned schemas. |
| [`kegari-controller`](crates/kegari-controller/README.md) | Control-host composition, dispatch, supervision and orchestration channel. |
| [`kegari-cli`](crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| [`kegari-v0`](crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |

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
dependency. Process journals, authorization and application supervision remain
in the backend.
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
admission, configuration transactions and the execution/delivery engine remain
backend-owned. Control-host and child process responsibilities are unchanged.
The backend depends directly on `kegari-codex` for protocol/session ownership,
request sequencing, deadlines, protocol recovery and observations. Its production
workspace dependencies are shared types and digest; disposable storage fixtures
are test-only. It has no backend dependency. Native launch and owned cleanup stay
in `kegari_v0::codex_host`; protected retained sessions, receipt lineage and
execution admission remain backend-owned. Shared run values are imported from
`kegari_types::run`, while receipt parsing remains backend policy.
All packages use workspace version `0.2.0-alpha.10`; the executable name, process
roles and serialized schemas remain unchanged. This extraction creates no new
prerelease or qualification claim.

From the workspace root, using the pinned toolchain; `--workspace` checks all thirteen
packages:

```sh
cargo build --offline --locked -p kegari-cli --bin kegari-v0
cargo run --offline --locked -p kegari-cli -- --help
cargo test --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

See [current progress](crates/kegari-v0/PROGRESS.md) and
[version milestones](crates/kegari-v0/MILESTONES.md). Goal mode remains cancelled;
native execution and operational qualification remain incomplete.
