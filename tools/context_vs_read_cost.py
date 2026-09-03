#!/usr/bin/env python3
"""Byte cost of retrieving one function three ways: read the file, run
`bitcode context`, or run `bitcode context --source-only`.

CLAUDE.md told agents to prefer `bitcode context` over reading a file and
cited "measured 41% fewer tokens". That number came from
`docs/authoring-cost.md` -- plan-authoring prompt tokens under a policy that
failed overall -- and never measured `bitcode context` against a file read at
all. This harness measures the claim that was being made.

Unlike `docs/names-cost.md` and `docs/context-with-tests-cost.md`, whose
numbers were hand-recorded with no script to regenerate them, this
measurement is reproducible:

    python3 tools/context_vs_read_cost.py --bitcode target/debug/bitcode \\
        --output docs/context-vs-read-cost-observation.json

The policy in `docs/context-vs-read-cost-policy.json` pins the node list and
the 40% threshold before measurement. This harness only gates the
`source_only` arm; the `context` arm is reported either way, including nodes
where it costs *more* than reading the file.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Mapping, Sequence

try:
    from tools.harness_support import run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "context-vs-read-cost-policy.json"
DEFAULT_OUTPUT = REPO_ROOT / "docs" / "context-vs-read-cost-observation.json"
DEFAULT_COMMAND_TIMEOUT_SECONDS = 300.0
# The authoring arm's envelope plus a large node's source stays well inside
# this; a run that trips it is a bug worth failing on, not truncating.
DEFAULT_MAX_OUTPUT_BYTES = 8 * 1024 * 1024


def reduction_ratio(before: int, after: int) -> float:
    """Fraction of `before` bytes saved by `after`.

    Negative when `after` is larger, which is a real outcome for a small
    function in a small file: the authoring envelope is a fixed cost that
    can exceed the whole file it replaces.
    """

    if before <= 0:
        raise ValueError(f"before must be positive, got {before}")
    return (before - after) / before


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


def node_file_map(bitcode: Path, root: Path) -> Mapping[str, str]:
    """Map every graph node path to the source file it was extracted from.

    Built by saving a graph with `analyze` and exporting it with `inspect`,
    which is the only command that reports a node's `file`.
    """

    analyze = run_bounded(
        (str(bitcode), "analyze", str(root), "--json"),
        cwd=root,
        timeout_seconds=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    )
    graph_path = json.loads(analyze.stdout)["graph_path"]

    exported = run_bounded(
        (str(bitcode), "inspect", graph_path, "--json"),
        cwd=root,
        timeout_seconds=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    )
    graph = json.loads(exported.stdout)
    return {
        node["path"]: node["file"]
        for node in graph["nodes"]
        if node.get("file")
    }


def stdout_bytes(bitcode: Path, root: Path, node_path: str, *, source_only: bool) -> int:
    """Raw stdout byte count of one `bitcode context` invocation."""

    command = [str(bitcode), "context", str(root), "--nodes", node_path, "--json"]
    if source_only:
        command.append("--source-only")
    result = run_bounded(
        tuple(command),
        cwd=root,
        timeout_seconds=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        max_output_bytes=DEFAULT_MAX_OUTPUT_BYTES,
    )
    return len(result.stdout.encode("utf-8"))


def measure(bitcode: Path, root: Path, node_paths: Sequence[str]) -> list[dict]:
    """One record per pinned node, in policy order."""

    files = node_file_map(bitcode, root)
    records = []
    for node_path in node_paths:
        source_file = files.get(node_path)
        if source_file is None:
            raise SystemExit(
                f"node pinned by the policy is absent from the graph: {node_path}"
            )
        read_bytes = (root / source_file).stat().st_size
        context_bytes = stdout_bytes(bitcode, root, node_path, source_only=False)
        source_only_bytes = stdout_bytes(bitcode, root, node_path, source_only=True)
        records.append(
            {
                "node": node_path,
                "file": source_file,
                "read_bytes": read_bytes,
                "context_bytes": context_bytes,
                "source_only_bytes": source_only_bytes,
                "context_reduction_ratio": reduction_ratio(read_bytes, context_bytes),
                "source_only_reduction_ratio": reduction_ratio(
                    read_bytes, source_only_bytes
                ),
            }
        )
    return records


def summarize(records: Sequence[Mapping], min_reduction_ratio: float) -> dict:
    """Aggregate the per-node records and apply the gated threshold."""

    read_total = sum(record["read_bytes"] for record in records)
    context_total = sum(record["context_bytes"] for record in records)
    source_only_total = sum(record["source_only_bytes"] for record in records)
    gated = reduction_ratio(read_total, source_only_total)
    return {
        "nodes_measured": len(records),
        "read_bytes_total": read_total,
        "context_bytes_total": context_total,
        "source_only_bytes_total": source_only_total,
        "context_reduction_ratio": reduction_ratio(read_total, context_total),
        "source_only_reduction_ratio": gated,
        "min_reduction_ratio": min_reduction_ratio,
        "nodes_where_context_costs_more_than_reading": [
            record["node"] for record in records if record["context_bytes"] > record["read_bytes"]
        ],
        "nodes_where_source_only_costs_more_than_reading": [
            record["node"]
            for record in records
            if record["source_only_bytes"] > record["read_bytes"]
        ],
        "policy_result": "PASS" if gated >= min_reduction_ratio else "FAIL",
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bitcode",
        type=Path,
        required=True,
        help="path to the bitcode binary under measurement",
    )
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="project to measure")
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args(argv)

    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        raise SystemExit(f"no bitcode binary at {bitcode}")
    root = args.root.resolve()

    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    node_paths = policy["node_selection"]["nodes"]
    min_reduction_ratio = policy["threshold"]["min_reduction_ratio"]

    records = measure(bitcode, root, node_paths)
    summary = summarize(records, min_reduction_ratio)

    observed_commit = head_commit(root)
    observation = {
        "schema_version": 1,
        "policy_id": policy["policy_id"],
        "policy_source_commit": policy["source_commit"],
        "observed_commit": observed_commit,
        "source_commit_matches": observed_commit == policy["source_commit"],
        "bitcode": str(bitcode),
        "summary": summary,
        "nodes": records,
    }

    rendered = json.dumps(observation, indent=2) + "\n"
    if args.output is not None:
        args.output.write_text(rendered, encoding="utf-8")
    else:
        sys.stdout.write(rendered)

    print(
        f"read {summary['read_bytes_total']}B -> "
        f"context {summary['context_bytes_total']}B "
        f"({summary['context_reduction_ratio']:.2%}) -> "
        f"source-only {summary['source_only_bytes_total']}B "
        f"({summary['source_only_reduction_ratio']:.2%}): "
        f"{summary['policy_result']}",
        file=sys.stderr,
    )
    return 0 if summary["policy_result"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
