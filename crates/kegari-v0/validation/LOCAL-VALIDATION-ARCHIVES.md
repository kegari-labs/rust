# Local authenticated validation archives

Partial SPEC 1.4 implementation and deterministic evidence for 0.2.0 / M02-4.
Baseline: `93cdbb06638f028ea2a683dc58dae265aa7402ef` on clean `main`.
Package remains development `0.2.0-alpha.9`; instruction assets remain version 6.
No minor milestone or native, actual-agent, provider or human qualification closes.

## Implemented boundary

Original worker history now seals the retained artifact inventory as well as
plans, attempts and errors. Both actual completion bridges preserve those byte
capabilities. Each completed artifact is retained before a later artifact or
acknowledgement can fail. A borrowed reader verifies the original complete seal
once, indexes exact attempt membership and reads only original retained chunks.
Overwritten or deleted scratch paths are never a historical fallback.

The private archive authenticates the workspace/runtime, original and current
bindings, source and candidate vector, ordered full plan/attempt history and
every observed artifact from failed, superseded and selected attempts. Exclusive
content-addressed chunks receive exact readback before the signed manifest is
written; final resolution verifies all streams and their aggregate identities.
Missing, altered, linked, reordered or substituted content refuses. Partial files
remain for reconciliation and are not silently overwritten. An absent declared
artifact remains a recorded failure; no bytes are invented for it.

The archive reader returns historical data, never an enrollment, execution,
worker or readiness capability. Actual delivery still requires original sealed
completion and selected execution capabilities. New discovered review, child,
integrated and parent records use schema 3 with compact archive and selected
attempt references. Consumers resolve the full archive before existing outcome,
work/revision/vector, policy and independent-adequacy checks. Hosted results stay
explicit. Legacy encodings and historical readers remain supported.

Parent origin capture retains the hashes of the exact authenticated manifest and
chunk reads, including failed outputs. Retirement rechecks that original closure.
Historical release readers apply the same compact/full comparison as ordinary
readback. Neither a decoded compact reference nor mechanical completion can
create integrated readiness or bypass the separate adequacy stage.

Actual child and integrated review input contains the full history, host attempt
references and all distinct original artifact bodies. UTF-8 text remains text;
binary or NUL-bearing data uses explicit base64. Assembly verifies the proof,
decoded length and byte digest and scans decoded bytes for supplied known secrets.
The existing credential-field guard and generation-digest projection remain
unchanged. Parent synthesis receives authenticated selected child validation
rows; compact publication cannot erase its prior input.

## Bounds and retained limitations

- Full history: 8 MiB; signed archive manifest: 256 KiB; chunks: 4 MiB.
- Distinct artifact proofs: 4,096; total artifact bytes: 256 MiB.
- Review construction shares a 1 MiB budget across context and encoded artifacts;
  actual admission separately requires the entire instruction prompt to fit 1 MiB.
- Publication retains its 60 KiB body and 256 KiB protected-record limits.

These limits refuse explicitly; no history or output is truncated to fit.
Larger review inputs still require authenticated paging. Binary byte access is
not proof of visual inspection. The existing visual upload/playback producer
remains unavailable; this archive does not implement or qualify it. Cold
discovered continuation/reassembly, broader environment admission, empty/unborn
baselines, native execution and actual-agent conformance remain incomplete.
There is no atomic filesystem-snapshot or arbitrary detached-descendant claim.

## Review and corrections

Independent review found that schema-3 child publication initially missed an
existing pre-publication size guard, and compact child rows initially removed
local-result details from parent synthesis. Both are corrected. Review also
identified that merely retaining artifacts did not supply their content to the
reviewer; bounded original bodies now enter the actual review input. Explicit
decoded secret checks prevent base64 from bypassing the assembly contract.

Storage, original artifact handoff, compact consumers, parent origin/history and
final assembly received independent static reviews. No material finding remains
in those bounded reviews. The existing generation projection already handled
authorization digests; its actual path was tested rather than weakening the
credential guard. No test assertion was removed or warning suppressed.

## Validation and provenance

All Cargo commands use installed stable Rust/Cargo 1.99.1,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked`.
Ordinary tests retain three ignored native cases; no prior approval is reused.

An initial all-target check found one missing import and helper visibility
errors. The corrected focused build passed. One initial direct test filter
matched zero cases; its output is retained and is not counted as coverage.
The corrected filters passed 14 archive-related tests, three actual-worker
artifact-retention regressions and two compact-evidence predicate tests on
unchanged source `sha256:010a4e346c444950308f1b3c8c223e3015ffe430795c1fce9a4d26297b51913d`
(744 inputs). The earlier standalone full-envelope test also passed.

The first strict lint found a redundant eighth argument. Full validation is now
derived from the authenticated archive instead of passing two copies; historical
schema-2 readers are unchanged. Strict all-target Clippy passes on final source
`sha256:3d36ac4c01b1657cf1ffdb7d1c98763ee33c0b3105bf5fa9cf05d75fbdeb97b3`.
Formatting and contract-inventory consistency pass; inventory tests pass 11.
Full regression on that source finished **1,227 passed, one failed, three native
ignored**, across 40 groups. The single failure was a new negative fixture:
`b"original"` has eight bytes, and its supposed wrong-length variant also used
eight. The test now uses the actual length plus one and verifies that every
negative variant changes its input. Its original rejection assertion is unchanged;
no production code changed. All **39 tests** in the corrected target pass, as does
strict lint of that target. Earlier passing targets remain valid and were not
rerun; all **1,228 distinct executed cases** now have passing evidence. The
failed full-run command is retained rather than relabelled as a successful run.

Final source:
`sha256:ca62d5ff3c175ce85b59ed2b87cade9f08652186bdf029e35f2dffbc70d15fb5`.
All 744 input hashes match after the final build. The only difference from the
full-regression source is the corrected test file. The release-profile development
executable built successfully; static inspection after build completion matches
an independent calculation over all 344 build inputs.

- Executable: `target/release/kegari-v0`, development only.
- SHA-256: `236febfaea22c9015fb2c35acc6e33cb80c19301d98de5aa1c6515dffad90c63`.
- Build identity: `sha256:defc76f235316b66e2455f8077ac6baddb2f0af3bb8c845cbb21f27447fcab65`.
- Metadata: handover 4, managed 5, process 3, workflow-source 1, assets 6.
- Archive: `target/validation/m02-archives-ca62d5ff3c17`, 534 files plus manifest
  SHA-256 `6078a4bf2d98d0b16209edc0cb76d93d3c56f197004247434b66b6fb644136bc`.

The archive retains complete source snapshots 2–5 as manifests referencing 506
shared content blobs, all checked against their original hashes. Source 1 has a
preliminary manifest only. Source-2 reconstruction of two files used the exact
baseline and unchanged test block; every original hash matches. Compiler/lint and
fixture failures, the zero-match filter, successful logs, metadata/recomputation
and the final executable remain available. All archive file hashes were verified.
All Cargo/test handles are terminal.

The new tests distinguish descriptive signed-data/storage predicates from actual
worker/Control components using injected command outcomes. They manufacture no
native or model-review evidence. Positive cases cover history exceeding the old
256 KiB record limit with compact publication below 60 KiB; negative cases cover
all-attempt omission, changed storage, selection substitution, inline mixing,
namespace/key changes, secret-bearing encoded data and shared input limits.

Commands:

- `cargo +stable clippy -p kegari-v0 --all-targets --offline --locked -- -D warnings`
- `cargo +stable test -p kegari-v0 --release --offline --locked --no-fail-fast`
- `cargo +stable build -p kegari-v0 --release --offline --locked --bin kegari-v0`
- `cargo +stable fmt --all -- --check`
- `python3 crates/kegari-v0/tools/contract_inventory.py --check`
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s crates/kegari-v0/tools -p 'test_contract_inventory.py'`

No native/live operation, installation, remote mutation or coordinator activation
was performed. This archive/storage/schema increment remains a separate verified
checkpoint from the prior parent-workflow increment; no history consolidation is
planned. Follow [MILESTONES.md](../MILESTONES.md) for remaining completion gates.
