# Kegari execution environments

This first extraction pass owns managed-environment construction, isolation
validation, immutable-resource binding, sandbox rendering and descriptive command
and environment observations. Import these interfaces from `kegari_execution`.
The crate does not launch commands or confer execution authority.

`IsolationPlan`, `ValidatedIsolation`, `BoundManagedExecution`, `ExecutionRole`
and `PathGrant` retain their existing semantics. Resource inventories and cwd
anchors remain private. Validated isolation cannot be deserialized or constructed
without the existing filesystem checks. Serialized plans remain schema 1;
managed domains remain schema 5 and observations remain schema 1.

## Application identity

`IsolationPlan::validate`, `BoundManagedExecution::automatic`,
`automatic_policy_digest` and `revalidate` now accept `runtime_build: &str`.
The resource-version and resource-file validation methods also take this final
argument. Production callers supply the backend's current `BUILD_ID`, never an
identity read from a retained record. `ValidatedIsolation` keeps the supplied
identity privately through repeated validation and cwd derivation. It is not a
new serialized field, configuration option or independently generated build ID.

Equal inputs, including the build identity and filesystem observations, retain
the existing serialized bytes and digests. Changing the application build still
invalidates retained managed domains as before.

## Native integration

`render_observed_command` returns an immutable `RenderedCommand` and its
`CommandObservation`, using the same rendering and observation algorithms as
ordinary inspection. Read-only accessors expose the executable, argv, environment
and sandbox profile. Neither result grants admission or launch authority.

Native commands, outputs, limits, cancellation, journals and joined cleanup live
in `kegari_v0::execution_host`. The former backend `isolation` module is removed
without compatibility reexports. Application checks and evidence remain in
`kegari_v0::execution`; native Git/Codex adapters retain their existing owners.
The backend's `codex_host::IsolationSessionScope` borrows a validated isolation
for Codex protocol validation without adding validation during construction.

The existing unqualified-command refusals remain in force. This crate adds no
toolset catalog, environment manifest requirement or operational qualification.

## Dependencies and checks

Production workspace dependencies are types, digest and platform. External
dependencies retain pinned serde/JSON, libc, SHA-256 and hex implementations;
tests use tempfile. There is no direct backend, configuration, storage, process,
provider, Codex or async-runtime dependency. Platform's existing transitive
dependencies are unchanged.

Run from the workspace root with the pinned toolchain:

```sh
cargo test -p kegari-execution --release --offline --locked
cargo test --workspace --release --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
```

See the [extraction report](../kegari-v0/validation/EXECUTION-ENVIRONMENT-EXTRACTION.md)
for compatibility evidence and the retained native-execution boundary.
