#!/usr/bin/env python3
"""Round-trip and byte cost of the composite `orient` MCP tool against the
chained baseline it replaces (`get_source` + `ask_codebase` for callers,
callees, and impact + `impacted_tests`), on the task corpus pinned in
`docs/orient-tool-policy.json`.

Run once, per CLAUDE.md and the policy's own method: this script is not
re-run to "improve" a recorded result. Reproduce with:

    cargo build -p aether-app --target-dir <target-dir>
    python3 tools/orient_benchmark.py --girder <target-dir>/debug/girder \\
        --output docs/orient-tool-observation.json

Every pinned repository this corpus references is already cached (offline)
by the core-representative-v1, typescript-support-v1, and go-support-v1
benchmarks; this script extracts fresh, disposable copies from those caches
and never re-downloads or re-pins anything.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any, Mapping, MutableMapping, Sequence

try:
    from tools.harness_support import run_bounded, BoundedProcessResult
    from tools.core_representative_benchmark import acquire_artifact, extract_archive
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import run_bounded, BoundedProcessResult
    from core_representative_benchmark import acquire_artifact, extract_archive


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "orient-tool-policy.json"
DEFAULT_OUTPUT = REPO_ROOT / "docs" / "orient-tool-observation.json"
COMMAND_TIMEOUT_SECONDS = 300.0
MAX_OUTPUT_BYTES = 16 * 1024 * 1024
ACQUIRE_TIMEOUT_SECONDS = 60.0

# Where each corpus repository's pinned artifact is declared. Every one of
# these repositories is already extracted at least once by its own
# benchmark, so `offline=True` below never triggers a network fetch.
REPO_SOURCE_FILES = {
    "docs/core-representative-corpus.json": (
        "repositories",
        ".benchmark-cache/core-representative-v1",
    ),
    "docs/typescript-support-policy.json": (
        "corpus",
        ".benchmark-cache/typescript-support-v1",
    ),
    "docs/go-support-policy.json": (
        "corpus",
        ".benchmark-cache/go-support-v1",
    ),
}

BULLET_LINE = re.compile(r"^\s*·\s+(.*)$", re.MULTILINE)
DISTANCE_SUFFIX = re.compile(r"\s+\(distance \d+\)$")


def load_repo_registry() -> Mapping[str, Mapping[str, Any]]:
    """Map each pinned repository id to its `artifact` block and cache dir,
    read from whichever of the three already-pinned corpus files declares
    it. No repository is re-pinned here."""

    registry: MutableMapping[str, Mapping[str, Any]] = {}
    for relative, (list_key, cache_dir) in REPO_SOURCE_FILES.items():
        data = json.loads((REPO_ROOT / relative).read_text(encoding="utf-8"))
        for repo in data[list_key]:
            registry[repo["id"]] = {
                "artifact": repo["artifact"],
                "cache_dir": REPO_ROOT / cache_dir,
            }
    return registry


def extract_repo(repo_id: str, registry: Mapping[str, Mapping[str, Any]], work_dir: Path) -> Path:
    entry = registry[repo_id]
    archive = acquire_artifact(
        entry["artifact"], entry["cache_dir"], offline=True, timeout_seconds=ACQUIRE_TIMEOUT_SECONDS
    )
    destination = work_dir / repo_id
    return extract_archive(archive, destination, entry["artifact"]["root"])


def build_baseline_argv(tool: str, arguments: Mapping[str, Any], root: Path) -> list[str]:
    """Mirror each MCP tool's `argv` builder in
    `crates/aether-app/src/project/commands/mcp.rs` exactly, so a baseline
    call costs precisely what that tool costs over MCP."""

    if tool == "get_source":
        argv = ["context", str(root), "--json", "--source-only"]
        nodes = arguments.get("nodes")
        if nodes:
            argv += ["--nodes", ",".join(nodes)]
        else:
            argv += [arguments["intent"]]
        return argv
    if tool == "ask_codebase":
        return ["query", str(root), arguments["question"]]
    if tool == "impacted_tests":
        return ["test-impact", str(root), "--quiet", *arguments.get("nodes", [])]
    if tool == "search_code":
        return ["search", str(root), arguments["query"]]
    raise ValueError(f"unrecognized baseline tool: {tool}")


def build_orient_argv(starting_point: Mapping[str, Any], root: Path) -> list[str]:
    argv = ["orient", str(root), "--json"]
    if starting_point["kind"] == "symbol":
        argv += ["--nodes", starting_point["value"]]
    else:
        argv += [starting_point["value"]]
    return argv


def run(girder: Path, argv: Sequence[str], root: Path) -> BoundedProcessResult:
    return run_bounded(
        (str(girder), *argv),
        cwd=root,
        timeout_seconds=COMMAND_TIMEOUT_SECONDS,
        max_output_bytes=MAX_OUTPUT_BYTES,
        check=True,
    )


def parse_bullet_paths(stdout: str) -> set[str]:
    """Extract the plain node path from every `  · <line>` result line
    `QueryResult::display()` prints. Callers/callees lines are the bare
    path; impact lines carry a trailing ` (distance N)` this strips."""

    paths = set()
    for line in BULLET_LINE.findall(stdout):
        paths.add(DISTANCE_SUFFIX.sub("", line.strip()))
    return paths


def parse_test_names(stdout: str) -> set[str]:
    """`test-impact --quiet` prints one bare test name per line and nothing
    else -- no preamble, unlike its non-quiet mode."""

    return {line.strip() for line in stdout.splitlines() if line.strip()}


def node_last_segment(path: str) -> str:
    return path.rsplit("::", 1)[-1]


def measure_task(girder: Path, root: Path, task: Mapping[str, Any]) -> Mapping[str, Any]:
    baseline_bytes = 0
    per_call = []
    callers_baseline: set[str] | None = None
    callees_baseline: set[str] | None = None
    impact_baseline: set[str] | None = None
    tests_baseline: set[str] | None = None

    for call in task["baseline"]["tool_calls"]:
        argv = build_baseline_argv(call["tool"], call["arguments"], root)
        result = run(girder, argv, root)
        n_bytes = len(result.stdout.encode("utf-8"))
        baseline_bytes += n_bytes
        per_call.append({"tool": call["tool"], "bytes": n_bytes})
        if call["tool"] == "ask_codebase":
            question = call["arguments"]["question"]
            paths = parse_bullet_paths(result.stdout)
            if question.startswith("what calls"):
                callers_baseline = paths
            elif question.startswith("what does"):
                callees_baseline = paths
            elif question.startswith("what would break"):
                impact_baseline = paths
        elif call["tool"] == "impacted_tests":
            tests_baseline = parse_test_names(result.stdout)

    round_trips_baseline = len(task["baseline"]["tool_calls"])

    starting_point = task["starting_point"]
    orient_argv = build_orient_argv(starting_point, root)
    orient_result = run(girder, orient_argv, root)
    composite_bytes = len(orient_result.stdout.encode("utf-8"))
    orient_output = json.loads(orient_result.stdout)
    node = orient_output["nodes"][0]

    record: MutableMapping[str, Any] = {
        "id": task["id"],
        "starting_point": starting_point,
        "round_trips_baseline": round_trips_baseline,
        "round_trips_composite": 1,
        "baseline_bytes": baseline_bytes,
        "composite_bytes": composite_bytes,
        "byte_ratio": composite_bytes / baseline_bytes,
        "baseline_calls": per_call,
        "orient_resolved_path": node["path"],
        "orient_confidence": node.get("confidence"),
        "orient_score": node.get("score"),
        "comparable": True,
    }

    if starting_point["kind"] == "intent":
        # By construction (see docs/orient-tool-policy.json), the second
        # baseline call is get_source pinned to the task's intended node.
        intended = task["baseline"]["tool_calls"][1]["arguments"]["nodes"][0]
        record["intended_path"] = intended
        record["resolution_matches_intended"] = node["path"] == intended
        if not record["resolution_matches_intended"]:
            # Baseline and composite are anchored to different nodes when
            # intent resolution missed: a byte-ratio or node-set comparison
            # would compare the cost/content of answers to two different
            # questions, so neither is gated for this task -- only whether
            # the resolution itself matched is (see
            # docs/orient-tool-policy.json's reported_but_not_gated).
            record["comparable"] = False
            record["correctness"] = None
            record["correctness_note"] = (
                "not applicable: orient resolved a different node than the "
                "baseline's hardcoded intended node, so their outputs are "
                "not comparable"
            )
            return record

    callers_ok, callers_detail = section_matches(node["callers"], callers_baseline)
    callees_ok, callees_detail = section_matches(node["callees"], callees_baseline)
    impact_ok, impact_detail = section_matches(node["impact"], impact_baseline)
    tests_ok, tests_detail = section_matches(node["tests"], tests_baseline, name_only=True)

    correctness = callers_ok and callees_ok and impact_ok and tests_ok
    record["correctness"] = correctness
    if not correctness:
        record["correctness_detail"] = {
            "callers": callers_detail,
            "callees": callees_detail,
            "impact": impact_detail,
            "tests": tests_detail,
        }
    return record


def section_matches(
    orient_section: Mapping[str, Any], baseline_set: set[str] | None, *, name_only: bool = False
) -> tuple[bool, Mapping[str, Any]]:
    """Compare one orient section against the baseline's node set for the
    same relationship.

    A section orient reports as `truncated` degraded deliberately (see
    `crate::project::commands::orient::section`): its `count` is still the
    true total, but `paths` lists only the first
    `MAX_LISTED_PER_SECTION` of them. That is not a correctness failure per
    `docs/orient-tool-policy.json`'s correctness clause -- only a listed
    path absent from the baseline set, or (for callers/callees/impact,
    where both sides key on the full semantic path) a `count` that
    disagrees with the baseline's true total, is.

    `name_only` (the `tests` section) is the one case where the baseline's
    own identity is coarser than orient's: `impacted_tests --quiet` prints
    a bare test *name*, which is not guaranteed unique across files, while
    orient counts distinct test *nodes*. A name collision between two
    different test files can make the baseline's distinct-name count
    legitimately smaller than orient's true node count with no node
    actually missing or invented, so a truncated `tests` section is
    checked by subset only, never by count equality.
    """

    baseline_set = baseline_set or set()
    paths = orient_section["paths"]
    values = {node_last_segment(p) for p in paths} if name_only else set(paths)
    if orient_section.get("truncated"):
        ok = values.issubset(baseline_set) and (
            name_only or orient_section["count"] == len(baseline_set)
        )
    else:
        ok = values == baseline_set
    detail = {
        "orient": sorted(values),
        "baseline": sorted(baseline_set),
        "orient_count": orient_section["count"],
        "truncated": orient_section.get("truncated", False),
    }
    return ok, detail


# Task ids the policy explicitly carves out of the gated byte-ratio check
# (see docs/orient-tool-policy.json's reported_but_not_gated): the two
# Go-corpus exact-symbol tasks, since go-support-v1's own observation
# recorded a missed Calls edge and this measurement does not re-verify it.
BYTE_RATIO_NOT_GATED = {"afero-readfile", "websocket-writejson"}


def build_checks(policy: Mapping[str, Any], records: Sequence[Mapping[str, Any]]) -> list[dict]:
    checks = []
    max_round_trips = policy["threshold"]["max_round_trips_for_composite"]
    max_byte_ratio = policy["threshold"]["max_byte_ratio_vs_baseline"]

    for record in records:
        task_id = record["id"]
        checks.append(
            {
                "id": f"{task_id}.round_trips",
                "actual": record["round_trips_composite"],
                "maximum": max_round_trips,
                "passed": record["round_trips_composite"] <= max_round_trips,
                "gated": True,
            }
        )
        byte_ratio_gated = task_id not in BYTE_RATIO_NOT_GATED and record["comparable"]
        checks.append(
            {
                "id": f"{task_id}.byte_ratio",
                "actual": record["byte_ratio"],
                "maximum": max_byte_ratio,
                "passed": record["byte_ratio"] <= max_byte_ratio,
                "gated": byte_ratio_gated,
            }
        )
        if record["correctness"] is None:
            checks.append(
                {
                    "id": f"{task_id}.correctness",
                    "actual": "not_applicable",
                    "passed": True,
                    "gated": False,
                }
            )
        else:
            checks.append(
                {
                    "id": f"{task_id}.correctness",
                    "actual": record["correctness"],
                    "passed": record["correctness"],
                    "gated": True,
                }
            )
    return checks


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--girder", type=Path, required=True)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args(argv)

    girder = args.girder.resolve()
    if not girder.is_file():
        raise SystemExit(f"no girder binary at {girder}")

    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    registry = load_repo_registry()

    work_dir = Path(tempfile.mkdtemp(prefix="orient-benchmark-"))
    extracted: dict[str, Path] = {}
    try:
        records = []
        for task in policy["corpus"]:
            repo_id = task["repo_ref"]["id"]
            if repo_id not in extracted:
                extracted[repo_id] = extract_repo(repo_id, registry, work_dir)
            root = extracted[repo_id]
            print(f"measuring {task['id']} ({repo_id}) ...", file=sys.stderr)
            records.append(measure_task(girder, root, task))
    finally:
        shutil.rmtree(work_dir, ignore_errors=True)

    checks = build_checks(policy, records)
    gated_checks = [c for c in checks if c["gated"]]
    overall_passed = all(c["passed"] for c in gated_checks)

    observation = {
        "schema_version": 1,
        "suite_id": policy["policy_id"],
        "kind": "orient-tool-observation",
        "policy_baseline_commit": policy["policy_baseline_commit"],
        "girder": str(girder),
        "tasks": records,
        "assessment": {"checks": checks, "passed": overall_passed},
    }

    rendered = json.dumps(observation, indent=2) + "\n"
    args.output.write_text(rendered, encoding="utf-8")

    failed = [c["id"] for c in gated_checks if not c["passed"]]
    print(
        f"{len(records)} task(s) measured; "
        f"{len(gated_checks) - len(failed)}/{len(gated_checks)} gated checks passed"
        + (f"; FAILED: {', '.join(failed)}" if failed else ""),
        file=sys.stderr,
    )
    return 0 if overall_passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
