#!/usr/bin/env python3
"""Verify and runtime-check the Go direct-call proof fixtures (Stage 3, Go).

Checks every fixture file against its pinned sha256, then runs `go vet` and
`go test` on each module with the installed toolchain. Plain Go only; never runs
girder. cgo fixtures are skipped (recorded) when no C compiler is present.

  python3 -m tools.validate_go_proof_fixtures --output runtime-validation.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "fixtures" / "go-direct-call-proof" / "v1"


def run(cmd, cwd, env):
    p = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=180)
    return p.returncode, (p.stdout + p.stderr).strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--work", default=str(Path.home() / "girder-evidence" / "go-work"))
    args = parser.parse_args()
    manifest = json.loads((FIXTURES / "manifest.json").read_text())
    work = Path(args.work)
    env = dict(os.environ, GOCACHE=str(work / "cache"), GOPATH=str(work / "path"),
               GOFLAGS="-count=1", GO111MODULE="on", GOTOOLCHAIN="local")
    version = subprocess.run(["go", "version"], capture_output=True, text=True).stdout.strip()
    has_cc = shutil.which("gcc") is not None or shutil.which("cc") is not None
    results, failed = [], 0
    for case in manifest["cases"]:
        entry = {"id": case["id"], "expected": case["expected"]}
        base = ROOT / case["path"]
        for rel, digest in case["files"].items():
            actual = hashlib.sha256((base / rel).read_bytes()).hexdigest()
            if actual != digest:
                entry["status"] = "HASH-MISMATCH:" + rel
                failed += 1
                break
        else:
            if case["rule"] == "G1-refuse-cgo" and not has_cc:
                entry["status"] = "skipped-no-c-compiler"
            else:
                code_vet, out_vet = run(["go", "vet", "./..."], base, env)
                code_test, out_test = run(["go", "test", "./..."], base, env)
                entry["go_vet_exit"], entry["go_test_exit"] = code_vet, code_test
                entry["status"] = "ok" if code_vet == 0 and code_test == 0 else "FAILED"
                if entry["status"] != "ok":
                    entry["output"] = (out_vet + "\n" + out_test)[-600:]
                    failed += 1
        results.append(entry)
    report = {"go_version": version, "has_c_compiler": has_cc, "cases": results,
              "failed": failed, "total": len(results)}
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n")
    print(f"{len(results)} cases, {failed} failed ({version})")
    for r in results:
        if r["status"] != "ok":
            print(" ", r["id"], r["status"], r.get("output", "")[:200])
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
