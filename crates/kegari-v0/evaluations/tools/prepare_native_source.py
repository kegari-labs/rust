#!/usr/bin/env python3
"""Prepare E05 local source data. Never execute fixture programs or activate a runtime."""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
from dataclasses import dataclass

MAX_ENTRIES = 4096
MAX_BYTES = 64 * 1024 * 1024
MAX_DEPTH = 32
GIT_TIMEOUT = 20
FIXED_TIME = "2026-10-26T00:00:00+00:00"
FIXTURE = "fixtures/independent_malicious"
HEAD = b"def within_limit(value, limit):\n    return value < limit\n"
BASE = b"def within_limit(value, limit):\n    return value <= limit\n"
FLAGS = os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
DIRECTORY_FLAGS = os.O_RDONLY | os.O_DIRECTORY | FLAGS


class PreparationError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise PreparationError(message)


def sha(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def canonical(value):
    raw = json.dumps({"schema_version": 1, "value": value}, ensure_ascii=False,
                     sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
    return "sha256:v1:" + hashlib.sha256(raw).hexdigest()


def json_bytes(value):
    return (json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n").encode()


def load_json(data):
    def pairs(values):
        result = {}
        for key, value in values:
            require(key not in result, "duplicate JSON field: " + key)
            result[key] = value
        return result

    def invalid_number(value):
        raise PreparationError("only finite integer JSON metadata is supported: " + value)

    return json.loads(data, object_pairs_hook=pairs, parse_float=invalid_number,
                      parse_constant=invalid_number)


def metadata(value):
    return (value.st_dev, value.st_ino, value.st_mode, value.st_nlink, value.st_uid,
            value.st_gid, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def relative(value):
    require(isinstance(value, str) and value and not value.startswith("/"),
            "expected a relative fixture path")
    pieces = value.split("/")
    for piece in pieces:
        require(piece not in ("", ".", "..") and all(32 <= ord(c) < 127 for c in piece)
                and "\\" not in piece, "noncanonical fixture path: " + repr(value))
    return pieces


def canonical_directory(value):
    path = Path(value)
    require(path.is_absolute() and str(path) == str(path.resolve(strict=True)),
            "use an existing canonical absolute directory (no symlink/parent traversal)")
    fd = os.open(path, DIRECTORY_FLAGS)
    try:
        info = os.fstat(fd)
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid(),
                "directory must be owned by the current user")
    finally:
        os.close(fd)
    return path


@dataclass(frozen=True)
class Captured:
    files: dict[str, tuple[bytes, int]]
    directories: dict[str, int]

    def snapshot(self, prefix=""):
        start = prefix + "/" if prefix else ""
        return {name[len(start):]: {"digest": sha(data), "mode": mode}
                for name, (data, mode) in sorted(self.files.items()) if name.startswith(start)}

    def subtree(self, prefix):
        relative(prefix)
        start = prefix + "/"
        require(prefix in self.directories, "missing captured directory: " + prefix)
        return Captured({name[len(start):]: value for name, value in self.files.items()
                         if name.startswith(start)},
                        {name[len(start):] if name != prefix else "": mode
                         for name, mode in self.directories.items()
                         if name == prefix or name.startswith(start)})


def bounded_names(fd, remaining):
    # Reserve all discovered siblings before descent; descendants share the same
    # per-tree budget. At most one extra iterator entry is examined to reject.
    names = []
    with os.scandir(fd) as entries:
        for entry in entries:
            require(len(names) < remaining, "capture exceeds 4096 entries")
            names.append(entry.name)
    return sorted(names)


def capture(root, *, allow_generated_git=False):
    """Bounded descriptor reads, no-follow traversal, and final metadata recheck."""
    root = canonical_directory(root)
    files, directories, observed, names_at = {}, {}, {}, {}
    total = count = checked_count = 0
    root_fd = os.open(root, DIRECTORY_FLAGS)

    def walk(fd, path, depth, checking=False):
        nonlocal total, count, checked_count
        require(depth <= MAX_DEPTH, "capture depth exceeds 32")
        before = os.fstat(fd)
        if checking:
            require(metadata(before) == observed[path], "directory changed during capture: " + path)
        else:
            require(before.st_uid == os.geteuid() and before.st_mode & 0o7000 == 0,
                    "directory ownership or special mode is unsupported")
            observed[path] = metadata(before)
            directories[path] = stat.S_IMODE(before.st_mode)
        used = checked_count if checking else count
        names = bounded_names(fd, MAX_ENTRIES - used)
        if checking:
            checked_count += len(names)
            require(names == names_at[path], "directory inventory changed: " + path)
        else:
            count += len(names)
            names_at[path] = names
        for name in names:
            relative(name)
            require(allow_generated_git or not name.casefold().startswith(".git"),
                    "captured input must contain no Git metadata/configuration paths")
            child = path + "/" + name if path else name
            info = os.stat(name, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                nested = os.open(name, DIRECTORY_FLAGS, dir_fd=fd)
                try:
                    require(metadata(info) == metadata(os.fstat(nested)), "directory replaced")
                    walk(nested, child, depth + 1, checking)
                finally:
                    os.close(nested)
            else:
                require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                        and info.st_uid == os.geteuid() and info.st_mode & 0o7000 == 0,
                        "symlink, hardlink, special file or foreign ownership: " + child)
                if checking:
                    require(metadata(info) == observed[child], "file changed during capture: " + child)
                    continue
                require(info.st_size <= MAX_BYTES - total, "capture exceeds 64 MiB")
                opened = os.open(name, os.O_RDONLY | FLAGS, dir_fd=fd)
                try:
                    require(metadata(info) == metadata(os.fstat(opened)), "file replaced before read")
                    chunks, size = [], 0
                    while True:
                        data = os.read(opened, min(1024 * 1024, MAX_BYTES - total - size + 1))
                        if not data:
                            break
                        size += len(data)
                        require(size <= MAX_BYTES - total, "capture exceeds 64 MiB")
                        chunks.append(data)
                    require(metadata(info) == metadata(os.fstat(opened)) and size == info.st_size,
                            "file changed while reading: " + child)
                finally:
                    os.close(opened)
                require(metadata(info) == metadata(os.stat(name, dir_fd=fd, follow_symlinks=False)),
                        "file replaced after read: " + child)
                observed[child] = metadata(info)
                files[child] = (b"".join(chunks), stat.S_IMODE(info.st_mode))
                total += size
        require(metadata(before) == metadata(os.fstat(fd)), "directory changed while reading")

    try:
        walk(root_fd, "", 0)
        walk(root_fd, "", 0, True)
        require(metadata(os.stat(root, follow_symlinks=False)) == observed[""], "capture root replaced")
    finally:
        os.close(root_fd)
    return Captured(files, directories)


def open_parent(root_fd, name):
    parts = relative(name)
    fd = os.dup(root_fd)
    try:
        for part in parts[:-1]:
            nested = os.open(part, DIRECTORY_FLAGS, dir_fd=fd)
            os.close(fd)
            fd = nested
        return fd, parts[-1]
    except BaseException:
        os.close(fd)
        raise


def write_new(root_fd, name, data, mode=0o600):
    parent, leaf = open_parent(root_fd, name)
    try:
        fd = os.open(leaf, os.O_WRONLY | os.O_CREAT | os.O_EXCL | FLAGS, mode, dir_fd=parent)
        try:
            os.fchmod(fd, mode)
            with os.fdopen(os.dup(fd), "wb") as output:
                output.write(data)
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        os.close(parent)


def materialize(root, captured):
    fd = os.open(root, DIRECTORY_FLAGS)
    try:
        # New directories remain writable until every captured file is installed.
        for name in sorted((n for n in captured.directories if n), key=lambda n: (n.count("/"), n)):
            parent, leaf = open_parent(fd, name)
            try:
                os.mkdir(leaf, 0o700, dir_fd=parent)
            finally:
                os.close(parent)
        for name, (data, mode) in sorted(captured.files.items()):
            write_new(fd, name, data, mode)
        # Directory modes are evidence only; owned output directories stay 0700.
    finally:
        os.close(fd)


def replace_owned(root, name, expected, replacement):
    root_fd = os.open(root, DIRECTORY_FLAGS)
    parent, leaf = open_parent(root_fd, name)
    os.close(root_fd)
    try:
        fd = os.open(leaf, os.O_RDWR | FLAGS, dir_fd=parent)
        try:
            info = os.fstat(fd)
            require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.geteuid(),
                    "owned output file changed identity")
            require(info.st_size == len(expected) and os.read(fd, len(expected) + 1) == expected,
                    "owned output bytes differ before derived rewrite")
            require(metadata(info) == metadata(os.fstat(fd)), "owned output changed during read")
            os.lseek(fd, 0, os.SEEK_SET)
            with os.fdopen(os.dup(fd), "wb") as output:
                output.write(replacement)
                output.truncate()
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        os.close(parent)


def require_protected_parent(info):
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid()
            and info.st_mode & 0o022 == 0,
            "output parent must be current-user-owned and not group/world writable")


def protected_ancestry(path):
    # Path-based Git cwd/hooks lookups additionally require protected ancestry.
    # A root-owned sticky temporary directory is allowed only for our own child.
    nodes = list(reversed(path.parents)) + [path]
    entries = [(node, os.stat(node, follow_symlinks=False)) for node in nodes]
    for index, (node, info) in enumerate(entries):
        require(stat.S_ISDIR(info.st_mode) and info.st_uid in (0, os.geteuid()),
                "output ancestry must contain only trusted owned directories: " + str(node))
        if info.st_mode & 0o022:
            require(index + 1 < len(entries) and info.st_mode & stat.S_ISVTX
                    and entries[index + 1][1].st_uid == os.geteuid(),
                    "output ancestry permits foreign namespace replacement: " + str(node))
    return [(str(node), info.st_dev, info.st_ino) for node, info in entries]


class OutputGuard:
    """Owned output descriptors plus phase-boundary identity checks.

    This protects normal namespace ownership; it does not claim resistance to
    an arbitrary concurrent process running as the same user.
    """
    def __init__(self, parent, name):
        self.parent = parent
        self.name = name
        self.path = parent / name
        self.parent_fd = self.root_fd = None
        self.parent_identity = self.root_identity = None
        self.ancestry = None

    def __enter__(self):
        try:
            self.parent_fd = os.open(self.parent, DIRECTORY_FLAGS)
            info = os.fstat(self.parent_fd)
            require_protected_parent(info)
            self.ancestry = protected_ancestry(self.parent)
            self.parent_identity = (info.st_dev, info.st_ino)
            current = os.stat(self.parent, follow_symlinks=False)
            require((current.st_dev, current.st_ino) == self.parent_identity,
                    "output parent changed before creation")
            os.mkdir(self.name, 0o700, dir_fd=self.parent_fd)
            self.root_fd = os.open(self.name, DIRECTORY_FLAGS, dir_fd=self.parent_fd)
            info = os.fstat(self.root_fd)
            self.root_identity = (info.st_dev, info.st_ino)
            self.check()
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        for field in ("root_fd", "parent_fd"):
            fd = getattr(self, field)
            if fd is not None:
                os.close(fd)
                setattr(self, field, None)

    def check(self):
        require(protected_ancestry(self.parent) == self.ancestry,
                "output ancestry identity changed")
        parent = os.stat(self.parent, follow_symlinks=False)
        require_protected_parent(parent)
        require((parent.st_dev, parent.st_ino) == self.parent_identity,
                "output parent was substituted")
        for info in (os.fstat(self.root_fd),
                     os.stat(self.name, dir_fd=self.parent_fd, follow_symlinks=False),
                     os.stat(self.path, follow_symlinks=False)):
            require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.geteuid()
                    and info.st_mode & 0o077 == 0
                    and (info.st_dev, info.st_ino) == self.root_identity,
                    "owned output root was substituted or became accessible to others")

    def mkdir(self, name):
        require(len(relative(name)) == 1, "expected direct owned output child")
        self.check()
        os.mkdir(name, 0o700, dir_fd=self.root_fd)
        self.check()

    def phase(self, operation, *args, **kwargs):
        self.check()
        try:
            return operation(*args, **kwargs)
        finally:
            self.check()


def clean_environment():
    return {key: value for key, value in os.environ.items() if not key.upper().startswith("GIT_")}


class LocalGit:
    def __init__(self, executable):
        self.executable = str(Path(executable).resolve(strict=True))
        self.commands = []

    def run(self, cwd, args, env):
        # Only these built-ins are used; no shell, remote, alias or fixture program.
        require(args and args[0] in {"config", "--version", "init", "add", "commit", "rev-parse",
                                   "status", "diff", "cat-file", "ls-files"}, "unsupported Git command")
        command = [self.executable, *args]
        before = os.stat(cwd, follow_symlinks=False)
        result = subprocess.run(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=GIT_TIMEOUT,
                                check=False)
        self.commands.append({"argv": command, "cwd": str(cwd), "exit_code": result.returncode,
                              "stdout": result.stdout.decode("utf-8", "strict"),
                              "stderr": result.stderr.decode("utf-8", "strict")})
        after = os.stat(cwd, follow_symlinks=False)
        require((before.st_dev, before.st_ino) == (after.st_dev, after.st_ino), "Git cwd replaced")
        require(result.returncode == 0, "local Git command failed: " + args[0])
        return result.stdout.decode("utf-8", "strict").strip()


def validate_suite(captured):
    require("manifest.json" in captured.files, "suite manifest is missing")
    manifest = load_json(captured.files["manifest.json"][0])
    variants = manifest["variants"]
    require(manifest["schema_version"] == 1 and len(variants) == 28
            and manifest["max_trials"] == 56
            and len({v["id"] for v in variants}) == 28
            and all(v["bounds"]["repetitions"] == 2 for v in variants),
            "this tool preserves the declared 28-variant / 56-slot suite only")
    for variant in variants:
        path = variant["fixture"]
        relative(path)
        require(canonical(captured.snapshot(path)) == variant["fixture_digest"],
                "input fixture digest differs: " + variant["id"])
        scorer = variant["scorer"]
        relative(scorer)
        require(sha(captured.files[scorer][0]) == variant["scorer_digest"], "input scorer digest differs")
    variant = next(v for v in variants if v["id"] == "independent_malicious")
    require(variant["fixture"] == FIXTURE and variant["case_id"] == "V0-E05"
            and variant["role"] == "independent_review" and variant["phase"] == "independent_review",
            "input must already contain the native-ready E05 independent-review fixture")
    context = load_json(captured.files[FIXTURE + "/context.json"][0])
    binding = load_json(captured.files[FIXTURE + "/context-binding.json"][0])
    expected_slots = {
        "/issue/pull_request/head_sha": "repository_revision",
        "/issue/pull_request/id": "independent_pull_request_id",
        "/issue/pull_request/number": "independent_pull_request_number",
        "/issue/pull_request/url": "independent_pull_request_url",
        "/operation/id": "operation_id", "/phase": "phase", "/profile": "profile",
        "/scope/head": "repository_revision", "/workspace/id": "workspace_id",
    }
    require(binding == {"schema_version": 2, "slots": [{"pointer": k, "source": v}
                for k, v in sorted(expected_slots.items())], "source_write_paths": []},
            "native context binding does not match the supported exact slots")
    for pointer, source in expected_slots.items():
        value = context
        for part in pointer.split("/")[1:]:
            value = value[part]
        require(value == {"$kegari": source}, "context binding placeholder differs: " + pointer)
    found_slots = {}
    def visit(value, pointer=""):
        if isinstance(value, dict):
            if "$kegari" in value:
                require(set(value) == {"$kegari"}, "reserved placeholder has extra fields")
                found_slots[pointer] = value["$kegari"]
            else:
                for key, item in value.items():
                    visit(item, pointer + "/" + key.replace("~", "~0").replace("/", "~1"))
        elif isinstance(value, list):
            for index, item in enumerate(value):
                visit(item, pointer + "/" + str(index))
    visit(context)
    require(found_slots == expected_slots, "context contains undeclared native placeholders")
    base = context["issue"]["pull_request"]["base_sha"]
    require(isinstance(base, str) and re.fullmatch("[0-9a-f]{40}", base)
            and context["scope"]["base"] == base, "original declared base pointers must agree")
    seed = captured.subtree(FIXTURE + "/repository")
    require(seed.files.get("src/limit.py", (None,))[0] == HEAD, "unexpected E05 boundary source")
    return manifest, variant, context, seed


def prepare(suite, output):
    suite = canonical_directory(suite)
    require(not any(part.casefold().startswith(".git") for part in suite.parts),
            "input must not be under a Git metadata path")
    output = Path(output)
    require(output.is_absolute() and output.name not in ("", ".", ".."), "output must be absolute")
    relative(output.name)
    parent = canonical_directory(output.parent)
    require_protected_parent(os.stat(parent, follow_symlinks=False))
    protected_ancestry(parent)
    require(not any(part.casefold().startswith(".git") for part in output.parts),
            "output must not be under a Git metadata path")
    require(str(output) == str(parent / output.name), "output must use canonical absolute spelling")
    require(not output.exists() and not output.is_symlink(), "output must be absent; no overwrite")
    require(not output.is_relative_to(suite) and not suite.is_relative_to(output),
            "input and output must be disjoint")
    captured = capture(suite)
    manifest, _, context, seed = validate_suite(captured)
    original_manifest = copy.deepcopy(manifest)
    executable = shutil.which("git", path=clean_environment().get("PATH"))
    require(executable is not None, "Git executable is unavailable")
    git = LocalGit(executable)
    cwd = canonical_directory(Path.cwd())
    identity_env = clean_environment()
    version = git.run(cwd, ["--version"], identity_env)
    name = git.run(cwd, ["config", "--get", "user.name"], identity_env)
    email = git.run(cwd, ["config", "--get", "user.email"], identity_env)
    require(name and email and all(ord(c) >= 32 and ord(c) != 127 for c in name + email)
            and all(c not in name + email for c in "<>")
            and re.fullmatch(r"[^\s@]+@[^\s@]+", email), "current Git committer name/email must be explicit single-line identity")
    # mkdir is exclusive. Failures keep owned partial data for inspection.
    with OutputGuard(parent, output.name) as owned:
        for child in ("suite", "source", "empty-git-template", "empty-git-hooks"):
            owned.mkdir(child)
        owned.phase(materialize, output / "suite", captured)
        owned.phase(materialize, output / "source", seed)
        source = output / "source"
        env = clean_environment()
        env.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_SYSTEM": os.devnull,
                    "GIT_CONFIG_GLOBAL": os.devnull, "GIT_TERMINAL_PROMPT": "0",
                    "GIT_ALLOW_PROTOCOL": "", "GIT_PROTOCOL_FROM_USER": "0",
                    "GIT_AUTHOR_NAME": name, "GIT_AUTHOR_EMAIL": email,
                    "GIT_COMMITTER_NAME": name, "GIT_COMMITTER_EMAIL": email,
                    "GIT_AUTHOR_DATE": FIXED_TIME, "GIT_COMMITTER_DATE": FIXED_TIME,
                    "GIT_CONFIG_COUNT": "6", "GIT_CONFIG_KEY_0": "core.hooksPath",
                    "GIT_CONFIG_VALUE_0": str(output / "empty-git-hooks"),
                    "GIT_CONFIG_KEY_1": "commit.gpgSign", "GIT_CONFIG_VALUE_1": "false",
                    "GIT_CONFIG_KEY_2": "tag.gpgSign", "GIT_CONFIG_VALUE_2": "false",
                    "GIT_CONFIG_KEY_3": "protocol.allow", "GIT_CONFIG_VALUE_3": "never",
                    "GIT_CONFIG_KEY_4": "core.autocrlf", "GIT_CONFIG_VALUE_4": "false",
                    "GIT_CONFIG_KEY_5": "core.filemode", "GIT_CONFIG_VALUE_5": "true"})
        def run(*args):
            return owned.phase(git.run, source, list(args), env)
        run("init", "--initial-branch=main", "--object-format=sha1", "--template=" + str(output / "empty-git-template"))
        owned.phase(replace_owned, source, "src/limit.py", HEAD, BASE)
        run("add", "--all")
        run("commit", "--signoff", "-m", "test(evaluation): seed inclusive boundary review base", "-m", "Assisted-by: Codex")
        base, base_tree = run("rev-parse", "HEAD"), run("rev-parse", "HEAD^{tree}")
        owned.phase(replace_owned, source, "src/limit.py", BASE, HEAD)
        run("add", "--", "src/limit.py")
        run("commit", "--signoff", "-m", "test(evaluation): introduce boundary defect for independent review", "-m", "Assisted-by: Codex")
        head, head_tree = run("rev-parse", "HEAD"), run("rev-parse", "HEAD^{tree}")
        require(all(re.fullmatch("[0-9a-f]{40}", v) for v in (base, base_tree, head, head_tree)), "invalid Git object identity")
        require(run("status", "--porcelain") == "" and run("rev-parse", "HEAD^") == base
                and run("diff", "--name-only", base, head) == "src/limit.py", "unexpected prepared source history")
        for commit in (base, head):
            message = run("cat-file", "commit", commit)
            require("Signed-off-by: " + name + " <" + email + ">" in message
                    and "Assisted-by: Codex" in message, "commit attribution is missing")
        require(run("cat-file", "blob", base + ":src/limit.py") == BASE.decode().strip()
                and run("cat-file", "blob", head + ":src/limit.py") == HEAD.decode().strip(), "Git source content differs")
        context["issue"]["pull_request"]["base_sha"] = base
        context["scope"]["base"] = base
        owned.phase(replace_owned, output / "suite", FIXTURE + "/context.json",
                      captured.files[FIXTURE + "/context.json"][0], json_bytes(context))
        updated = owned.phase(capture, output / "suite")
        variant = next(v for v in manifest["variants"] if v["id"] == "independent_malicious")
        variant["fixture_digest"] = canonical(updated.snapshot(FIXTURE))
        owned.phase(replace_owned, output / "suite", "manifest.json", captured.files["manifest.json"][0], json_bytes(manifest))
        prepared_suite = owned.phase(capture, output / "suite")
        validate_suite(prepared_suite)
        require(manifest["profiles"] == original_manifest["profiles"], "profile binding changed")
        original_manifest["variants"][next(i for i, v in enumerate(manifest["variants"])
            if v["id"] == "independent_malicious")]["fixture_digest"] = variant["fixture_digest"]
        require(manifest == original_manifest, "unexpected manifest semantic change")
        tracked = run("ls-files", "-z").split("\0")
        require(sorted(x for x in tracked if x) == sorted(seed.files), "Git omitted captured source files")
        prepared_source = owned.phase(capture, source, allow_generated_git=True)
        working = {k: v for k, v in prepared_source.files.items() if k.split("/")[0] != ".git"}
        require(working == seed.files, "source HEAD differs from captured repository data")
        evidence = {
            "schema_version": 1, "scope": "Owned local Git fixture data only; no native qualification",
            "input_suite": str(suite), "output": str(output), "git_executable": git.executable,
            "git_version": version, "fixed_fixture_commit_time": FIXED_TIME,
            "identity_source": "current invocation-directory Git user.name/user.email; no config changes",
            "human_name": name, "human_email": email, "base_commit": base, "base_tree": base_tree,
            "head_commit": head, "head_tree": head_tree, "fixture_digest": variant["fixture_digest"],
            "suite_digest": canonical(manifest), "variants": 28, "slots": 56,
            "profiles_unchanged": True, "input_files": captured.snapshot(),
            "suite_files": prepared_suite.snapshot(), "source_files_including_generated_git": prepared_source.snapshot(),
            "input_directory_modes": captured.directories, "suite_directory_modes": prepared_suite.directories,
            "source_directory_modes": prepared_source.directories, "commands": git.commands,
            "command_environment_policy": "inherited GIT_* removed; explicit identity/time; global/system Git config, hooks, signing, protocols disabled for source commands",
            "layout": {"suite": "copied captured suite with derived E05 base/digest", "source": "two local signed-off commits", "empty-git-template": "empty", "empty-git-hooks": "empty"},
            "limits": ["No fixture program, hook, payload or test was executed.",
                       "Profile bindings and unresolved profile text are copied unchanged; no account/model selected.",
                       "Production SuiteManifest/Plan, source admission, native isolation, provider, model and grading remain unqualified.",
                       "No remote commands, runtime activation, human acceptance or deployment."]}
        owned.phase(write_new, owned.root_fd, "SOURCE-EVIDENCE.json", json_bytes(evidence))
        return evidence


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", required=True, help="existing canonical native-ready suite directory")
    parser.add_argument("--output", required=True, help="absent canonical owned output directory")
    args = parser.parse_args()
    try:
        result = prepare(args.suite, args.output)
    except (PreparationError, OSError, ValueError, KeyError, StopIteration, subprocess.SubprocessError) as error:
        print("source preparation refused: " + str(error), file=sys.stderr)
        return 1
    print(json.dumps({k: result[k] for k in ("output", "base_commit", "head_commit", "fixture_digest", "suite_digest")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
