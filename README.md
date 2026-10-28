# Kegari Rust workspace

| Package | Responsibility |
| --- | --- |
| [`kegari-types`](crates/kegari-types/README.md) | Shared command, response, error and snapshot data types. |
| [`kegari-config`](crates/kegari-config/README.md) | Configuration models, TOML parsing, validation and path resolution. |
| [`kegari-digest`](crates/kegari-digest/README.md) | Shared byte hashing and canonical JSON identities. |
| [`kegari-providers`](crates/kegari-providers/README.md) | Typed GitHub/Linear network adapters and records. |
| [`kegari-storage`](crates/kegari-storage/README.md) | Protected filesystem access, durable records and lock primitives. |
| [`kegari-process`](crates/kegari-process/README.md) | Guarded process spawning, owned process groups and exit evidence. |
| [`kegari-cli`](crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| [`kegari-v0`](crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |

Both CLI and backend depend directly on `kegari-config` and `kegari-types`;
`kegari-config` depends on `kegari-types`. The CLI also temporarily depends on the
backend; the backend does not depend on the CLI. Shared types and configuration
models are imported directly from their owning crates, without backend
compatibility reexports. Configuration transactions, workflow integration,
command validation, authorization, exit-code mapping and
snapshot operations remain in the backend. Shared hashing belongs to `kegari-digest`.
Provider adapters depend on config/types/digest; the backend retains the mutation
broker, receipts and evidence admission. Neither new crate depends on the backend.
CLI and backend also depend directly on `kegari-storage`, which depends on
`kegari-types` and `kegari-digest`. Domain persistence policy remains in the backend.
The backend also depends directly on `kegari-process`, which owns the single
spawn/retirement interlock and owned process-group handling. It depends on
storage/types without a backend dependency; the CLI has no direct process
dependency. Process journals, authorization and application supervision remain
in the backend.
All packages use workspace version `0.2.0-alpha.10`; the executable name, process
roles and serialized schemas remain unchanged. This extraction creates no new
prerelease or qualification claim.

From the workspace root, using the pinned toolchain; `--workspace` checks all eight
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
