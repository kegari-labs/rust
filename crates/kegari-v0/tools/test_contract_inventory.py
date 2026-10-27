"""Inventory completeness and evidence retention, not implementation conformance."""
import copy
import hashlib
import pathlib
import re
import tempfile
import unittest
from unittest.mock import patch

import contract_inventory as inventory


class ContractInventoryTests(unittest.TestCase):
    def test_approved_spec_has_no_unindexed_content_through_section_14(self):
        text = (inventory.ROOT / "SPEC.md").read_text()
        lines = text.splitlines()
        end = lines.index("## 15. Attribution and Apache-2.0 license")
        expected = {
            number
            for number, line in enumerate(lines[:end], 1)
            if line.strip()
            and not re.match(r"^#{1,6}(?:\s|$)", line)
            and not re.match(r"^\|[\s:|-]+\|$", line)
        }
        covered = set()
        for section, start, block in inventory.content_blocks(text):
            self.assertLessEqual(int(section.split(".")[0]), 14)
            block_lines = block.splitlines()
            self.assertEqual(lines[start - 1:start - 1 + len(block_lines)], block_lines)
            positions = set(range(start, start + len(block_lines)))
            self.assertFalse(covered & positions)
            covered |= positions
        self.assertEqual(expected, covered)
        generated = inventory.inventory()
        ids = [item["id"] for item in generated["clauses"]]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(generated["spec_sha256"], inventory.APPROVED)
        self.assertEqual(generated["spec_version"], "1.4")
        self.assertEqual(len(generated["requirements"]), 140)
        self.assertTrue({"V0-H04", "V0-T38", "V0-T39", "V0-E09"}.issubset(
            {item["id"] for item in generated["requirements"]}))
        self.assertTrue(all(item["status"] == "not_assessed" and not item["evidence"]
                            for item in generated["clauses"] + generated["requirements"]))

    def test_previously_omitted_contract_obligations_are_present(self):
        blocks = list(inventory.content_blocks((inventory.ROOT / "SPEC.md").read_text()))
        expected = [
            ("6.2", "1. Refresh configuration, permission reductions, operator intent and connection health."),
            ("6.2", "6. Schedule the next tick, respecting polling/rate limits; coalesce refresh requests so they cannot create duplicate concurrent ticks."),
            ("6.2", "Immediately before dispatch: refresh the work identity"),
            ("6.3", "An active agent operation is one provider session"),
            ("6.3", "Pause/filter exclusion permits the currently admitted session"),
            ("6.4", "Each phase/work-generation has at most `max_attempts_per_phase`"),
            ("13", "“Retained” means the applicable behavior is normative here"),
            ("14", "The later implementation MUST verify actual supported schemas, permissions and versions"),
            ("14", "These verification limits are acceptance obligations"),
        ]
        for section, text in expected:
            with self.subTest(section=section, text=text):
                self.assertTrue(any(saved_section == section and text in block
                                    for saved_section, _, block in blocks))
        forms = [block for section, _, block in blocks
                 if section == "3.5" and block.startswith("```kegari\n")]
        self.assertEqual(len(forms), 2)
        self.assertIn('"kind":"parent"', forms[0])
        self.assertIn('"kind":"child"', forms[1])
        self.assertTrue(all(form.endswith("\n```") for form in forms))

    def test_context_lists_tables_and_fences_keep_exact_content(self):
        text = ("# Title\n\nPreamble without a requirement keyword.\n\n"
                "## 6.4 Bounds\n\nEach session has at most two turns.\n\n"
                "1. Refresh facts.\n2. Admit within bounds.\n\n"
                "| Fact | Meaning |\n| --- | --- |\n| Bound | Two |\n\n"
                "```json\n## 99. This is fenced data\n| --- | --- |\n\n{}\n```\n\n"
                "~~~text\nA second fenced form.\n~~~\n\n"
                "## 13. Traceability\n\nRetained behavior.\n\n"
                "## 14. Limits\n\nVerify actual compatibility.\n\n"
                "## 15. Attribution\n\nLegal material MUST NOT become a runtime test claim.\n")
        blocks = list(inventory.content_blocks(text))
        self.assertEqual([section for section, _, _ in blocks],
                         ["0", "6.4", "6.4", "6.4", "6.4", "6.4", "6.4", "13", "14"])
        self.assertEqual(blocks[2][2], "1. Refresh facts.\n2. Admit within bounds.")
        self.assertEqual(blocks[5][2], "```json\n## 99. This is fenced data\n| --- | --- |\n\n{}\n```")
        self.assertEqual(blocks[6][2], "~~~text\nA second fenced form.\n~~~")
        self.assertFalse(any("Legal material" in block for _, _, block in blocks))
        self.assertEqual(list(inventory.content_blocks("## 1. Heading\n\nLast block")),
                         [("1", 3, "Last block")])

    def test_unterminated_form_is_not_silently_omitted(self):
        with self.assertRaisesRegex(SystemExit, "unterminated fenced block"):
            list(inventory.content_blocks("## 3.5 Forms\n\n```json\n{}"))

    def test_status_and_stale_identity_are_preserved_without_inheritance(self):
        def clause(text, line):
            digest = hashlib.sha256(text.encode()).hexdigest()
            return {"id": f"clause:6.4:{digest[:12]}", "section": "6.4", "line": line,
                    "sha256": digest, "excerpt": text, "status": "not_assessed", "evidence": []}

        same = clause("An unchanged obligation.", 10)
        changed_old = clause("An older obligation.", 20)
        changed_new = clause("A changed obligation.", 21)
        same.update(status="partial", evidence=[{"path": "saved-evidence.json"}],
                    review_note="Retain this exact assessment.")
        changed_old.update(status="partial", evidence=[{"path": "old-evidence.json"}])
        old_stale = {"id": "clause:6.4:already_stale", "status": "not_assessed", "evidence": []}
        definition = {"schema_version": 1, "sha256": "a" * 64, "clauses": []}
        prior = {"schema_version": 2, "spec_sha256": inventory.APPROVED,
                 "clauses": [same, changed_old], "requirements": [
            {"id": "V0-T14", "definition": definition,
             "status": "partial", "evidence": [{"path": "named-evidence.json"}]}],
            "stale": [old_stale]}
        generated = {"clauses": [clause("An unchanged obligation.", 11), changed_new],
                     "spec_sha256": inventory.APPROVED,
                     "requirements": [{"id": "V0-T14", "definition": definition,
                                       "status": "not_assessed", "evidence": []}],
                     "stale": []}
        result = inventory.preserve_evidence(copy.deepcopy(generated), prior)
        self.assertEqual(result["clauses"][0], {**same, "line": 11})
        self.assertEqual(result["clauses"][1], changed_new)
        self.assertEqual(result["requirements"], prior["requirements"])
        self.assertEqual(result["stale"], [
            {**changed_old, "spec_sha256": inventory.APPROVED,
             "stale_reason": "definition_removed_or_changed"}, old_stale])
        self.assertEqual(inventory.preserve_evidence(copy.deepcopy(generated), result), result)

    @staticmethod
    def named_ledger(text, legacy=False):
        digest = hashlib.sha256(text.encode()).hexdigest()
        clause = {"id": f"clause:12.2:{digest[:12]}", "section": "12.2", "line": 1,
                  "sha256": digest, "excerpt": text, "status": "not_assessed", "evidence": []}
        item = {"id": "V0-T14", "status": "not_assessed", "evidence": []}
        if not legacy:
            item["definition"] = inventory.named_definition("V0-T14", [clause])
        return {"schema_version": 1 if legacy else 2,
                "spec_sha256": inventory.LEGACY_UNBOUND_SPEC if legacy else inventory.APPROVED,
                "clauses": [clause], "requirements": [item], "stale": []}

    def test_same_named_id_changed_definition_cannot_inherit_assessment(self):
        prior = self.named_ledger("| V0-T14 | The original obligation. |")
        prior["requirements"][0].update(
            status="partial", evidence=[{"reference": "original-result.json", "status": "targeted_pass"}])
        before = copy.deepcopy(prior)
        current = self.named_ledger("| V0-T14 | A different obligation with the same ID. |")
        result = inventory.preserve_evidence(copy.deepcopy(current), prior)
        self.assertEqual(result["requirements"][0]["status"], "not_assessed")
        self.assertEqual(result["requirements"][0]["evidence"], [])
        historical = next(item for item in result["stale"] if item["id"] == "V0-T14")
        for key, value in before["requirements"][0].items():
            self.assertEqual(historical[key], value)
        self.assertEqual(historical["stale_reason"], "named_definition_changed")
        self.assertEqual(historical["spec_sha256"], before["spec_sha256"])
        self.assertEqual(prior, before)
        self.assertEqual(inventory.preserve_evidence(copy.deepcopy(current), result), result)

    def test_known_legacy_definitions_are_derived_before_preserving_evidence(self):
        original = "| V0-T14 | An unchanged obligation. |"
        prior = self.named_ledger(original, legacy=True)
        prior["requirements"][0].update(status="partial", evidence=[{"reference": "legacy-result.json"}])
        current = self.named_ledger(original)
        kept = inventory.preserve_evidence(current, prior)
        self.assertEqual(kept["requirements"][0]["status"], "partial")
        self.assertEqual(kept["requirements"][0]["evidence"], prior["requirements"][0]["evidence"])
        self.assertEqual(kept["stale"], [])
        changed = inventory.preserve_evidence(
            self.named_ledger("| V0-T14 | The amended obligation. |"), prior)
        self.assertEqual(changed["requirements"][0]["status"], "not_assessed")
        old = next(item for item in changed["stale"] if item["id"] == "V0-T14")
        self.assertEqual(old["spec_sha256"], inventory.LEGACY_UNBOUND_SPEC)
        self.assertEqual(old["evidence"], prior["requirements"][0]["evidence"])

    def test_unknown_or_stripped_unbound_legacy_evidence_is_not_adopted(self):
        text = "| V0-T14 | An obligation. |"
        prior = self.named_ledger(text, legacy=True)
        prior["requirements"][0].update(status="partial", evidence=[{"reference": "unknown.json"}])
        for schema, digest in [(1, "0" * 64), (2, inventory.APPROVED)]:
            with self.subTest(schema=schema, digest=digest):
                candidate = {**prior, "schema_version": schema, "spec_sha256": digest}
                with self.assertRaisesRegex(SystemExit, "explicit legacy reconciliation"):
                    inventory.preserve_evidence(self.named_ledger(text), candidate)

    def test_normative_section_change_invalidates_named_context_assessment(self):
        current = inventory.inventory()
        prior = copy.deepcopy(current)
        for item in prior["requirements"]:
            if item["id"] in ("V0-H02", "V0-T34", "V0-T12"):
                item.update(status="partial", evidence=[{"reference": "earlier-context.json"}])
        changed = next(item for item in prior["clauses"] if item["section"] == "7.5")
        changed["sha256"] = hashlib.sha256(b"earlier context obligation").hexdigest()
        changed["id"] = "clause:7.5:" + changed["sha256"][:12]
        for item in prior["requirements"]:
            item["definition"] = inventory.named_definition(item["id"], prior["clauses"])
        result = inventory.preserve_evidence(current, prior)
        by_id = {item["id"]: item for item in result["requirements"]}
        self.assertEqual(by_id["V0-H02"]["status"], "not_assessed")
        self.assertEqual(by_id["V0-T34"]["status"], "not_assessed")
        self.assertEqual(by_id["V0-T12"]["status"], "partial")

    def test_missing_or_duplicate_defining_rows_refuse_reconciliation(self):
        clauses = self.named_ledger("| V0-T14 | An obligation. |")["clauses"]
        for invalid in [[], clauses + clauses]:
            with self.subTest(count=len(invalid)):
                with self.assertRaisesRegex(SystemExit, "exactly one defining contract row"):
                    inventory.named_definition("V0-T14", invalid)

    def test_changed_spec_still_requires_explicit_reconciliation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "SPEC.md").write_text("## 1. Changed contract\n\nA different obligation.\n")
            with patch.object(inventory, "ROOT", root):
                with self.assertRaisesRegex(SystemExit, "SPEC differs from approved contract"):
                    inventory.inventory()


if __name__ == "__main__":
    unittest.main()
