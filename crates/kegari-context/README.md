# Kegari context

`kegari-context` owns workflow documents, bounded instruction assembly,
repository guidance, embedded assets and instruction-resume compatibility.
The package remains at `0.2.0-alpha.10`; formats, instruction assets and the
pinned Codex version are unchanged.

Import document interfaces through `kegari_context::workflow` and assembly
interfaces through `kegari_context::instructions`. The private resume module
retains sealed assembly and compatibility evidence. There are no backend
compatibility reexports or unchecked constructors for assembled instructions
or verified resume instructions.

The crate supplies context for an agent; execution and admission authority remain
backend-owned. It does not run checks, hooks, services, providers or workers.
Workflow check/service declarations and permission manifests describe inputs;
they do not grant execution or broaden an existing native boundary.

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

From the workspace root with the pinned toolchain and dependency cache:

```sh
cargo test --offline --locked -p kegari-context --release
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use synthetic data and disposable fixtures. Passing checks do not establish
native/live qualification. This extraction adds no configuration requirement,
workflow redesign, toolset support or release packaging.
