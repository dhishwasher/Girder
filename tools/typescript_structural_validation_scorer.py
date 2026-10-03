#!/usr/bin/env python3
"""Separate scorer for the frozen TypeScript structural member validation cases.

`fixtures/typescript-structural-validation/v1/manifest.json` is never part of
docs/dispatch-corpus.json, its 49-case denominator, or any dispatch-corpus
improvement claim. Unlike tools/dispatch_corpus_scorer.py (unchanged), origin
resolution here checks node KIND and exact IDENTITY, not just names:

- Only `Function` nodes from `girder names --json` are executable origins.
  A Field/Type/signature node is recorded, never accepted (policy D1).
- A `unique` origin must equal `predicted_origin_path` exactly; an
  `ambiguous` one must equal `predicted_candidates` exactly. Several Function
  candidates are a recorded failure by design, never a choice of one.
- `no-identity` / `no-executable-target` require zero Function candidates.

Test cells use the dispatch scorer's exact/conservative/unsound rules.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Mapping

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))

from tools.dispatch_corpus_scorer import (  # noqa: E402
    UNSOUND_CELLS,
    cell_label,
    classified_selection,
    observed_class,
)

MANIFEST = REPO_ROOT / "fixtures/typescript-structural-validation/v1/manifest.json"
PINNED_MANIFEST_SHA256 = "aa114b8e049b2c19e952af00cbb0107010c00b413621a73f3db0b36c6b916692"


def names(binary: Path, project_dir: Path, bare: str) -> list[dict[str, str]]:
    result = subprocess.run(
        [str(binary), "names", str(project_dir), bare, "--json"],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if result.returncode != 0:
        raise RuntimeError(f"girder names failed: {result.stderr.strip()}")
    return json.loads(result.stdout)


def qualified(candidates: list[dict[str, str]], bare: str, qualifier: str | None) -> list[dict[str, str]]:
    if not qualifier:
        return candidates
    return [c for c in candidates if f"::{qualifier}::{bare}" in c["path"]]


def resolve_origin(case: Mapping, candidates: list[dict[str, str]]) -> dict[str, object]:
    origin = case["origin"]
    matches = qualified(candidates, origin["symbol"], origin.get("qualifier"))
    functions = sorted(c["path"] for c in matches if c.get("kind") == "Function")
    rejected = sorted(f"{c['path']} ({c.get('kind')})" for c in matches if c.get("kind") != "Function")
    observed = "unique" if len(functions) == 1 else "ambiguous" if functions else "none"
    expected = case["expected_origin_resolution"]
    if expected == "unique":
        passed = observed == "unique" and functions[0] == case["predicted_origin_path"]
    elif expected == "ambiguous":
        passed = observed == "ambiguous" and functions == sorted(case["predicted_candidates"])
    else:  # no-identity / no-executable-target
        passed = observed == "none"
    return {
        "expected_resolution": expected,
        "observed_resolution": observed,
        "function_candidates": functions,
        "rejected_non_function_candidates": rejected,
        "resolution_passed": passed,
    }


def score_case(binary: Path, root: Path, case: Mapping) -> dict[str, object]:
    project_dir = root / case["fixture_dir"]
    record: dict[str, object] = {"id": case["id"]}
    try:
        resolution = resolve_origin(case, names(binary, project_dir, case["origin"]["symbol"]))
    except RuntimeError as error:
        return {**record, "status": "failed", "reason": str(error)}
    record.update(resolution)
    if not resolution["resolution_passed"]:
        return {**record, "status": "failed", "reason": "origin resolution contract not met"}
    if resolution["observed_resolution"] != "unique":
        # Refusal or ambiguity is the expected outcome; no test is scored.
        return {**record, "status": "resolution-only", "tests": []}
    origin_path = resolution["function_candidates"][0]
    selection = classified_selection(binary, project_dir, origin_path)
    tests = []
    for test in case["tests"]:
        found = [
            c["path"]
            for c in names(binary, project_dir, test["framework_id"])
            if c.get("kind") == "Function"
        ]
        if len(found) != 1:
            tests.append({"test_id": test["id"], "status": "failed", "reason": f"test paths {found}"})
            continue
        observed = observed_class(selection, found[0])
        tests.append(
            {
                "test_id": test["id"],
                "status": "scored",
                "test_path": found[0],
                "expected": test["expected"],
                "observed": observed,
                "cell": cell_label(test["expected"], observed),
            }
        )
    return {**record, "status": "scored", "origin_path": origin_path, "tests": tests}


def summarize(results: list[Mapping]) -> dict[str, object]:
    cells = {k: 0 for k in ("exact", "conservative", "unsafe_exclusion", "overclaim", "failed")}
    for result in results:
        for test in result.get("tests", []):
            cells[test["cell"] if test["status"] == "scored" else "failed"] += 1
    failed_cases = [r["id"] for r in results if r["status"] == "failed"]
    return {
        "cases": len(results),
        "resolution_passed": sum(1 for r in results if r.get("resolution_passed")),
        "failed_cases": failed_cases,
        "cells": cells,
        "unsound_cells": sum(cells[c] for c in UNSOUND_CELLS),
        "passed": not failed_cases
        and cells["failed"] == 0
        and sum(cells[c] for c in UNSOUND_CELLS) == 0,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    raw = MANIFEST.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != PINNED_MANIFEST_SHA256:
        raise SystemExit(f"validation manifest hash {digest} != pinned {PINNED_MANIFEST_SHA256}")
    manifest = json.loads(raw)
    root = REPO_ROOT / manifest["fixtures_root"]
    for case in manifest["cases"]:
        fixture = root / case["fixture_dir"] / "app.test.ts"
        if hashlib.sha256(fixture.read_bytes()).hexdigest() != case["fixture_sha256"]:
            raise SystemExit(f"fixture hash mismatch: {fixture}")
    results = [score_case(args.binary, root, case) for case in manifest["cases"]]
    document = {
        "manifest_sha256": digest,
        "binary": str(args.binary),
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "summary": summarize(results),
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print(json.dumps(document["summary"]))
    return 0 if document["summary"]["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
