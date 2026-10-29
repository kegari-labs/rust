# Kegari context

`kegari-context` owns workflow documents, bounded instruction assembly,
repository guidance, embedded assets and instruction-resume compatibility.
The package remains at `0.2.0-alpha.10`; instruction assets now use identity `9`
for the Codex-owned repository execution contract; the installed generic workflow
has identity `2`. The pinned Codex version is
unchanged; historical asset identities are not reinterpreted.

Import document interfaces through `kegari_context::workflow` and assembly
interfaces through `kegari_context::instructions`. The private resume module
retains sealed assembly and compatibility evidence. There are no backend
compatibility reexports or unchecked constructors for assembled instructions
or verified resume instructions.

The crate supplies context for an agent; Kegari retains workflow admission and
result acceptance. Under [SPEC 1.5](../kegari-v0/SPEC.md), Codex app-server owns
repository tools, execution, sandbox enforcement and supported terminal lifecycle.
This crate does not run checks, hooks, services, providers or workers. Descriptive
workflow inputs and permission manifests grant no additional authority.

The embedded assets use native Codex repository tools and describe Kegari
`validation_record` as association of actual observed results, never command
launch. They no longer instruct agents to use custom command/edit or resource
inventory tools. These changes accompany the production caller migration; they
do not establish native/live qualification. The
[instruction guide](instructions/README.md) states the boundary.

`AssemblyInput::runtime_build` carries the existing host application identity.
Production callers supply `kegari_v0::BUILD_ID`; the assembler copies it at the
existing manifest-construction point. There is no separate context build script
or build identity. The workspace-inherited package version still binds the
embedded asset catalog. Identical supplied build identity and filesystem inputs
preserve exact prompt/manifest bytes and digests; a new application build
legitimately produces a different manifest identity.

Encoded review artifacts use the shared descriptive
`kegari_types::artifact::ArtifactProof`. Context validation checks their shape,
length, digests, explicit UTF-8/base64 encoding and decoded secret bytes before
assembly. Protected capture, archive verification, reads and evidence admission
remain in the backend. Backend `execution::artifact_reference(&ArtifactProof)`
replaces the old record method; `ArtifactReference` remains backend-owned.

The complete [instruction catalog](instructions/) and optional
[starter workflow](examples/WORKFLOW.md) are embedded. Installed-default
selection still verifies the catalog; explicit missing or invalid workflows
never fall back. Starter export retains create-new behavior. The backend keeps
configuration selection and required-check cross-validation, and its separate
evaluation-binding example.

Production dependencies are `kegari-config`, `kegari-digest`, `kegari-types` and
pinned serialization/encoding libraries. Tests use temporary fixtures and UUID
generation; there is no backend development dependency or process, runtime,
platform, storage, provider or Codex dependency.

Use the smallest relevant focused check under the [development policy](../../docs/development.md#feature-first-development-policy).
These broader commands are a later stabilization reference, not a per-feature gate:

```sh
cargo test --offline --locked -p kegari-context --release
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use synthetic data and disposable fixtures. Passing checks do not establish
native/live qualification. This extraction adds no configuration requirement,
workflow redesign, toolset support or release packaging.
