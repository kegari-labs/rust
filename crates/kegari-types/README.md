# Kegari shared types

This package owns the shared serialized data used by the CLI and backend:

- `api`: `CommandName`, `CommandRequest` and `Envelope`.
- `artifact`: descriptive `ArtifactProof` records with their existing serialized format.
- `error`: `Error`, `Result`, error constructors and safe standard trait implementations.
- `snapshot`: `Segment`, `Node` and `Page`.
- `run`: `RunReference` and `PhaseKey`, preserving their existing serialized formats.

Consumers import these types directly from `kegari_types`; the backend does not
reexport them. CLI, controller, control API and backend depend directly on this package. The CLI
also retains its temporary backend dependency. Codex protocol state uses the shared
run data directly.

These data types do not grant authority or perform provider, filesystem or
process operations. Command validation/classification and snapshot capture/reassembly belong to
`kegari-control-api`. Current-operator authorization and exit-code mapping remain
backend free functions. Run receipt parsing and validation also remain backend
policy: replace `kegari_v0::control::RunReference` and
`kegari_v0::scheduler::PhaseKey` imports with `kegari_types::run::{RunReference,
PhaseKey}`, and replace `RunReference::from_receipt` with
`kegari_v0::control::run_reference_from_receipt`. Deserializing run data does not
validate admission or establish receipt lineage.
Artifact descriptions are imported from `kegari_types::artifact::ArtifactProof`
instead of `kegari_v0::execution::ArtifactProof`. The former `reference()` method
is now backend `execution::artifact_reference(&ArtifactProof)`; protected reads,
artifact capture, archive verification and evidence admission stay backend-owned.

Moving the types does not change their serialized schemas or turn deserialized
data into verified execution or delivery evidence.

The workspace version remains `0.2.0-alpha.10`, and the executable remains
`kegari-v0`. Build it through `kegari-cli` and check all packages from the workspace
root:

```sh
cargo build --offline --locked -p kegari-cli --bin kegari-v0
cargo test --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

See [current progress](../kegari-v0/PROGRESS.md) for validation results and limits.
The archived alpha.10 artifact and its evidence predate this extraction; the
package split does not establish new native or operational qualification.
