# Contract inventory correction

Status: verified locally; six parser/retention tests and ledger consistency pass.
This corrects validation bookkeeping and does not establish product conformance.

The earlier 363-block coverage inventory was internally consistent but incomplete.
Its selector required uppercase requirement keywords outside tables, skipped
fenced forms, and stopped at §12. It therefore omitted rules such as §6.4’s
attempt/session/review-round bounds, §6.2’s mandatory scheduler steps, §6.3’s
active-operation boundaries, the required §3.5 metadata forms, and §14’s explicit
verification obligations. Earlier successful inventory `--check` results establish
consistency with that selector; they do not establish complete normative coverage.
The historical test results and their source identities remain unchanged.

The separately prepared correction inventories 496 content blocks across the
preamble and §§1–14, including context without inferring requirement strength.
Headings, table separators and §15 attribution/legal material are explicitly
outside this implementation-coverage index. All 363 existing entries, all 136
named requirements and stale entries are preserved exactly. Each of the 133 added
blocks is `not_assessed` with empty evidence; V0-T14 remains unassessed. No SPEC
text or evidence status was upgraded.

Six TEMP inventory tests and the regenerated ledger consistency check passed.
A separate comparison confirms every in-scope nonstructural SPEC line is indexed
exactly once. These checks validate inventory and evidence-retention mechanics;
they are not product-conformance tests and do not increase the Rust test totals.

The exact three-file correction was installed against verified base/draft hashes.
Repository commands passed with unchanged tool/test/ledger/SPEC inputs:

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s crates/kegari-v0/tools -p test_contract_inventory.py -v`:6/6.
- `python3 crates/kegari-v0/tools/contract_inventory.py --check`:496 blocks/136 named requirements.
- `git diff --check`:passed.

The [machine record](contract-inventory-checks.json) contains command/log hashes,
Python version, source identity and exact preservation comparisons. The603 Rust
build inputs retain the full8-tested digest
`sha256:5e614d0b3417e80dc611d69f5750e7b3610c8cdfeac0b06104e24a8c8e86f3aa`;
no Cargo invocation was needed for this Python/ledger/prose correction.

Two independent static/data reviews found no additional material defect in the
parser, required omitted sections, unique coverage, stale identities or evidence
preservation. They independently checked all559 in-scope nonstructural lines
exactly once. Inclusion still does not decide a clause's normative strength.

The approved SPEC is unchanged. Future evidence assessments must explicitly cite
actual deterministic/agent/provider/human results; none is inferred by this tool.
