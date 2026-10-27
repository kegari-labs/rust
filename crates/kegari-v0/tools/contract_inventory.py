#!/usr/bin/env python3
"""Build a review ledger from the approved contract, without declaring conformance.

Every content block in the preamble and sections 1–14 is included, without
inferring requirement strength from keywords. This conservative inventory includes
context as well as normative prose, lists, table rows and fenced forms. Headings
and table separators are structural; section 15 is attribution and verbatim legal
material outside this implementation-coverage ledger. The SPEC remains authority.
Evidence entries are maintained explicitly. Changed clause and named-definition
identities preserve prior entries as stale instead of inheriting an assessment.
Definition bindings do not replace review of changed cross-references or turn
historical partial evidence into current implementation/qualification evidence.
"""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
APPROVED_VERSION = "1.4"
APPROVED = "b01064b5238554d2def840cabf6a9e5d555535e35ce4877838a55ade2dd0a6e7"
LEGACY_UNBOUND_SPEC = "35d4515585fe92350ec1cc8b78a5919c834d559a48d48d071239d39305032fb3"

# H requirements are defined by their normative section as well as the summary
# row. T17/T34 explicitly exercise the amended carry/context obligations; bind
# those sections too, rather than carrying their old partial assessments forward.
DEFINITION_SECTIONS = {
    "V0-H01": "3.6", "V0-H02": "7.5", "V0-H03": "12.4", "V0-H04": "1.4",
    "V0-T17": "9.2", "V0-T34": "7.5",
}


def content_blocks(text):
    """Yield (section, first line, exact text); do not guess normative strength."""
    section = "0"  # Preamble, before the first numbered section.
    paragraph = []
    start = 0
    fence = None

    def block():
        if paragraph and int(section.split(".")[0]) <= 14:
            return [(section, start, "\n".join(paragraph))]
        return []

    for line_number, line in enumerate(text.splitlines(), 1):
        if fence is not None:
            paragraph.append(line)
            if re.fullmatch(re.escape(fence[0]) + "{" + str(len(fence)) + r",}\s*", line):
                yield from block()
                paragraph = []
                fence = None
            continue
        heading = re.match(r"^#{1,6} (\d+(?:\.\d+)*)(?:\.?\s)", line)
        if re.match(r"^#{1,6}(?:\s|$)", line):
            yield from block()
            paragraph = []
            if heading:
                section = heading[1]
            continue
        opening = re.match(r"^(`{3,}|~{3,})", line)
        if opening:
            yield from block()
            paragraph = [line]
            start = line_number
            fence = opening[1]
            continue
        if not line.strip() or line.startswith("|"):
            yield from block()
            paragraph = []
            if line.startswith("|") and not re.match(r"^\|[\s:|-]+\|$", line):
                if int(section.split(".")[0]) <= 14:
                    yield section, line_number, line
        else:
            if not paragraph:
                start = line_number
            paragraph.append(line)
    if fence is not None:
        raise SystemExit("SPEC contains an unterminated fenced block")
    yield from block()


def named_definition(identifier, clauses):
    """Bind the exact defining row/section, excluding changing line numbers.

    Legacy clause excerpts retain the complete first table cell; full SHA-256
    values, not excerpts, bind the selected content. Missing/ambiguous definitions
    refuse reconciliation. This is a review ledger, not a capability or a parser
    for target-project manifests.
    """
    if identifier.startswith("REQ-") or identifier.startswith("V0-H"):
        section = "13"
    elif identifier.startswith("SC-"):
        section = "12.1"
    elif identifier.startswith("V0-T"):
        section = "12.2"
    else:
        section = "12.4.1"
    rows = []
    for clause in clauses:
        excerpt = clause["excerpt"]
        if clause["section"] != section or not excerpt.startswith("|"):
            continue
        first_cell = excerpt.split("|", 2)[1]
        if identifier in re.findall(r"(?:REQ-|SC-|V0-[TEH])\d{2}\b", first_cell):
            rows.append(clause)
    if len(rows) != 1:
        raise SystemExit(f"{identifier} needs exactly one defining contract row")
    selected = {rows[0]["id"]: rows[0]}
    normative = DEFINITION_SECTIONS.get(identifier)
    if normative:
        members = [clause for clause in clauses
                   if clause["section"] == normative
                   or clause["section"].startswith(normative + ".")]
        if not members:
            raise SystemExit(f"{identifier} is missing its normative section")
        selected.update((clause["id"], clause) for clause in members)
    ordered = sorted(selected.values(), key=lambda clause: clause["id"])
    payload = [[clause["section"], clause["sha256"]] for clause in ordered]
    digest = hashlib.sha256(json.dumps(payload, separators=(",", ":")).encode()).hexdigest()
    return {"schema_version": 1, "sha256": digest,
            "clauses": [clause["id"] for clause in ordered]}


def inventory():
    raw = (ROOT / "SPEC.md").read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != APPROVED:
        raise SystemExit("SPEC differs from approved contract; reconcile explicitly")
    blocks = []
    for section, line, text in content_blocks(raw.decode()):
        identity = hashlib.sha256(text.encode()).hexdigest()
        blocks.append({
            "id": f"clause:{section}:{identity[:12]}",
            "section": section, "line": line, "sha256": identity,
            "excerpt": text[:180], "status": "not_assessed", "evidence": [],
        })
    identifiers = []
    for prefix, count, width in [("REQ", 62, 2), ("SC", 26, 2), ("V0-T", 39, 2), ("V0-E", 9, 2), ("V0-H", 4, 2)]:
        for number in range(1, count + 1):
            identifier = f"{prefix}{'-' if prefix in ('REQ', 'SC') else ''}{number:0{width}}"
            identifiers.append({"id": identifier, "definition": named_definition(identifier, blocks),
                                "status": "not_assessed", "evidence": []})
    return {"schema_version": 2, "spec_version": APPROVED_VERSION, "spec_sha256": digest,
            "notice": "Complete contract content inventory through section 14, including preamble, context and required fenced forms; headings, table separators and section 15 attribution/legal material are excluded. Inclusion does not infer requirement strength or validation. Named definition bindings cover their defining rows and explicitly mapped normative sections, not every transitive reference. Preserved partial evidence retains its original source/scope and is not current-spec qualification. Changed definitions are unassessed; their prior records remain stale history. Full clauses remain in SPEC. Deterministic, actual-agent, integration and human evidence are distinct.",
            "clauses": blocks, "requirements": identifiers, "stale": []}


def preserve_evidence(generated, prior):
    def archive(item, reason, definition=None):
        historical = dict(item)
        if definition is not None:
            historical["definition"] = definition
        historical.update(spec_sha256=prior["spec_sha256"], stale_reason=reason)
        generated["stale"].append(historical)

    for field in ("clauses", "requirements"):
        saved = {item["id"]: item for item in prior[field]}
        for item in generated[field]:
            previous = saved.pop(item["id"], None)
            if previous:
                if field == "requirements":
                    definition = previous.get("definition")
                    if definition is None:
                        if prior.get("schema_version") != 1 or prior.get("spec_sha256") != LEGACY_UNBOUND_SPEC:
                            raise SystemExit("Unbound named evidence needs explicit legacy reconciliation")
                        definition = named_definition(previous["id"], prior["clauses"])
                    if definition != item["definition"]:
                        archive(previous, "named_definition_changed", definition)
                        continue
                item.update({key: value for key, value in previous.items() if key not in ("id", "line", "section", "sha256", "excerpt", "definition")})
        for previous in saved.values():
            archive(previous, "definition_removed_or_changed")
    generated.setdefault("stale", []).extend(prior.get("stale", []))
    return generated


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = ROOT / "validation" / "coverage.json"
    generated = inventory()
    if path.exists():
        generated = preserve_evidence(generated, json.loads(path.read_text()))
    rendered = json.dumps(generated, indent=2, ensure_ascii=False) + "\n"
    if args.check:
        if not path.exists() or path.read_text() != rendered:
            raise SystemExit("coverage inventory needs reconciliation")
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rendered)
    print(f"{len(generated['clauses'])} contract blocks; {len(generated['requirements'])} named requirements; no implicit passes")


if __name__ == "__main__":
    main()
