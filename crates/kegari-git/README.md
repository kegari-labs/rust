# Kegari Git records and rules

`kegari-git` owns Git records, naming rules, validation and canonical digest
calculations. This is the first extraction pass; actual Git operations and their
authority remain in `kegari_v0::git`. The package retains workspace version
`0.2.0-alpha.10`, the pinned Git version, serialized formats and digest semantics.

Import moved interfaces directly from the crate root:

| Area | Interfaces |
| --- | --- |
| Constants and repository identity | `SUPPORTED_GIT_VERSION`, `DELTA_SCHEMA`, `RepositoryId`, `Revision`, `validate_oid` |
| Naming and scope | `BranchName`, `title_suffix`, `WriteScope`, `validate_repo_path` |
| Delta records | `DeltaManifest`, `ReplacementMapping`, `ReviewDelta`, `DeltaArtifact` |
| Candidate and integration records | `CandidateManifest`, `ProductionIntegration` |
| Source metadata | `CommitIdentity`, `SourceCommitMetadata`, `SourceTreeEntry`, `GitCommitIdentity` |

These fourteen types have one implementation in this crate. Replace their old
`kegari_v0::git` imports with `kegari_git`; the backend supplies no compatibility
reexports. Git-specific records remain here rather than in `kegari-types`.

The existing `validate_repo_path`, `CommitIdentity::validate` and
`SourceCommitMetadata::validate` checks are public so the retained backend
operations use the same rules. They perform no I/O or authorization. Other
validation order, errors, field visibility and methods are unchanged. In
particular, deserialization and digest calculation gain no implicit validation.
`CommitIdentity` continues accepting unknown fields; the other serialized records
retain their refusal. `DeltaArtifact` remains non-serializable. Review digests
exclude future squash metadata while retaining their existing reviewed inputs;
replacement accounting retains depth bounds and duplicate-contribution refusal.

The backend retains `Git`, execution scopes, limits, start gates, cancellation,
process journals, clone ownership, host-read capabilities and operation requests.
It also retains objects whose private construction establishes actual observations:
`ProposedDelta`, `Candidate`, `CommittedSource`, `GitPublicationObjects`,
`VerifiedProductionIntegration`, `VerifiedIndependentClone`,
`VerifiedReusableSource` and `VerifiedHostReadRoot`. This extraction adds no
unchecked constructors or runner callbacks. Branch construction still invokes
Git ref validation, and delta ordering still performs actual Git verification.

Production workspace dependencies are only `kegari-types` and `kegari-digest`,
alongside pinned serde, chrono and UUID data/serialization support. The crate
performs no filesystem or process operations, reads no clock and generates no
UUIDs. Tests use fixed data and serde_json. There is no backend dependency,
including development dependencies; CLI and controller need no direct dependency.
Application identity remains backend-owned, with this manifest and source tree
included in its hash and rebuild tracking.

From the workspace root with the pinned toolchain/cache:

Workspace-wide test/lint commands below are later stabilization references.
The current [goal](../kegari-v0/GOAL.md) requires only affected compilation and
minimal focused validation while features are being implemented.

```sh
cargo test --release --offline --locked -p kegari-git
cargo test --release --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Existing disposable-Git, delivery and process tests remain backend-owned. See the
[extraction report](../kegari-v0/validation/GIT-RECORDS-EXTRACTION.md) for results,
compatibility evidence and source/build identity, and
[current progress](../kegari-v0/PROGRESS.md) for the current checkpoint. This pass
does not complete the Git adapter extraction, introduce a qualification claim or
package another release. The alpha.10 archive remains unchanged.
