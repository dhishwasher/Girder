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

Every command failure, timeout, invalid JSON, or input-hash mismatch is
recorded as a failure in the written output (never an abort before output),
and makes the exit status nonzero.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Mapping

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT))

from tools.dispatch_corpus_scorer import UNSOUND_CELLS, cell_label, observed_class  # noqa: E402

MANIFEST = REPO_ROOT / "fixtures/typescript-structural-validation/v1/manifest.json"
PINNED_MANIFEST_SHA256 = "aa114b8e049b2c19e952af00cbb0107010c00b413621a73f3db0b36c6b916692"
DEFAULT_TIMEOUT_SECONDS = 120.0


class CommandFailure(Exception):
    """A girder invocation that cannot be scored; always recorded, never raised past a case."""


def run_json(binary: Path, args: list[str], timeout: float) -> Any:
    command = [str(binary), *args]
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise CommandFailure(f"timeout after {timeout}s: {' '.join(command)}") from None
    except OSError as error:
        raise CommandFailure(f"could not execute {command[0]}: {error}") from None
    if result.returncode != 0:
        raise CommandFailure(
            f"exit {result.returncode}: {' '.join(command)}: {result.stderr.strip()[:500]}"
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise CommandFailure(f"invalid JSON from {' '.join(command)}: {error}") from None


def names(binary: Path, project_dir: Path, bare: str, timeout: float) -> list[dict[str, str]]:
    document = run_json(binary, ["names", str(project_dir), bare, "--json"], timeout)
    if not isinstance(document, list) or not all(
        isinstance(c, dict) and isinstance(c.get("path"), str) for c in document
    ):
        raise CommandFailure(f"unexpected names output shape for {bare!r}")
    return document


def classified_selection(binary: Path, project_dir: Path, origin: str, timeout: float) -> dict[str, Any]:
    document = run_json(
        binary,
        ["test-impact", str(project_dir), "--quiet", "--classified", "--unbounded", origin],
        timeout,
    )
    if not isinstance(document, dict) or document.get("schema_version") != 1:
        raise CommandFailure(f"unsupported classified output: {str(document)[:200]!r}")
    selection: dict[str, Any] = {}
    for bucket in ("must", "may", "unknown"):
        paths = document.get(bucket, {}).get("paths") if isinstance(document.get(bucket), dict) else None
        if not isinstance(paths, list) or not all(isinstance(path, str) for path in paths):
            raise CommandFailure(f"unexpected test-impact output shape: {bucket}.paths is not a list of strings")
        selection[bucket] = set(paths)
    boundaries = document.get("boundaries")
    count = boundaries.get("count") if isinstance(boundaries, dict) else None
    if not isinstance(count, int) or isinstance(count, bool):
        raise CommandFailure("unexpected test-impact output shape: boundaries.count is not an integer")
    selection["boundary_count"] = count
    return selection


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


def score_case(binary: Path, root: Path, case: Mapping, timeout: float) -> dict[str, object]:
    project_dir = root / case["fixture_dir"]
    record: dict[str, object] = {"id": case["id"]}
    try:
        resolution = resolve_origin(case, names(binary, project_dir, case["origin"]["symbol"], timeout))
    except CommandFailure as error:
        return {**record, "status": "failed", "resolution_passed": False, "reason": str(error)}
    record.update(resolution)
    if not resolution["resolution_passed"]:
        return {**record, "status": "failed", "reason": "origin resolution contract not met"}
    if resolution["observed_resolution"] != "unique":
        # Refusal or ambiguity is the expected outcome; no test is scored.
        return {**record, "status": "resolution-only", "tests": []}
    origin_path = resolution["function_candidates"][0]
    try:
        selection = classified_selection(binary, project_dir, origin_path, timeout)
    except CommandFailure as error:
        return {**record, "status": "failed", "reason": str(error), "tests": []}
    tests = []
    for test in case["tests"]:
        try:
            found = [
                c["path"]
                for c in names(binary, project_dir, test["framework_id"], timeout)
                if c.get("kind") == "Function"
            ]
        except CommandFailure as error:
            tests.append({"test_id": test["id"], "status": "failed", "reason": str(error)})
            continue
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


def summarize(results: list[Mapping], input_errors: list[str] | None = None) -> dict[str, object]:
    cells = {k: 0 for k in ("exact", "conservative", "unsafe_exclusion", "overclaim", "failed")}
    for result in results:
        for test in result.get("tests", []):
            cells[test["cell"] if test["status"] == "scored" else "failed"] += 1
    failed_cases = [r["id"] for r in results if r["status"] == "failed"]
    unsound = sum(cells[c] for c in UNSOUND_CELLS)
    return {
        "cases": len(results),
        "resolution_passed": sum(1 for r in results if r.get("resolution_passed")),
        "failed_cases": failed_cases,
        "input_errors": list(input_errors or []),
        "cells": cells,
        "unsound_cells": unsound,
        "passed": not failed_cases and not input_errors and cells["failed"] == 0 and unsound == 0,
    }


def check_inputs(manifest_path: Path, pinned: str) -> tuple[dict | None, list[str], str | None]:
    errors: list[str] = []
    try:
        raw = manifest_path.read_bytes()
    except OSError as error:
        return None, [f"cannot read manifest: {error}"], None
    digest = hashlib.sha256(raw).hexdigest()
    if digest != pinned:
        # Never dereference an unpinned manifest: its shape is untrusted.
        return None, [f"manifest hash {digest} != pinned {pinned}"], digest
    try:
        manifest = json.loads(raw)
        root = REPO_ROOT / manifest["fixtures_root"]
        cases = [(case, root / case["fixture_dir"] / "app.test.ts", case["fixture_sha256"]) for case in manifest["cases"]]
    except (json.JSONDecodeError, KeyError, TypeError, AttributeError) as error:
        return None, [f"invalid manifest: {error!r}"], digest
    for _case, fixture, expected in cases:
        try:
            actual = hashlib.sha256(fixture.read_bytes()).hexdigest()
        except OSError as error:
            errors.append(f"cannot read {fixture}: {error}")
            continue
        if actual != expected:
            errors.append(f"fixture hash mismatch: {fixture}")
    return manifest, errors, digest


def main(argv: list[str] | None = None, manifest_path: Path = MANIFEST, pinned: str = PINNED_MANIFEST_SHA256) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=DEFAULT_TIMEOUT_SECONDS)
    args = parser.parse_args(argv)
    manifest, input_errors, digest = check_inputs(manifest_path, pinned)
    results: list[dict[str, object]] = []
    if manifest is not None and not input_errors:
        root = REPO_ROOT / manifest["fixtures_root"]
        for case in manifest["cases"]:
            try:
                results.append(score_case(args.binary, root, case, args.timeout))
            except Exception as error:  # noqa: BLE001 - last resort: persist, never abort
                results.append(
                    {"id": case.get("id"), "status": "failed", "reason": f"internal scorer error: {error!r}"}
                )
    try:
        binary_sha256 = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    except OSError as error:
        binary_sha256 = None
        input_errors.append(f"cannot read binary: {error}")
    document = {
        "manifest_sha256": digest,
        "binary": str(args.binary),
        "binary_sha256": binary_sha256,
        "summary": summarize(results, input_errors),
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, indent=2) + "\n")
    print(json.dumps(document["summary"]))
    return 0 if document["summary"]["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
