# Kegari configuration

This package owns configuration models at its crate root, including `Config`,
`Workspace`, connection and repository policy types. It provides TOML parsing,
explicit file loading, structural validation and path resolution. It depends on
`kegari-types` for shared errors; both the CLI and backend depend on this package.

Parsing and validation do not verify provider identities, grant execution
authority or perform configuration transactions. Missing delivery policy remains
representable for discovery, while accessors for required downstream policy
continue to refuse unresolved declarations. The backend owns provider preflight,
credential handling, durable configuration updates and workflow integration.
Configuration revision calculation remains in the backend and uses the shared
canonical hashing implementation from `kegari-digest`.

Configuration schema `1` and workspace version `0.2.0-alpha.10` are unchanged.
This package extraction changes Rust ownership, not the serialized configuration
contract, installed executable name or historical alpha.10 evidence. It creates
no new prerelease or operational qualification claim.

## Rust consumer migration

Import models directly from the crate root, for example
`use kegari_config::{Config, Workspace};`. There are no compatibility type
reexports from `kegari_v0::config`.

| Previous call | Backend integration call |
| --- | --- |
| `config.revision()` | `kegari_v0::config::config_revision(&config)` |
| `workspace.load_workflow()` | `kegari_v0::config::load_workflow(&workspace)` |
| `config.load_workflows()` | `kegari_v0::config::load_workflows(&config)` |

`Config::parse`, `Config::load`, `Config::validate` and `Config::resolve_paths`
remain methods on the configuration model. `ConfigReload` and `inspect_reload`
remain in `kegari_v0::config`; configuration transactions remain in
`kegari_v0::configuration`.

## Workspace commands

The application executable remains owned by `kegari-cli`. From the workspace root:

Workspace-wide test/lint commands below are later stabilization references.
The current [goal](../kegari-v0/GOAL.md) requires only affected compilation and
minimal focused validation while features are being implemented.

```sh
cargo build --offline --locked -p kegari-cli --bin kegari-v0
cargo run --offline --locked -p kegari-cli -- --help
cargo test --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Workspace-wide checks cover all current packages. See
[current progress](../kegari-v0/PROGRESS.md) for actual validation results and
remaining limits; these commands are instructions, not evidence of a completed run.
