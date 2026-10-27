# SPEC 1.4 requirement inventory baseline

The inventory and checks below describe the original reconciliation checkpoint;
the later automatic-domain source update is recorded separately at the end. Its
implementation does not alter these historical counts or promote their evidence.

The M02-1 inventory reconciliation is complete. It records **519 current contract
blocks and 140 named requirements**, with no new implementation, native,
actual-agent, integration or human qualification claim. The incompatible
implementation still requires removal/rework and validation under
[MILESTONES.md](../MILESTONES.md); this inventory check does not complete M02-1.

The approved contract is [SPEC 1.4](../SPEC.md), SHA-256
`b01064b5238554d2def840cabf6a9e5d555535e35ce4877838a55ade2dd0a6e7`.
The repository baseline inspected for this reconciliation was
`304cc984721787f9148e7c0b246ad28dde60b149`, with the previously retained alpha.9
working changes present. Only the inventory tool, its focused Python tests, the
coverage ledger and this record are part of this inventory change. No product
code, runtime schemas, instructions, configuration or historical report was
changed by it.

## Identity and preservation rules

[The inventory tool](../tools/contract_inventory.py) now emits coverage-ledger
schema 2 with the approved specification version/hash. This is a maintainer
ledger format, not a runtime operation or authority format. Clause identities
still bind exact content through section 14; headings/table separators and
section 15 attribution/legal material remain outside this implementation ledger.

Named requirements now bind the full SHA-256 identities of their defining table
rows. H01–H04 additionally bind their normative sections (3.6, 7.5, 12.4 and 1.4).
T17 and T34 also bind sections 9.2 and 7.5: their carry/context obligations changed
even though their short scenario rows did not. Changed identities start
`not_assessed` with no current evidence. Their prior assessments and complete
evidence remain under `stale`, with the source SPEC hash and reason. Existing
stale records remain unchanged.

The one supported migration from unbound named rows is the known SPEC 1.3 ledger
(`35d4515585fe92350ec1cc8b78a5919c834d559a48d48d071239d39305032fb3`). It derives
old named definitions from that ledger's exact clause hashes and sections before
comparing them with the new definitions. It does not infer unchanged meaning from
an ID alone. Unknown unbound legacy data or missing/ambiguous definitions refuse
reconciliation. A repeated generation preserves the result without duplicate
history or automatic restoration of old assessments.

These bindings cover defining rows and the explicitly listed normative sections,
not every transitive cross-reference. Future amendments still need a review of
affected scope and evidence. An unchanged `partial` entry retains only its
original source, test class and stated limits; it does not become a SPEC 1.4 pass
or validate later production changes.

## Reconciled scope

| Inventory | Before | After | Treatment |
| --- | --- | --- | --- |
| Current clauses | 503 | 519 | 470 unchanged identities; 33 old identities archived; 49 new identities unassessed |
| Named requirements | 136 | 140 | Four new IDs; eight existing definitions reset; all other assessments/evidence preserved |
| Stale records | 3 | 44 | Original three plus 33 superseded clauses and eight old named definitions |
| Evidence entries, current plus stale | 326 | 326 | No historical evidence entry removed or promoted |

The new `V0-H04`, `V0-T38`, `V0-T39` and `V0-E09` rows are all `not_assessed`
with empty evidence. The existing named definitions reset as follows:

| Named requirement | Prior assessment | Reason for reassessment |
| --- | --- | --- |
| V0-T01 | not_assessed | Omitted workflow now selects the installed generic workflow; explicit invalid selection still blocks |
| V0-T17 | partial | Acceptance carry must preserve discovered validation obligations and refreshed command/environment evidence |
| V0-T33 | not_assessed | Generic workflow and optional starter replace mandatory workflow/command preparation |
| V0-T34 | partial | Instruction composition and provenance now include selected/default workflow identity |
| V0-E08 | partial | Continuation/recovery now explicitly retains discovered commands and validation obligations |
| V0-H01 | not_assessed | Delivered guidance and target preparation must support generic discovery |
| V0-H02 | partial | Its normative instruction-composition/provenance section changed |
| V0-H03 | partial | Required evaluation coverage now includes T38/T39 and E09 |

Current statuses are **477 unassessed / 42 partial clauses** and
**127 unassessed / 13 partial named requirements**. The eight archived partial
records consist of three superseded clauses and five named assessments. None of
the current rows has a full-pass status.

The three superseded clause assessments are
`clause:7.5:9b6744b5258a`, `clause:9.2:3de3818887a1` and
`clause:12.4.1:c8ed13bfc22e`. Their instruction-retirement, carried-acceptance and
evaluation evidence stays available with its original limits, including
[parent-retirement evidence](LOCAL-PARENT-RETIREMENT.md) and
[carried-acceptance evidence](LOCAL-CARRIED-ACCEPTANCE.md). Those historical
reports and their original SPEC 1.2/1.3 claims were not rewritten.

## Checks performed

All commands ran from the Rust repository root with Python bytecode writes
disabled. These checks exercise the inventory and evidence-preservation logic;
they do not execute the product or qualify generic commands.

| Command/check | Observed result |
| --- | --- |
| `python3 -B -m unittest discover -s crates/kegari-v0/tools -p test_contract_inventory.py -v` | 11 tests passed, no failures or skips; 0.026 seconds |
| `python3 -B crates/kegari-v0/tools/contract_inventory.py` | Generated 519 blocks / 140 named requirements |
| `python3 -B crates/kegari-v0/tools/contract_inventory.py --check` | Exact generated ledger matches; no implicit passes |
| Before/after JSON comparison | Every old assessment/evidence retained unchanged in current or stale records; all 49 new clause and four new named rows unassessed; all eight changed named definitions unassessed; 326 evidence entries retained |
| `git diff --check -- crates/kegari-v0/tools/contract_inventory.py crates/kegari-v0/tools/test_contract_inventory.py crates/kegari-v0/validation/coverage.json` | Passed |

The six existing inventory tests remain, with the named-retention fixture updated
to supply a definition identity. Five additional tests exercise same-ID changed
definitions, known legacy migration, unknown/stripped unbound evidence refusal,
normative-section changes despite an unchanged summary row, and missing/duplicate
definition refusal. Existing tests also cover all content through section 14,
fences, original stale history, repeat reconciliation and a changed approved hash.
The first run of these eleven tests passed; no failed validation run was replaced.

Before-edit files and hashes are retained at
`/private/tmp/kegari-spec14-inventory-before-rw_nr5lb/BASES.json`. The prior
coverage file SHA-256 is
`b500725efbad71c071ee77cbe272e681c7df611146f46aaa236a9e64e43a2780`.
The reconciled files at this checkpoint have these SHA-256 identities:

| File | SHA-256 |
| --- | --- |
| `tools/contract_inventory.py` | `089fa4d8163657959866c7abedeecaa38a0380e2db995cc98389f61af00c0804` |
| `tools/test_contract_inventory.py` | `35e0031dc3fef7a0b302aa12fc0e82f799f9a59d0c500664b85ca86128fb48c4` |
| `validation/coverage.json` | `379b62fe76d3510ea5d0503b2bc3dd8c64b450c91c636f25a615eebef85dfded` |

No Cargo test, product binary, provider request, native qualification, actual-agent
trial, human calibration, installation or commit was performed for this change.
Independent review and the remaining M02-1 removal/rework audit belong to the
coordinating agent's integration checkpoint.

## Later automatic-domain source update

After the `97a9d02` checkpoint, the current source replaces the mandatory
executable/digest/mapping catalog with host-produced managed schema 5. Preparation
binds a fixed backend-policy digest, then derives the actual command domain from
owned repository/scratch paths, fixed OS resources and already-admitted generic
resource grants. Worker contracts and cleanup retain that exact isolation.
Bounded schema-2 resource observations record original bytes, entry versions and
link text without claiming link-target or complete dependency closure. Full
content checks remain outside Control; serialized gates use retained bindings.

Configuration rejects removed catalog fields, including null/empty forms, and
retained catalog schemas 1–4 are not migrated. Schema 5 binds the exact build
identity, so a different build also refuses an incompatible retained cleanup
binding; it does not rewrite the evidence or fall back to deleting owned state.
Current source behavior and remaining limits are documented in
[configuration](../docs/configuration.md#generic-execution-migration) and
[process containment](../docs/process-containment.md#current-automatic-command-domain).

This is a source/contract description, not a new executed validation result.
The original inventory counts, file hashes and eleven-test result above remain
their original checkpoint evidence. This documentation update adds no coverage
assessment or test/native pass. Arbitrary worker-command invocation, the complete
discovery/validation-obligation flow and generic native qualification remain
incomplete. External host tooling still needs actual generic resource authority;
neither discovered paths nor an automatic domain grant new credentials, network
access or public-PR qualification.
