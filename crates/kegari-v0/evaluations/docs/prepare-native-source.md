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

## Explicit E07 source-data migration (runtime support pending)

The default invocation and E05 input requirements above are unchanged. The closed
`--recipe e07-pr-attack-v1` branch is a separate, explicit **data migration**. It
accepts the exact shipped legacy `pr_attack` fixture and captured reviewed
`native/pr_attack-v1.json` recipe, rather than treating that fixture as already
native-ready. Unknown recipe names, changed recipe bytes, already migrated E07
input, or an occupied policy/binding destination are refused. This branch does
not select an account, materialize a canary, import E05 objects, or activate a
runtime.

```sh
python3 -B evaluations/tools/prepare_native_source.py \
  --suite /absolute/canonical/captured-suite-with-reviewed-e07-recipe \
  --output /absolute/canonical/user-owned-temp-parent/new-e07-source \
  --recipe e07-pr-attack-v1
```

All existing input/output ownership, ancestry, no-follow, capture, identity,
Git-command and no-overwrite restrictions still apply. The recipe creates a fresh
local Git source from E07's own files, replacing only its scratch-bound workflow
with the declared independent workflow without local checks. ROOT_V1/NESTED_V1,
scripts/tests and file modes remain. Its base has `value < limit`; the one-parent
head restores the correct `value <= limit`. Both commits have the actual human
signoff and assistance trailer, using the disclosed fixed timestamp. Their full
OIDs are obtained from those actual new commits. No source programs run.

Only after Git has produced the actual base does the tool materialize a copied
suite with both base pointers filled. It adds the exact schema-3 ten-slot context
binding and policy declaration, puts the literal adversarial text in PR body,
and uses a placeholder **only for the future declared host scalar**
`/evidence/forbidden_canary_path`. It does not substitute an arbitrary pathname or
canary value. The old `../scorer` target is replaced explicitly by wording naming
that operand; no symlink, sibling file or permission change is created.

The copied suite uses revision `4-e07-native-path-v1`. Only E07's phase,
initial_state, context/binding/workflow, optional policy reference and derived
fixture identity change, plus the declared policy file and suite identity. The
other 27 variant objects/assets, every scorer byte/digest, profile bindings or
unresolved text, limits, requirements, outcomes, rubrics, prohibited effects,
artifacts and all 28 variants / 56 trial slots are preserved. The tool verifies
the complete expected output file bytes/modes and an independent manifest field
allowlist, not just those counts. Existing file modes remain exact; owned output
directories stay 0700 as in E05. New declaration files use mode 0600.

**This output is not supported for live execution yet.** Current runtime support
for schema 3 and the actual owner-bound canary operand before contract assembly
is still missing. `SOURCE-EVIDENCE.json` records `runtime_supported: false`,
`dispatch_authorized: false`, `canary_materialized: false` and the precise missing
prerequisites. These disclosures are evidence fields, not undeclared suite or
context fields. Current loaders must refuse unsupported schemas/fields; do not
edit them out to force a run. Scorer `kegari-observable-v3` and all its assertions
remain unchanged, so this preparation does not qualify E07 or manufacture its
missing host state facts.

The evidence includes exact input/recipe/context-binding/policy hashes, original
and derived suite identities, commands/results, OIDs/trees, modes, allowed file
and manifest changes, and the unsupported-runtime limit. On failure, partial
owned output remains inspectable. No output context/manifest exists before a
real base commit is known; do not copy a null or guessed base from recipe data
into a loadable fixture. A later runtime implementation and explicit configured
suite/source/subject binding remain separate prerequisites.

### E07 component checks

These authored tests need an explicit actual suite containing the reviewed
recipe and unchanged legacy E07 fixture. Run with `-B` so input capture is not
changed by import bytecode. The positive test uses the invoking human's Git
identity and real disposable local commits; it never sets a fixed human identity
or uses fabricated positive OIDs. Other tests exercise data/refusal paths and an
explicit first-commit failure. They are local components, not native trials.

```sh
KEGARI_TEST_E07_SUITE=/absolute/canonical/captured-suite-with-reviewed-e07-recipe \
  python3 -B -m unittest discover -s evaluations/tools -p test_prepare_native_source.py -v
```

Set `KEGARI_TEST_NATIVE_SUITE` as documented above as well to run both positive
recipe groups. Missing explicit inputs skip those groups; a skipped group is not
a pass. Current E05 tests and assertions remain unchanged. Actual results belong
to the root task's later recorded validation, not to this unexecuted draft.
