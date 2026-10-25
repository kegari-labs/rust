#!/usr/bin/env python3
"""Ordinary fixture validation; the host separately binds its result to candidate bytes."""
import argparse, json, runpy, sys
from pathlib import Path
parser = argparse.ArgumentParser()
parser.add_argument("--result", required=True)
args = parser.parse_args()
sys.path.insert(0, str(Path.cwd()))
try:
    runpy.run_module("tests.check_limit", run_name="__main__")
    status = "passed"
except (Exception, SystemExit):
    status = "failed"
Path(args.result).write_text(json.dumps({"status": status, "suite": "boundary"}) + "\n")
raise SystemExit(0 if status == "passed" else 1)
