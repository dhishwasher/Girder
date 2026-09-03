#!/usr/bin/env python3
"""Top-1/top-5 accuracy of `bitcode search` against a pinned description corpus.

`find_definition` with an exact symbol name works well; `get_source --intent`
and `search_code`/`bitcode search` -- the common case, since an agent usually
has a description and not an exact semantic path -- did not. Both routes
through the same `SemanticGraph::semantic_search`
(`crates/aether-graph/src/similarity.rs`), so measuring `bitcode search`'s
stdout is a faithful proxy for all three call sites.

The policy in `docs/description-search-accuracy-policy.json` pins the corpus
(descriptions paired with the semantic path a competent human would expect)
and the top-1/top-5 thresholds before measurement. Run it with
`--baseline` to record the pre-fix implementation for contrast (not gated);
without it, the result is gated against the policy threshold:

    python3 tools/description_search_accuracy.py --bitcode target/debug/bitcode \\
        --output docs/description-search-accuracy-observation.json
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Mapping, Sequence

try:
    from tools.harness_support import run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "description-search-accuracy-policy.json"
DEFAULT_OUTPUT = REPO_ROOT / "docs" / "description-search-accuracy-observation.json"
DEFAULT_COMMAND_TIMEOUT_SECONDS = 300.0
DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024

# `bitcode search` prints "  <score>  <path>" per hit, best first, e.g.
# "  0.42  crate::foo::bar". Anchored so a stray indented line elsewhere in
# stdout cannot be mistaken for a hit.
HIT_LINE = re.compile(r"^ {2}(-?\d+\.\d+) {2}(\S+)$")


def parse_hits(stdout: str) -> list[str]:
    """Node paths from `bitcode search` stdout, best hit first."""

    return [match.group(2) for line in stdout.splitlines() if (match := HIT_LINE.match(line))]


def rank_of(expected_path: str, hits: Sequence[str]) -> int | None:
    """1-based rank of `expected_path` in `hits`, or None if absent."""

    try:
        return hits.index(expected_path) + 1
    except ValueError:
        return None


def head_commit(root: Path) -> str | None:
    """The tree's current HEAD, or None if it is not a git repository."""

    try:
        completed = subprocess.run(
            ("git", "rev-parse", "HEAD"),
            cwd=root,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if completed.returncode != 0:
        return None
    return completed.stdout.strip() or None


def search(bitcode: Path, root: Path, description: str) -> list[str]:
    """Ranked node paths `bitcode search <root> "<description>"` returns."""

    result = run_bounded(
        (str(bitcode), "search", str(root), description),
        cwd=root,
        timeout_seconds=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    )
    return parse_hits(result.stdout)


def measure(bitcode: Path, root: Path, corpus: Sequence[Mapping]) -> list[dict]:
    """One record per corpus item, in policy order."""

    records = []
    for item in corpus:
        expected = item["expected_path"]
        hits = search(bitcode, root, item["description"])
        rank = rank_of(expected, hits)
        records.append(
            {
                "description": item["description"],
                "expected_path": expected,
                "hits": hits,
                "rank_of_expected": rank,
                "top1_correct": rank == 1,
                "top5_correct": rank is not None and rank <= 5,
            }
        )
    return records


def summarize(records: Sequence[Mapping], threshold: Mapping[str, float] | None) -> dict:
    """Aggregate accuracy over the corpus. `threshold=None` reports without gating."""

    total = len(records)
    if total == 0:
        raise ValueError("corpus must not be empty")
    top1 = sum(1 for record in records if record["top1_correct"])
    top5 = sum(1 for record in records if record["top5_correct"])
    top1_accuracy = top1 / total
    top5_accuracy = top5 / total
    summary = {
        "items_measured": total,
        "top1_accuracy": top1_accuracy,
        "top5_accuracy": top5_accuracy,
        "top1_misses": [
            record["description"] for record in records if not record["top1_correct"]
        ],
        "top5_misses": [
            record["description"] for record in records if not record["top5_correct"]
        ],
    }
    if threshold is None:
        summary["gated"] = False
        summary["policy_result"] = "BASELINE"
        return summary
    summary["gated"] = True
    summary["min_top1_accuracy"] = threshold["min_top1_accuracy"]
    summary["min_top5_accuracy"] = threshold["min_top5_accuracy"]
    passed = (
        top1_accuracy >= threshold["min_top1_accuracy"]
        and top5_accuracy >= threshold["min_top5_accuracy"]
    )
    summary["policy_result"] = "PASS" if passed else "FAIL"
    return summary


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bitcode",
        type=Path,
        required=True,
        help="path to the bitcode binary under measurement",
    )
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="project to search")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument(
        "--baseline",
        action="store_true",
        help="record accuracy without gating against the policy threshold",
    )
    args = parser.parse_args(argv)

    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        raise SystemExit(f"no bitcode binary at {bitcode}")
    root = args.root.resolve()

    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    corpus = policy["corpus"]
    threshold = None if args.baseline else policy["threshold"]

    records = measure(bitcode, root, corpus)
    summary = summarize(records, threshold)

    observed_commit = head_commit(root)
    observation = {
        "schema_version": 1,
        "policy_id": policy["policy_id"],
        "policy_source_commit": policy["source_commit"],
        "observed_commit": observed_commit,
        "source_commit_matches": observed_commit == policy["source_commit"],
        "bitcode": str(bitcode),
        "summary": summary,
        "items": records,
    }

    rendered = json.dumps(observation, indent=2) + "\n"
    if args.output is not None:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)

    print(
        f"top-1 {summary['top1_accuracy']:.2%} "
        f"top-5 {summary['top5_accuracy']:.2%} "
        f"over {summary['items_measured']} items: {summary['policy_result']}",
        file=sys.stderr,
    )
    return 0 if summary["policy_result"] in ("PASS", "BASELINE") else 1


if __name__ == "__main__":
    raise SystemExit(main())
