#!/usr/bin/env python3
"""Build a review ledger from the approved contract, without declaring conformance.

Each normative prose block/table row has a content identity and source location.
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


def inventory():
    raw = (ROOT / "SPEC.md").read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != APPROVED:
        raise SystemExit("SPEC differs from approved contract; reconcile explicitly")
    blocks = []
    section = "1"
    paragraph = []
    start = 0
    in_fence = False

    def add(text, line, table=False):
        if not text or int(section.split(".")[0]) > 12:
            return
        # Section tables are normative even when individual rows omit MUST.
        if table or re.search(r"\b(MUST|SHALL|REQUIRED|SHOULD|MAY|RECOMMENDED)\b", text):
            identity = hashlib.sha256(text.encode()).hexdigest()
            blocks.append({
                "id": f"clause:{section}:{identity[:12]}",
                "section": section, "line": line, "sha256": identity,
                "excerpt": text[:180], "status": "not_assessed", "evidence": [],
            })

    for line_number, line in enumerate(raw.decode().splitlines(), 1):
        heading = re.match(r"^#{2,4} (\d+(?:\.\d+)*)(?:\.?\s)", line)
        if heading:
            add("\n".join(paragraph), start)
            paragraph = []
            section = heading[1]
        if line.startswith("```"):
            in_fence = not in_fence
        if in_fence:
            continue
        if not line.strip() or heading or line.startswith("|"):
            add("\n".join(paragraph), start)
            paragraph = []
            if line.startswith("|") and not re.match(r"^\|[\s:|-]+\|$", line):
                add(line, line_number, table=True)
        else:
            if not paragraph:
                start = line_number
            paragraph.append(line)
    add("\n".join(paragraph), start)
    identifiers = []
    for prefix, count, width in [("REQ", 62, 2), ("SC", 26, 2), ("V0-T", 37, 2), ("V0-E", 8, 2), ("V0-H", 3, 2)]:
        for number in range(1, count + 1):
            identifier = f"{prefix}{'-' if prefix in ('REQ', 'SC') else ''}{number:0{width}}"
            identifiers.append({"id": identifier, "status": "not_assessed", "evidence": []})
    return {"schema_version": 1, "spec_sha256": digest,
            "notice": "Coverage inventory is not validation. Full clauses remain in SPEC. Deterministic, actual-agent, integration and human evidence are distinct.",
            "clauses": blocks, "requirements": identifiers, "stale": []}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = ROOT / "validation" / "coverage.json"
    generated = inventory()
    if path.exists():
        prior = json.loads(path.read_text())
        for field in ("clauses", "requirements"):
            saved = {item["id"]: item for item in prior[field]}
            for item in generated[field]:
                previous = saved.pop(item["id"], None)
                if previous:
                    item.update({key: value for key, value in previous.items() if key not in ("id", "line", "section", "sha256", "excerpt")})
            if saved:
                generated.setdefault("stale", []).extend(saved.values())
        generated.setdefault("stale", []).extend(prior.get("stale", []))
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
