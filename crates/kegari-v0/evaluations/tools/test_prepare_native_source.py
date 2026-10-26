"""Component tests for local source data preparation; never a native evaluation.

Root runs with KEGARI_TEST_NATIVE_SUITE pointing at an actual native-ready suite.
The positive tests use the invoking human's current Git identity, never a fixed one.
"""
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
from unittest import mock

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("prepare_native_source", HERE / "prepare_native_source.py")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kegari-source-capture-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()

    def test_regular_bytes_and_executable_mode_are_captured_without_execution(self):
        payload = self.root / "payload.py"
        payload.write_bytes(b"raise AssertionError('fixture must never execute')\n")
        payload.chmod(0o755)
        got = module.capture(self.root)
        self.assertEqual(got.files["payload.py"], (payload.read_bytes(), 0o755))

    def test_symlink_hardlink_fifo_and_git_metadata_are_refused(self):
        outside = self.root / "outside"
        outside.write_text("never follow")
        suite = self.root / "suite"
        suite.mkdir()
        alias = suite / "alias"
        alias.symlink_to(outside)
        with self.assertRaises(module.PreparationError):
            module.capture(suite)
        alias.unlink()
        os.link(outside, alias)
        with self.assertRaises(module.PreparationError):
            module.capture(suite)
        alias.unlink()
        os.mkfifo(alias)
        with self.assertRaises(module.PreparationError):
            module.capture(suite)
        alias.unlink()
        (suite / ".GiTconfig").write_text("[filter]\n")
        with self.assertRaisesRegex(module.PreparationError, "Git metadata"):
            module.capture(suite)

    def test_capture_bounds_and_post_read_mutation_are_refused(self):
        file = self.root / "source"
        file.write_bytes(b"original")
        with mock.patch.object(module, "MAX_BYTES", 2):
            with self.assertRaisesRegex(module.PreparationError, "64 MiB"):
                module.capture(self.root)
        with mock.patch.object(module, "MAX_ENTRIES", 0):
            with self.assertRaisesRegex(module.PreparationError, "4096"):
                module.capture(self.root)
        nested = self.root / "nested"
        nested.mkdir()
        with mock.patch.object(module, "MAX_DEPTH", 0):
            with self.assertRaisesRegex(module.PreparationError, "depth"):
                module.capture(self.root)
        real_read = os.read
        changed = False

        def mutate_after_actual_read(fd, count):
            nonlocal changed
            data = real_read(fd, count)
            if data and not changed:
                changed = True
                file.write_bytes(b"changed-size-after-read")
            return data

        with mock.patch.object(module.os, "read", side_effect=mutate_after_actual_read):
            with self.assertRaisesRegex(module.PreparationError, "changed"):
                module.capture(self.root)
        self.assertTrue(changed)

    def test_directory_name_iterator_stops_at_cap_plus_one_and_closes(self):
        # Pure iterator plumbing around the same production helper, not a huge
        # allocated directory. It may inspect one overflow entry, never the rest.
        class Entries:
            def __init__(self):
                self.read = 0
                self.closed = False

            def __enter__(self):
                return self

            def __exit__(self, *_):
                self.closed = True

            def __iter__(self):
                return self

            def __next__(self):
                self.read += 1
                if self.read > 4:
                    raise AssertionError("iterator consumed beyond the permitted overflow probe")
                return type("Entry", (), {"name": str(self.read)})()

        entries = Entries()
        with mock.patch.object(module.os, "scandir", return_value=entries):
            with self.assertRaisesRegex(module.PreparationError, "4096"):
                module.bounded_names(123, 3)
        self.assertEqual(entries.read, 4)
        self.assertTrue(entries.closed)

    def test_directory_budget_reserves_siblings_and_rechecks_with_fresh_total(self):
        # All three root entries reserve capacity before the nested directory is
        # inspected. Its one child exceeds the whole-tree bound of three.
        (self.root / "a").mkdir()
        (self.root / "a/child").write_bytes(b"child")
        (self.root / "b").write_bytes(b"b")
        (self.root / "c").write_bytes(b"c")
        with mock.patch.object(module, "MAX_ENTRIES", 3):
            with self.assertRaisesRegex(module.PreparationError, "4096"):
                module.capture(self.root)
        with mock.patch.object(module, "MAX_ENTRIES", 4):
            result = module.capture(self.root)
        self.assertEqual(set(result.files), {"a/child", "b", "c"})

        # The final verification pass uses the same bounded collection. Insert
        # a late root entry just as that pass starts; do not skip the inventory
        # or metadata check merely because the first capture fitted its bound.
        real = module.bounded_names
        calls = 0
        def add_on_recheck(fd, remaining):
            nonlocal calls
            calls += 1
            if calls == 3:
                (self.root / "late").write_bytes(b"late")
            return real(fd, remaining)
        with mock.patch.object(module, "MAX_ENTRIES", 4):
            with mock.patch.object(module, "bounded_names", side_effect=add_on_recheck):
                with self.assertRaisesRegex(module.PreparationError, "inventory changed|4096"):
                    module.capture(self.root)
        self.assertEqual(calls, 3)

    def test_writable_parent_and_replaceable_ancestry_refuse_before_output(self):
        for mode in (0o720, 0o702):
            parent = self.root / ("parent-" + str(mode))
            parent.mkdir()
            parent.chmod(mode)
            with self.assertRaisesRegex(module.PreparationError, "group/world writable"):
                with module.OutputGuard(parent, "never-created"):
                    self.fail("unsafe output must not be opened")
            self.assertFalse((parent / "never-created").exists())
            parent.chmod(0o700)
        writable = self.root / "writable-ancestor"
        writable.mkdir()
        writable.chmod(0o777)
        parent = writable / "private-child"
        parent.mkdir(mode=0o700)
        with self.assertRaisesRegex(module.PreparationError, "foreign namespace"):
            with module.OutputGuard(parent, "never-created"):
                self.fail("replaceable ancestry must not be used")
        self.assertFalse((parent / "never-created").exists())
        writable.chmod(0o700)

    def test_substituted_output_root_refuses_next_phase_without_writing_either_root(self):
        # Deterministic same-user substitution tests detection at a phase boundary;
        # it does not claim protection from arbitrary concurrent same-UID races.
        original = self.root / "output"
        retained = self.root / "retained"
        with module.OutputGuard(self.root, "output") as owned:
            original.rename(retained)
            original.mkdir(mode=0o700)
            marker = original / "unrelated"
            marker.write_bytes(b"preserve")
            with self.assertRaisesRegex(module.PreparationError, "substituted"):
                owned.phase(module.write_new, owned.root_fd, "must-not-write", b"no")
            with self.assertRaisesRegex(module.PreparationError, "substituted"):
                owned.mkdir("must-not-create")
            self.assertEqual(list(retained.iterdir()), [])
            self.assertEqual(list(original.iterdir()), [marker])
            self.assertEqual(marker.read_bytes(), b"preserve")

    def test_duplicate_json_and_noninteger_metadata_are_refused(self):
        for data in (b'{"a":1,"a":2}', b'{"a":NaN}', b'{"a":1.25}'):
            with self.assertRaises(module.PreparationError):
                module.load_json(data)

    def test_source_environment_drops_inherited_git_authority(self):
        with mock.patch.dict(os.environ, {"GIT_DIR": "/foreign", "GIT_CONFIG_COUNT": "99",
                                         "GIT_SSH_COMMAND": "never-execute"}):
            clean = module.clean_environment()
        self.assertFalse(any(k.upper().startswith("GIT_") for k in clean))


@unittest.skipUnless(os.environ.get("KEGARI_TEST_NATIVE_SUITE"),
                     "root must select the actual native-ready captured suite")
class ActualLocalGitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kegari-source-actual-git-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.suite = Path(os.environ["KEGARI_TEST_NATIVE_SUITE"]).resolve(strict=True)

    def test_actual_two_commits_preserve_suite_profiles_modes_and_payload_bytes(self):
        original = module.capture(self.suite)
        before = module.load_json(original.files["manifest.json"][0])
        output = self.root / "prepared"
        result = module.prepare(self.suite, output)
        actual = module.capture(output / "suite")
        after = module.load_json(actual.files["manifest.json"][0])
        evidence = json.loads((output / "SOURCE-EVIDENCE.json").read_text())
        self.assertEqual(before["profiles"], after["profiles"])
        self.assertEqual(len(after["variants"]), 28)
        self.assertEqual(after["max_trials"], 56)
        self.assertTrue(all(v["bounds"]["repetitions"] == 2 for v in after["variants"]))
        changed = [p for p in original.files if original.files[p] != actual.files[p]]
        self.assertTrue(set(changed) <= {"manifest.json", module.FIXTURE + "/context.json"})
        self.assertEqual(set(original.files), set(actual.files))
        for path, (_, mode) in original.files.items():
            self.assertEqual(mode, actual.files[path][1])
        self.assertEqual(module.capture(self.suite).files, original.files)
        context = module.load_json(actual.files[module.FIXTURE + "/context.json"][0])
        self.assertEqual(context["scope"]["base"], result["base_commit"])
        self.assertEqual(context["issue"]["pull_request"]["base_sha"], result["base_commit"])
        self.assertEqual(module.canonical(after), evidence["suite_digest"])
        self.assertNotEqual(result["base_commit"], result["head_commit"])
        self.assertTrue(evidence["profiles_unchanged"])
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
        self.assertFalse(list((output / "empty-git-hooks").iterdir()))
        self.assertFalse(list((output / "empty-git-template").iterdir()))
        self.assertTrue(all(c["exit_code"] == 0 for c in evidence["commands"]))
        commits = [c for c in evidence["commands"] if c["argv"][1] == "commit"]
        self.assertEqual(len(commits), 2)
        self.assertTrue(all("--signoff" in c["argv"] for c in commits))
        self.assertTrue(all("Assisted-by: Codex" in c["argv"] for c in commits))
        # A reproducible second invocation uses the same current human identity
        # and explicit fixture timestamps; no fixed test identity is installed.
        second = module.prepare(self.suite, self.root / "second")
        self.assertEqual((result["base_commit"], result["head_commit"], result["suite_digest"]),
                         (second["base_commit"], second["head_commit"], second["suite_digest"]))

    def test_existing_output_and_nested_output_refuse_before_git_or_write(self):
        output = self.root / "existing"
        output.mkdir()
        with mock.patch.object(module.LocalGit, "run", side_effect=AssertionError("no Git expected")):
            with self.assertRaisesRegex(module.PreparationError, "absent"):
                module.prepare(self.suite, output)
            with self.assertRaisesRegex(module.PreparationError, "disjoint"):
                module.prepare(self.suite, self.suite / "new-output-must-not-exist")
        self.assertEqual(list(output.iterdir()), [])
        self.assertFalse((self.suite / "new-output-must-not-exist").exists())

    def test_stale_fixture_input_refuses_without_output(self):
        copy_root = self.root / "input"
        copy_root.mkdir()
        module.materialize(copy_root, module.capture(self.suite))
        fixture = copy_root / module.FIXTURE / "repository/src/limit.py"
        fixture.write_bytes(b"unexpected source\n")
        with mock.patch.object(module.LocalGit, "run", side_effect=AssertionError("no Git expected")):
            with self.assertRaisesRegex(module.PreparationError, "fixture digest"):
                module.prepare(copy_root, self.root / "never-created")
        self.assertFalse((self.root / "never-created").exists())

    def test_missing_human_identity_refuses_before_output(self):
        # Failure injection tests identity refusal only. Positive evidence above
        # uses actual local Git and actual invoking human config.
        output = self.root / "unidentified"
        with mock.patch.object(module.LocalGit, "run", return_value=""):
            with self.assertRaisesRegex(module.PreparationError, "committer"):
                module.prepare(self.suite, output)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
