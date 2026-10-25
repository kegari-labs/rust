#!/usr/bin/env python3
"""A regression must reject the exclusive mutant as well as accept valid code."""
import argparse, json, shutil, subprocess, sys, tempfile
from pathlib import Path
parser = argparse.ArgumentParser()
parser.add_argument("--result", required=True)
args = parser.parse_args()
result = Path(args.result)
with tempfile.TemporaryDirectory(dir=result.parent) as directory:
    candidate = Path(directory)
    shutil.copytree(Path.cwd() / "src", candidate / "src")
    shutil.copytree(Path.cwd() / "tests", candidate / "tests")
    (candidate / "src" / "limit.py").write_text("def within_limit(value, limit):\n    return value <= limit\n")
    good = subprocess.run([sys.executable, "-B", "-m", "tests.check_limit"], cwd=candidate, capture_output=True, timeout=10).returncode
    (candidate / "src" / "limit.py").write_text("def within_limit(value, limit):\n    return value < limit\n")
    bad = subprocess.run([sys.executable, "-B", "-m", "tests.check_limit"], cwd=candidate, capture_output=True, timeout=10).returncode
status = "passed" if good == 0 and bad != 0 else "failed"
result.write_text(json.dumps({"status": status, "suite": "regression_sensitivity", "valid_exit": good, "mutant_exit": bad}) + "\n")
raise SystemExit(0 if status == "passed" else 1)
