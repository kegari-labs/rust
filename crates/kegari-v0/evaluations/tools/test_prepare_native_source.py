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


class RecipeDispatchTests(unittest.TestCase):
    def test_default_dispatch_is_original_e05_and_unknown_recipe_never_reads(self):
        with mock.patch.object(module, "prepare", return_value="original") as e05:
            with mock.patch.object(module, "prepare_e07", return_value="explicit") as e07:
                self.assertEqual(module.prepare_selected("suite", "output"), "original")
                e05.assert_called_once_with("suite", "output")
                e07.assert_not_called()
                self.assertEqual(module.prepare_selected("suite", "output", module.E07_RECIPE), "explicit")
                e07.assert_called_once_with("suite", "output")
                with self.assertRaisesRegex(module.PreparationError, "unsupported explicit"):
                    module.prepare_selected("suite", "output", "arbitrary-recipe")
                self.assertEqual(e05.call_count, 1)
                self.assertEqual(e07.call_count, 1)


@unittest.skipUnless(os.environ.get("KEGARI_TEST_E07_SUITE"),
                     "root must select the actual legacy E07 suite with reviewed recipe data")
class E07RecipeTests(unittest.TestCase):
    def setUp(self):
        self.suite = Path(os.environ["KEGARI_TEST_E07_SUITE"]).resolve(strict=True)
        self.captured = module.capture(self.suite)

    def test_e07_recipe_plans_only_its_exact_source_without_any_object_ids(self):
        recipe, manifest, seed = module.plan_e07(self.captured)
        original = module.load_json(self.captured.files["manifest.json"][0])
        self.assertEqual(manifest, original)
        self.assertNotIn("base_sha", recipe["context_without_base"]["issue"]["pull_request"])
        self.assertNotIn("base", recipe["context_without_base"]["scope"])
        self.assertEqual(recipe["context_without_base"]["evidence"],
                         {"forbidden_canary_path": {"$kegari": "forbidden_canary_path"}})
        self.assertEqual(recipe["binding"]["schema_version"], 3)
        self.assertEqual(len(recipe["binding"]["slots"]), 10)
        before = self.captured.subtree(module.E07_FIXTURE + "/repository")
        self.assertEqual(set(seed.files), set(before.files))
        self.assertEqual(seed.directories, before.directories)
        for path, value in before.files.items():
            self.assertEqual(seed.files[path][1], value[1])
            if path != "WORKFLOW.md":
                self.assertEqual(seed.files[path], value)
        self.assertNotIn(b"@KegariScratch@", seed.files["WORKFLOW.md"][0])
        self.assertIn(b"  local: {}", seed.files["WORKFLOW.md"][0])
        self.assertIn(b"ROOT_V1", seed.files["AGENTS.md"][0])
        self.assertIn(b"NESTED_V1", seed.files["src/AGENTS.md"][0])
        self.assertEqual(seed.files["src/limit.py"][0], recipe["head_limit"].encode())
        self.assertEqual(module.capture(self.suite).files, self.captured.files)

    def test_e07_changed_recipe_variant_assets_or_policy_destination_refuse(self):
        changes = []
        files = dict(self.captured.files)
        raw, mode = files[module.E07_RECIPE_PATH]
        files[module.E07_RECIPE_PATH] = (raw + b"\n", mode)
        changes.append((files, "recipe bytes"))
        files = dict(self.captured.files)
        path = module.E07_FIXTURE + "/repository/src/limit.py"
        files[path] = (b"different input\n", files[path][1])
        changes.append((files, "fixture digest"))
        files = dict(self.captured.files)
        files[path] = (files[path][0], files[path][1] ^ stat.S_IXUSR)
        changes.append((files, "fixture digest"))
        files = dict(self.captured.files)
        manifest = module.load_json(files["manifest.json"][0])
        next(v for v in manifest["variants"] if v["id"] == "pr_attack")["phase"] = "independent_review"
        files["manifest.json"] = (module.json_bytes(manifest), files["manifest.json"][1])
        changes.append((files, "input variant"))
        files = dict(self.captured.files)
        files[module.E07_POLICY_PATH] = (b"do not overwrite", 0o600)
        changes.append((files, "existing binding or policy"))
        for files, message in changes:
            with self.subTest(message=message):
                with mock.patch.object(module.LocalGit, "run", side_effect=AssertionError("no Git")):
                    with self.assertRaisesRegex(module.PreparationError, message):
                        module.plan_e07(module.Captured(files, dict(self.captured.directories)))


@unittest.skipUnless(os.environ.get("KEGARI_TEST_E07_SUITE"),
                     "root must select actual legacy E07 inputs and invoke local Git explicitly")
class E07ActualLocalGitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kegari-e07-source-actual-git-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.suite = Path(os.environ["KEGARI_TEST_E07_SUITE"]).resolve(strict=True)

    def test_actual_e07_edge_preserves_other_cases_and_discloses_missing_runtime(self):
        before = module.capture(self.suite)
        recipe, original, seed = module.plan_e07(before)
        output = self.root / "e07"
        result = module.prepare_selected(self.suite, output, module.E07_RECIPE)
        actual = module.capture(output / "suite")
        after, variant = module.verify_e07_output(before, actual, recipe, original, result["base_commit"])
        self.assertEqual(after["profiles"], original["profiles"])
        self.assertEqual(len(after["variants"]), 28)
        self.assertEqual(sum(v["bounds"]["repetitions"] for v in after["variants"]), 56)
        self.assertEqual([v for v in original["variants"] if v["id"] != "pr_attack"],
                         [v for v in after["variants"] if v["id"] != "pr_attack"])
        for v in original["variants"]:
            self.assertEqual(actual.files[v["scorer"]], before.files[v["scorer"]])
        self.assertEqual(module.capture(self.suite).files, before.files)
        context = module.load_json(actual.files[module.E07_FIXTURE + "/context.json"][0])
        self.assertEqual(context["scope"]["base"], result["base_commit"])
        self.assertEqual(context["issue"]["pull_request"]["base_sha"], result["base_commit"])
        self.assertNotEqual(result["base_commit"], result["head_commit"])
        self.assertNotEqual(result["base_tree"], result["head_tree"])
        old_e05 = module.load_json(before.files[module.FIXTURE + "/context.json"][0])
        self.assertNotEqual(result["base_commit"], old_e05["scope"]["base"])
        self.assertEqual(context["issue"]["pull_request"]["body"], variant["initial_state"])
        self.assertNotIn("../scorer", variant["initial_state"])
        self.assertEqual(context["evidence"], {"forbidden_canary_path": {"$kegari": "forbidden_canary_path"}})
        self.assertFalse(result["runtime_supported"])
        self.assertFalse(result["dispatch_authorized"])
        self.assertFalse(result["canary_materialized"])
        self.assertEqual(len(result["missing_runtime_prerequisites"]), 2)
        self.assertNotIn("runtime_supported", context)
        self.assertNotIn("runtime_supported", after)
        self.assertEqual(module.canonical(after), result["suite_digest"])
        self.assertEqual(json.loads((output / "SOURCE-EVIDENCE.json").read_text()), result)
        actual_source = module.capture(output / "source", allow_generated_git=True)
        working = {k: v for k, v in actual_source.files.items() if k.split("/")[0] != ".git"}
        self.assertEqual(working, seed.files)
        commands = result["commands"]
        self.assertTrue(all(c["exit_code"] == 0 for c in commands))
        commits = [c for c in commands if c["argv"][1] == "commit"]
        self.assertEqual(len(commits), 2)
        self.assertTrue(all("--signoff" in c["argv"] and "Assisted-by: Codex" in c["argv"] for c in commits))
        blobs = [c for c in commands if c["argv"][1:3] == ["cat-file", "blob"]]
        self.assertEqual([c["stdout"] for c in blobs], [recipe["base_limit"], recipe["head_limit"]])
        self.assertFalse(list((output / "empty-git-hooks").iterdir()))
        self.assertFalse(list((output / "empty-git-template").iterdir()))
        second = module.prepare_selected(self.suite, self.root / "second", module.E07_RECIPE)
        self.assertEqual((result["base_commit"], result["head_commit"], result["suite_digest"]),
                         (second["base_commit"], second["head_commit"], second["suite_digest"]))
        # Use only these actual generated identities for output-corruption checks.
        for path in ("scorer/pr_attack.json", module.FIXTURE + "/context.json"):
            changed = dict(actual.files)
            changed[path] = (changed[path][0] + b"\n", changed[path][1])
            with self.assertRaisesRegex(module.PreparationError, "allowed delta"):
                module.verify_e07_output(before, module.Captured(changed, actual.directories),
                                         recipe, original, result["base_commit"])
        changed = dict(actual.files)
        path = module.E07_FIXTURE + "/repository/src/limit.py"
        changed[path] = (changed[path][0], changed[path][1] ^ stat.S_IXUSR)
        with self.assertRaisesRegex(module.PreparationError, "allowed delta"):
            module.verify_e07_output(before, module.Captured(changed, actual.directories),
                                     recipe, original, result["base_commit"])

    def test_e07_existing_nested_and_stale_inputs_refuse_without_git(self):
        existing = self.root / "existing"
        existing.mkdir()
        with mock.patch.object(module.LocalGit, "run", side_effect=AssertionError("no Git")):
            with self.assertRaisesRegex(module.PreparationError, "absent"):
                module.prepare_selected(self.suite, existing, module.E07_RECIPE)
            with self.assertRaisesRegex(module.PreparationError, "disjoint"):
                module.prepare_selected(self.suite, self.suite / "never-created", module.E07_RECIPE)
            copied = self.root / "copy"
            copied.mkdir()
            module.materialize(copied, module.capture(self.suite))
            path = copied / module.E07_FIXTURE / "repository/src/limit.py"
            path.write_bytes(b"changed input\n")
            with self.assertRaisesRegex(module.PreparationError, "fixture digest"):
                module.prepare_selected(copied, self.root / "refused", module.E07_RECIPE)
        self.assertFalse((self.root / "refused").exists())
        self.assertFalse((self.suite / "never-created").exists())
        self.assertEqual(list(existing.iterdir()), [])

    def test_e07_missing_identity_refuses_before_output(self):
        output = self.root / "missing-identity"
        with mock.patch.object(module.LocalGit, "run", return_value=""):
            with self.assertRaisesRegex(module.PreparationError, "committer"):
                module.prepare_selected(self.suite, output, module.E07_RECIPE)
        self.assertFalse(output.exists())

    def test_e07_failed_actual_commit_never_writes_placeholder_context(self):
        actual_run = module.LocalGit.run
        output = self.root / "failed-commit"
        def refuse_commit(git, cwd, args, env):
            if args[0] == "commit":
                raise module.PreparationError("explicit component failure before first commit")
            return actual_run(git, cwd, args, env)
        # Real invoking identity, actual local init/add; no fabricated positive OIDs.
        with mock.patch.object(module.LocalGit, "run", new=refuse_commit):
            with self.assertRaisesRegex(module.PreparationError, "before first commit"):
                module.prepare_selected(self.suite, output, module.E07_RECIPE)
        self.assertEqual(list((output / "suite").iterdir()), [])
        self.assertFalse((output / "SOURCE-EVIDENCE.json").exists())


class CoordinationSubstrateTests(unittest.TestCase):
    """Copy/reset data only; never an Authorized gate, worker or model run."""
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kegari-e01-copy-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.suite = HERE.parent

    def test_e01_copy_only_preserves_all_other_variants_profiles_scorers_modes_and_input(self):
        before = module.capture(self.suite)
        output = self.root / "copied"
        with mock.patch.object(module, "LocalGit", side_effect=AssertionError("no Git")), \
             mock.patch.object(module, "clean_environment", side_effect=AssertionError("no host environment")):
            evidence = module.prepare_selected(self.suite, output, module.E01_RECIPE)
        actual = module.capture(output / "suite")
        recipe, manifest = module.plan_e01(before)
        result, variant = module.verify_e01_output(before, actual, recipe, manifest)
        self.assertEqual(module.capture(self.suite).files, before.files)
        self.assertEqual(result["profiles"], manifest["profiles"])
        self.assertEqual([v for v in result["variants"] if v["id"] != "coordination"],
                         [v for v in manifest["variants"] if v["id"] != "coordination"])
        for name, value in before.files.items():
            if name.startswith("scorer/"):
                self.assertEqual(actual.files[name], value)
        self.assertEqual(list((output / "suite/fixtures/coordination/repository").iterdir()), [])
        self.assertEqual(actual.files[module.E01_FIXTURE + "/context.json"],
                         before.files[module.E01_FIXTURE + "/context.json"])
        self.assertNotIn(module.E01_FIXTURE + "/context-binding.json", actual.files)
        self.assertEqual(evidence["commands"], [])
        self.assertIsNone(evidence["base_commit"])
        self.assertIsNone(evidence["head_commit"])
        for name in ("dispatch_authorized", "native_executed", "runtime_supported"):
            self.assertIs(evidence[name], False)
        self.assertEqual(evidence["fixture_digest"], variant["fixture_digest"])
        self.assertEqual(evidence["slots"], 56)

    def test_e01_refuses_overwrite_recipe_change_and_unrelated_derived_changes(self):
        before = module.capture(self.suite)
        recipe, manifest = module.plan_e01(before)
        expected, _, _ = module.derived_e01_suite(before, recipe, manifest)
        for target in ["scorer/coordination.json", "fixtures/independent_malicious/context.json",
                       module.E01_FIXTURE + "/context.json"]:
            files = dict(expected.files)
            files[target] = (files[target][0] + b" ", files[target][1])
            with self.assertRaises(module.PreparationError):
                module.verify_e01_output(before, module.Captured(files, expected.directories), recipe, manifest)
        files = dict(before.files)
        files[module.E01_RECIPE_PATH] = (files[module.E01_RECIPE_PATH][0] + b" ", 0o644)
        with self.assertRaisesRegex(module.PreparationError, "recipe bytes"):
            module.plan_e01(module.Captured(files, before.directories))
        existing = self.root / "existing"
        existing.mkdir()
        with self.assertRaisesRegex(module.PreparationError, "absent"):
            module.prepare_selected(self.suite, existing, module.E01_RECIPE)
        self.assertEqual(list(existing.iterdir()), [])

    def test_e01_refuses_new_binding_prior_and_nonempty_output_repository(self):
        before = module.capture(self.suite)
        for leaf in ["context-binding.json", "prior.json", "substrate.json"]:
            files = dict(before.files)
            files[module.E01_FIXTURE + "/" + leaf] = (b"{}", 0o600)
            # Update the fixture digest so stale-byte validation is not enough:
            # the closed recipe rejects this changed variant identity before
            # the fixture topology gate is reached.
            value = json.loads(files["manifest.json"][0])
            variant = next(v for v in value["variants"] if v["id"] == "coordination")
            variant["fixture_digest"] = module.canonical(module.Captured(files, before.directories).snapshot(module.E01_FIXTURE))
            files["manifest.json"] = (module.json_bytes(value), files["manifest.json"][1])
            with self.assertRaises(module.PreparationError):
                module.plan_e01(module.Captured(files, before.directories))
        recipe, manifest = module.plan_e01(before)
        expected, _, _ = module.derived_e01_suite(before, recipe, manifest)
        directories = dict(expected.directories)
        directories[module.E01_FIXTURE + "/repository/.git"] = 0o700
        with self.assertRaises(module.PreparationError):
            module.verify_e01_output(before, module.Captured(expected.files, directories), recipe, manifest)

    def test_e01_selector_keeps_default_and_e07_routes_exact(self):
        with mock.patch.object(module, "prepare", return_value="default") as ordinary, \
             mock.patch.object(module, "prepare_e07", return_value="e07") as e07, \
             mock.patch.object(module, "prepare_e01", return_value="e01") as e01:
            self.assertEqual(module.prepare_selected("suite", "output"), "default")
            self.assertEqual(module.prepare_selected("suite", "output", module.E07_RECIPE), "e07")
            self.assertEqual(module.prepare_selected("suite", "output", module.E01_RECIPE), "e01")
            ordinary.assert_called_once_with("suite", "output")
            e07.assert_called_once_with("suite", "output")
            e01.assert_called_once_with("suite", "output")
            with self.assertRaises(module.PreparationError):
                module.prepare_selected("suite", "output", "unknown")


if __name__ == "__main__":
    unittest.main()
