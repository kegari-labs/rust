# Kegari scheduler

`kegari-scheduler` owns synchronous queue selection, capacity allocation, attempt
accounting and retry timing. The package remains at workspace version
`0.2.0-alpha.10`; serialized formats and application behavior are unchanged.

Import the existing interfaces directly from the crate root:

- `Scheduler`, `Limits`, `ReadyWork`, `Allocation`, `Quota`, `Trigger`, `WorkKind`.
- `Attempts`, `AttemptRecord`, `retry_at`.

The old `kegari_v0::scheduler` module is removed without compatibility reexports.
`PhaseKey` remains in `kegari_types::run`. The backend's distinct private release
attempt record is unchanged. The scheduler's existing `Attempts` helper is
preserved without introducing new production use or replacing receipt accounting.

The scheduler owns no runtime, process, storage, configuration or network layer.
Its operations use caller-supplied time, identities and admission callbacks;
production code reads no clock and generates no UUIDs. Queue refresh, capacity,
workspace fairness, priority, quota handling, attempt bounds and retry behavior
retain their existing implementation and errors.

The host callback passed to `admit_next` must revalidate current facts, reserve its
permit and persist the required run receipt before returning an operation ID.
An allocation does not establish durable admission or native process ownership.
Configuration-to-limit conversion, actual admission and recovery, the runtime
driver's locks and scheduler replacement/restoration sequence remain backend-owned.

The crate's only production workspace dependency is `kegari-types`, alongside
pinned serde, chrono and UUID serialization/data support. It has no backend
dependency, including in tests. Test-only clock/UUID generation and `kegari-digest`
preserve the original fixtures. CLI and controller need no direct dependency.
Application identity remains backend-owned; its hash and rebuild tracking cover
this crate's manifest and source tree.

From the workspace root with the pinned toolchain/cache:

```sh
cargo test --release --offline --locked -p kegari-scheduler
cargo test --release --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

See the [extraction report](../kegari-v0/validation/SCHEDULER-EXTRACTION.md) for
compatibility evidence and verification. This refactor adds no scheduling policy,
operational qualification or release package.
