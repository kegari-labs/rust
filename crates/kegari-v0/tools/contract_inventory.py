#!/usr/bin/env python3
"""Build a review ledger from the approved contract, without declaring conformance.

Every content block in the preamble and sections 1–14 is included, without
inferring requirement strength from keywords. This conservative inventory includes
context as well as normative prose, lists, table rows and fenced forms. Headings
and table separators are structural; section 15 is attribution and verbatim legal
material outside this implementation-coverage ledger. The SPEC remains authority.
Evidence entries are maintained explicitly. A changed contract preserves prior
entries as stale instead of silently inheriting a pass.
"""
import argparse
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]
APPROVED = "80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326"


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
    for prefix, count, width in [("REQ", 62, 2), ("SC", 26, 2), ("V0-T", 37, 2), ("V0-E", 8, 2), ("V0-H", 3, 2)]:
        for number in range(1, count + 1):
            identifier = f"{prefix}{'-' if prefix in ('REQ', 'SC') else ''}{number:0{width}}"
            identifiers.append({"id": identifier, "status": "not_assessed", "evidence": []})
    return {"schema_version": 1, "spec_sha256": digest,
            "notice": "Complete contract content inventory through section 14, including preamble, context and required fenced forms; headings, table separators and section 15 attribution/legal material are excluded. Inclusion does not infer requirement strength or validation. Full clauses remain in SPEC. Deterministic, actual-agent, integration and human evidence are distinct.",
            "clauses": blocks, "requirements": identifiers, "stale": []}


def preserve_evidence(generated, prior):
    for field in ("clauses", "requirements"):
        saved = {item["id"]: item for item in prior[field]}
        for item in generated[field]:
            previous = saved.pop(item["id"], None)
            if previous:
                item.update({key: value for key, value in previous.items() if key not in ("id", "line", "section", "sha256", "excerpt")})
        if saved:
            generated.setdefault("stale", []).extend(saved.values())
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
