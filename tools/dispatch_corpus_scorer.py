#!/usr/bin/env python3
"""Stage 2 dispatch-corpus scorer.

Scores Girder's classified test-impact answer against docs/dispatch-corpus.json's
precommitted expectations, using only the public surface:
`girder test-impact <case_dir> --quiet --classified --unbounded <origin_path>`.

Each test in each case gets an expected class (must/may/unknown/excluded, per
docs/call-classification-policy.md, or a true negative). The scorer resolves
origins and tests to graph paths via `girder names <dir> <identifier> --json`
(exact name match, not the similarity-ranked `search`, which can drop an
exact short identifier below its relevance floor). A symbol's origin may
carry an optional "qualifier" (e.g. a receiver type or enclosing scope) to
disambiguate multiple candidates sharing a bare name (e.g. two trait/
interface implementors both naming a method the same thing); if an origin
cannot be resolved (or is still ambiguous after any qualifier), that is a
recorded failure for the case, never a reason to edit it.

Confusion-matrix cell labels (see docs/roadmap.md Stage 2). "excluded" is
the STRONGEST possible claim (a test is provably unreachable), not the
weakest -- getting it wrong is the single worst failure this tool can make,
since the quiet CLI union (must|may|unknown) is exactly what an agent uses
to decide what to run; a test placed in none of the three buckets is
silently never run. must/may/unknown all land a test IN that union, so
which of those three Girder picks does not affect whether the test runs --
only how much reasoning it's given for why. A cell is therefore unsound only
when Girder's own label claims more than is true:
  - unsafe_exclusion: observed excluded, expected anything else -- a
    reachable test silently dropped from the union. The single worst case.
  - overclaim: observed must when expected is not must (false uniqueness --
    exactly what Stage 1's trustworthy-Must gate measures), OR observed may
    when expected is unknown or excluded (claims a complete, viable
    candidate set that provably doesn't exist).
  - conservative: every other non-exact cell -- Girder is equally or more
    inclusive than the truth requires; safe, just imprecise or unlabeled.
  - exact: Girder's answer matches the expected class exactly.
"""
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Mapping, Sequence

REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_PATH = REPO_ROOT / "docs" / "dispatch-corpus.json"

def run(binary: Path, args: Sequence[str]) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(binary), *args],
        capture_output=True,
        text=True,
        timeout=60,
    )


def resolve_symbol(
    binary: Path, project_dir: Path, symbol: str, qualifier: str | None = None
) -> str | list[str] | None:
    """Exact-name resolution via `girder names`. `symbol` may be a
    pytest-style `file.py::name` id; only the bare trailing identifier is
    looked up. Returns the single matching path, `None` if there were zero
    matches, or a list of paths if more than one candidate remains after
    applying `qualifier` (an ambiguity the caller must treat as a failure,
    not silently pick one of)."""
    bare = symbol.rsplit("::", 1)[-1]
    result = run(binary, ["names", str(project_dir), bare, "--json"])
    if result.returncode != 0:
        return None
    try:
        candidates = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None
    paths = [c["path"] for c in candidates]
    if qualifier:
        qualified = [p for p in paths if f"::{qualifier}::{bare}" in p or p == f"crate::{qualifier}::{bare}"]
        if qualified:
            paths = qualified
    if len(paths) == 1:
        return paths[0]
    if not paths:
        return None
    return paths


def classified_selection(binary: Path, project_dir: Path, origin_path: str) -> dict[str, set[str]]:
    result = run(
        binary,
        ["test-impact", str(project_dir), "--quiet", "--classified", "--unbounded", origin_path],
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"test-impact failed for {project_dir} origin={origin_path}: {result.stderr.strip()}"
        )
    document = json.loads(result.stdout)
    if document.get("schema_version") != 1:
        raise RuntimeError(f"unsupported classified schema_version: {document.get('schema_version')!r}")
    return {
        "must": set(document["must"]["paths"]),
        "may": set(document["may"]["paths"]),
        "unknown": set(document["unknown"]["paths"]),
        "boundary_count": document["boundaries"]["count"],
    }


def observed_class(selection: Mapping[str, set[str]], test_path: str) -> str:
    for bucket in ("must", "may", "unknown"):
        if test_path in selection[bucket]:
            return bucket
    return "excluded"


def cell_label(expected: str, observed: str) -> str:
    if expected == observed:
        return "exact"
    if observed == "excluded":
        return "unsafe_exclusion"
    if observed == "must":
        return "overclaim"
    if observed == "may" and expected in ("unknown", "excluded"):
        return "overclaim"
    return "conservative"


UNSOUND_CELLS = ("unsafe_exclusion", "overclaim")


def score_case(binary: Path, case: Mapping[str, object]) -> dict[str, object]:
    project_dir = REPO_ROOT / case["fixture_dir"]  # type: ignore[arg-type]
    origin = case["origin"]  # type: ignore[assignment]
    origin_path = resolve_symbol(
        binary, project_dir, origin["symbol"], origin.get("qualifier")  # type: ignore[index,union-attr]
    )
    if origin_path is None:
        return {
            "id": case["id"],
            "status": "failed",
            "reason": f"could not resolve origin symbol {origin['symbol']!r} in {project_dir}",  # type: ignore[index]
        }
    if isinstance(origin_path, list):
        return {
            "id": case["id"],
            "status": "failed",
            "reason": f"origin symbol {origin['symbol']!r} is ambiguous: {origin_path}",  # type: ignore[index]
        }
    try:
        selection = classified_selection(binary, project_dir, origin_path)
    except RuntimeError as error:
        return {"id": case["id"], "status": "failed", "reason": str(error)}

    test_results = []
    for test in case["tests"]:  # type: ignore[union-attr]
        test_path = resolve_symbol(binary, project_dir, test["framework_id"])
        if test_path is None or isinstance(test_path, list):
            test_results.append(
                {
                    "test_id": test["id"],
                    "status": "failed",
                    "reason": f"could not resolve test symbol {test['framework_id']!r} "
                    f"(result: {test_path!r})",
                }
            )
            continue
        observed = observed_class(selection, test_path)
        test_results.append(
            {
                "test_id": test["id"],
                "status": "scored",
                "origin_path": origin_path,
                "test_path": test_path,
                "expected": test["expected"],
                "observed": observed,
                "cell": cell_label(test["expected"], observed),
            }
        )
    return {
        "id": case["id"],
        "language": case["language"],
        "status": "scored",
        "origin_path": origin_path,
        "boundary_count": selection["boundary_count"],
        "tests": test_results,
    }


def confusion_matrix(results: Sequence[Mapping[str, object]]) -> dict[str, object]:
    cell_kinds = ("exact", "conservative", "unsafe_exclusion", "overclaim", "failed")
    per_language: dict[str, dict[str, int]] = {}
    pooled: dict[str, int] = {k: 0 for k in cell_kinds}
    # Must precision: of the cells where Girder observed "must", how many were
    # truly must (expected == must)? Any other expected class observed as
    # "must" is a false positive -- a false single-target claim, regardless
    # of whether the truth was may, unknown, or excluded.
    must_tp = must_fp = 0
    # Must-or-May recall: of the cells whose true class was must or may (a
    # genuinely reachable, classifiable call), how many did Girder land in
    # must or may (rather than flooding to unknown, which is safe but
    # uninformative)?
    must_or_may_positive = must_or_may_tp = 0
    for case in results:
        if case.get("status") != "scored":
            pooled["failed"] += 1
            continue
        lang = case["language"]
        per_language.setdefault(lang, {k: 0 for k in cell_kinds})
        for test in case["tests"]:  # type: ignore[union-attr]
            if test.get("status") != "scored":
                pooled["failed"] += 1
                per_language[lang]["failed"] += 1
                continue
            cell = test["cell"]
            pooled[cell] += 1
            per_language[lang][cell] += 1
            if test["observed"] == "must":
                if test["expected"] == "must":
                    must_tp += 1
                else:
                    must_fp += 1
            if test["expected"] in ("must", "may"):
                must_or_may_positive += 1
                if test["observed"] in ("must", "may"):
                    must_or_may_tp += 1
    total = sum(pooled.values())
    unsound_total = pooled["unsafe_exclusion"] + pooled["overclaim"]
    return {
        "pooled": pooled,
        "pooled_unsound_total": unsound_total,
        "per_language": per_language,
        "total_test_cells": total,
        "exact_match_rate": round(pooled["exact"] / total, 6) if total else None,
        "must_true_positives": must_tp,
        "must_false_positives": must_fp,
        "must_precision_on_corpus": round(must_tp / (must_tp + must_fp), 6)
        if (must_tp + must_fp)
        else None,
        "must_or_may_true_positives": must_or_may_tp,
        "must_or_may_positive_denominator": must_or_may_positive,
        "must_or_may_recall_on_corpus": round(must_or_may_tp / must_or_may_positive, 6)
        if must_or_may_positive
        else None,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=CORPUS_PATH)
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    if not args.bitcode.is_file():
        parser.error(f"binary does not exist: {args.bitcode}")

    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    results = [score_case(args.bitcode.resolve(), case) for case in corpus["cases"]]
    matrix = confusion_matrix(results)

    document = {
        "schema_version": 1,
        "corpus_schema_version": corpus["schema_version"],
        "case_count": len(corpus["cases"]),
        "results": results,
        "confusion_matrix": matrix,
    }

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
        print(f"wrote {args.output}")
    if args.json or not args.output:
        print(json.dumps(document, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
