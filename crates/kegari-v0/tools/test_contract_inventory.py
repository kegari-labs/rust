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
        prior = {"clauses": [same, changed_old], "requirements": [
            {"id": "V0-T14", "status": "partial", "evidence": [{"path": "named-evidence.json"}]}],
            "stale": [old_stale]}
        generated = {"clauses": [clause("An unchanged obligation.", 11), changed_new],
                     "requirements": [{"id": "V0-T14", "status": "not_assessed", "evidence": []}],
                     "stale": []}
        result = inventory.preserve_evidence(copy.deepcopy(generated), prior)
        self.assertEqual(result["clauses"][0], {**same, "line": 11})
        self.assertEqual(result["clauses"][1], changed_new)
        self.assertEqual(result["requirements"], prior["requirements"])
        self.assertEqual(result["stale"], [changed_old, old_stale])
        self.assertEqual(inventory.preserve_evidence(copy.deepcopy(generated), result), result)

    def test_changed_spec_still_requires_explicit_reconciliation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "SPEC.md").write_text("## 1. Changed contract\n\nA different obligation.\n")
            with patch.object(inventory, "ROOT", root):
                with self.assertRaisesRegex(SystemExit, "SPEC differs from approved contract"):
                    inventory.inventory()


if __name__ == "__main__":
    unittest.main()
