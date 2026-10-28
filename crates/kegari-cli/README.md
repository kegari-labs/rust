# Kegari CLI

This package owns the `kegari-v0` executable and the `cli`, `client` and
`local_cli` library modules. Rust consumers use `kegari_cli` for these modules.
Command arguments, output, local setup, service actions and hidden process
entrypoints retain their existing behaviour.

The CLI imports shared command/response, error and snapshot data directly from
[`kegari-types`](../kegari-types/README.md), and configuration models directly from
[`kegari-config`](../kegari-config/README.md). Its temporary dependency on
`kegari-v0` remains for configuration transactions and workflow integration,
command validation and classification, role and exit-code decisions, snapshot
operations, credential transactions, registration, services, build metadata and the runtime. The
backend imports the same shared types and configuration models without
reexporting them. This is not yet a lightweight standalone client.

Protected filesystem access and target-store storage primitives are imported
directly from `kegari-storage`. OS account lookup, host identities, executable and
process identities, and the browser-launch adapter are imported directly from
[`kegari-platform`](../kegari-platform/README.md). URL validation remains in the CLI;
account-home lookup remains separate from native host/boot discovery.

Secret input and control-client values are imported directly from
[`kegari-secrets`](../kegari-secrets/README.md). The backend retains authorization
of credential imports and provider conversion; the secrets crate has no provider
or backend dependency. Importing `Secret` does not access Keychain or add a
Keychain prerequisite to remote-only client use.

CLI hash fixtures use `kegari-digest` as a development dependency; the CLI has no
direct provider-adapter or process dependency. `kegari-platform` uses the shared
`kegari-process` spawn boundary. Registration, service lifecycle and installed-build
verification continue through the backend platform module.

All packages share workspace version `0.2.0-alpha.10`. CLI version output and
internal metadata identify the same assembled application; its build identity
must cover the application sources in all ten packages.

From the workspace root:

```sh
cargo build --offline --locked --release -p kegari-cli --bin kegari-v0
cargo run --offline --locked -p kegari-cli -- --help
cargo test --offline --locked -p kegari-cli
```

Use `--workspace` for complete test and lint checks across all ten packages. The
evaluation-inspection binary remains in the backend package. See the [command guide](../kegari-v0/docs/cli.md),
[backend documentation](../kegari-v0/README.md) and [progress](../kegari-v0/PROGRESS.md).
The archived alpha.10 package is unchanged by this source refactor; no new
prerelease, native qualification or service activation is implied.
