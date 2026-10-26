# Prepare the first E05 native source fixture

This tool prepares **local data only** for `independent_malicious`. It does not run the evaluator, select an account/model, activate a runtime, execute a fixture program, or qualify a native run.

The input must already be a captured native-ready suite: 28 variants, 56 declared trial slots, E05 phase `independent_review`, and the schema-v2 context binding with the exact nine native scalar slots. Legacy templates are refused. The tool copies the full bounded suite and preserves profile bindings and unresolved-profile text exactly, whether resolved or unresolved. Later production `SuiteManifest`/`Plan`, source admission and all native gates still apply.

Run from the intended repository directory whose current human Git `user.name` and `user.email` should sign the two disposable fixture commits. The tool reads those values; it never sets global/local human identity configuration. Use canonical absolute paths, including `/private/tmp` rather than a symlink spelling such as `/tmp`.

```sh
python3 -B evaluations/tools/prepare_native_source.py \
  --suite /absolute/canonical/native-ready-suite \
  --output /absolute/canonical/user-owned-temp-parent/new-e05-source
```

The output parent must already exist, be canonical, belong to the current user, and have no group/world write bits. Its ancestry must also be protected: directories are owned by root or the current user, and writable ancestors must be sticky with a current-user-owned next child (for example, a user-owned 0700 temporary directory within root-owned sticky `/private/tmp`). `/private/tmp` itself is normally root-owned and is not an acceptable direct parent; use an existing owned temporary directory within it. The output path must not exist and must be disjoint from the suite. Input capture rejects symlinks, hardlinks, special files, Git metadata/configuration paths, foreign ownership and noncanonical paths. Capture is bounded to 4,096 entries, 64 MiB and depth 32. Descriptor-backed directory iteration reserves the shared entry budget before descent and sorts only a bounded collection, in both capture and metadata verification passes. Files are read through no-follow/nonblocking descriptors with identity/metadata checks before and after reads and a final whole-tree metadata recheck. Captured bytes are the source of the copy; input is never rewritten. Output root and initial child directories are created relative to retained parent/root descriptors. Output directories are owned and mode 0700, captured file modes are preserved. Retained ancestry/root identities are rechecked around path-based copy/capture/derived-rewrite/Git phases; final evidence writes use the retained output descriptor. This does not claim resistance to an arbitrary concurrent process running as the same user. A failure after output creation leaves partial local data for inspection; rerun only with another absent output path.

The output contains:

- `suite/`: captured suite; only E05's two base-SHA context pointers and its derived manifest fixture digest are rewritten.
- `source/`: disposable Git repository with base `value <= limit`, then head `value < limit`. Both commits use `--signoff`, Conventional Commit subjects and `Assisted-by: Codex`.
- `empty-git-template/` and `empty-git-hooks/`: deliberately empty Git inputs.
- `SOURCE-EVIDENCE.json`: exact command arguments, working directories, exit results, object/tree IDs, captured file digests/modes, suite identity, fixed timestamp and qualification limits.

The fixed commit time `2026-10-26T00:00:00+00:00` is disclosed fixture metadata, not an observation timestamp or freshness claim. Identity and captured source still affect object IDs. Inherited `GIT_*` variables are removed. Source commands additionally disable system/global Git configuration, hooks, signing and transports. There are no remote commands, shell commands, aliases, fixture scripts or validation commands. In particular, malicious `scripts/payload.py` remains inert data.

The emitted suite identity is derived from the copied manifest. This tool does not update runtime configuration to trust that identity. Any explicit runtime binding remains a separate operator/production action with the usual admission checks.

## Local component checks

The tests are separate from native qualification. The actual-Git tests require an explicit native-ready suite and the invoking human's configured identity; they create only disposable local repositories. They check exact two-commit content, signatures' signoff text, deterministic IDs, mode/profile/slot preservation and refusal boundaries. No cryptographic commit signature is claimed.

```sh
KEGARI_TEST_NATIVE_SUITE=/absolute/canonical/native-ready-suite \
  python3 -B -m unittest discover -s evaluations/tools -p test_prepare_native_source.py -v
```

See [local validation evidence](../../validation/LOCAL-NATIVE-EVALUATION.md) for recorded executions and their exact source identities. Running the component checks does not qualify live model execution, native confinement, actual process exit, scoring or human acceptance.
