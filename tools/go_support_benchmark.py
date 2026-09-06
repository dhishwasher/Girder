#!/usr/bin/env python3
"""Run the frozen Go support policy against its pinned corpora."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Mapping, Sequence

try:
    from tools.core_representative_benchmark import (
        acquire_artifact,
        extract_archive,
        median,
        parse_peak_rss,
        sha256_file,
        source_inventory,
        verify_inventory,
    )
    from tools.harness_support import run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from core_representative_benchmark import (
        acquire_artifact,
        extract_archive,
        median,
        parse_peak_rss,
        sha256_file,
        source_inventory,
        verify_inventory,
    )
    from harness_support import run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_POLICY = REPO_ROOT / "docs" / "go-support-policy.json"
DEFAULT_CACHE = REPO_ROOT / ".benchmark-cache" / "go-support-v1"
DEFAULT_OUTPUT = REPO_ROOT / "docs" / "go-support-observation.json"
MEASURE_PROCESS = REPO_ROOT / "tools" / "measure_process.py"
MAX_GRAPH_OUTPUT_BYTES = 128 * 1024 * 1024


def has_is_test(node: Mapping[str, Any]) -> bool:
    return ["is_test", "true"] in node.get("attributes", [])


def evaluate_cases(
    repository: Mapping[str, Any],
    nodes: Sequence[Mapping[str, Any]],
    edges: Sequence[Mapping[str, Any]],
) -> list[Mapping[str, Any]]:
    """Classify edge and is_test probes while exposing missing endpoints."""
    node_index = {node.get("path"): node for node in nodes}
    edge_set = {
        (edge.get("source"), edge.get("target"), edge.get("kind")) for edge in edges
    }
    results = []
    for case in repository["semantic_cases"]:
        source = case["source"]
        target = case["target"]
        source_present = source in node_index
        target_present = target in node_index
        endpoints_present = source_present and target_present
        if case["kind"] == "IsTest":
            observed = endpoints_present and source == target and has_is_test(node_index[source])
        else:
            observed = endpoints_present and (source, target, case["kind"]) in edge_set
        expected = case["expected_present"]
        classification = (
            "tp" if expected and observed else "fn" if expected else "fp" if observed else "tn"
        )
        results.append(
            {
                "id": case["id"],
                "kind": case["kind"],
                "source": source,
                "target": target,
                "expected_present": expected,
                "observed_present": observed,
                "source_endpoint_present": source_present,
                "target_endpoint_present": target_present,
                "classification": classification,
            }
        )
    return results


def summarize_cases(cases: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
    totals = {label: 0 for label in ("tp", "fp", "fn", "tn")}
    for case in cases:
        totals[case["classification"]] += 1
    predicted = totals["tp"] + totals["fp"]
    actual = totals["tp"] + totals["fn"]
    return {
        **totals,
        "missing_endpoints": sum(
            not case["source_endpoint_present"] or not case["target_endpoint_present"]
            for case in cases
        ),
        "precision": round(totals["tp"] / predicted, 6) if predicted else 1.0,
        "recall": round(totals["tp"] / actual, 6) if actual else 1.0,
    }


def summarize_repository(runs: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
    cases = runs[0]["semantic_cases"]
    if any(run["semantic_cases"] != cases for run in runs[1:]):
        raise RuntimeError("semantic case outcomes changed across identical runs")
    walls = sorted(run["wall_ms"] for run in runs)
    rss = sorted(run["peak_rss_kib"] for run in runs)
    return {
        "wall_median_ms": median(walls),
        "wall_max_ms": max(walls),
        "peak_rss_median_kib": median(rss),
        "peak_rss_max_kib": max(rss),
        "artifact_unique_digests": len(
            {run["graph"]["artifact_sha256"] for run in runs}
        ),
        "semantic_unique_digests": len(
            {run["graph"]["canonical_semantic_sha256"] for run in runs}
        ),
        "unique_node_edge_count_pairs": len(
            {(run["graph"]["nodes"], run["graph"]["edges"]) for run in runs}
        ),
        "semantic": summarize_cases(cases),
    }


def aggregate_semantics(repositories: Sequence[Mapping[str, Any]]) -> Mapping[str, Any]:
    totals = {label: 0 for label in ("tp", "fp", "fn", "tn")}
    precisions = []
    recalls = []
    missing_endpoints = 0
    for repository in repositories:
        semantic = repository["summary"]["semantic"]
        for label in totals:
            totals[label] += semantic[label]
        missing_endpoints += semantic["missing_endpoints"]
        precisions.append(semantic["precision"])
        recalls.append(semantic["recall"])
    predicted = totals["tp"] + totals["fp"]
    actual = totals["tp"] + totals["fn"]
    aggregate = {
        **totals,
        "missing_endpoints": missing_endpoints,
        "micro_precision": round(totals["tp"] / predicted, 6) if predicted else 1.0,
        "micro_recall": round(totals["tp"] / actual, 6) if actual else 1.0,
        "macro_precision": round(sum(precisions) / len(precisions), 6),
        "macro_recall": round(sum(recalls) / len(recalls), 6),
    }
    return {"aggregate": aggregate, "languages": {"go": dict(aggregate)}}


def run_repository_once(
    driver: Path,
    inspector: Path,
    project: Path,
    repository: Mapping[str, Any],
    inventory: Any,
    run_directory: Path,
    ordinal: int,
    *,
    analyze_timeout_seconds: float,
    inspect_timeout_seconds: float,
    rayon_threads: int,
) -> Mapping[str, Any]:
    graph_path = run_directory / f"{repository['id']}-{ordinal}.aetherb"
    usage_path = run_directory / f"{repository['id']}-{ordinal}-usage.json"
    analyzed = run_bounded(
        (
            sys.executable,
            str(MEASURE_PROCESS),
            str(usage_path),
            "--",
            str(driver),
            str(project),
            str(graph_path),
        ),
        cwd=REPO_ROOT,
        env={**os.environ, "RAYON_NUM_THREADS": str(rayon_threads)},
        timeout_seconds=analyze_timeout_seconds,
        max_output_bytes=1024 * 1024,
    )
    summary = json.loads(analyzed.stdout)
    if summary.get("schema_version") != 1 or summary.get("source_files") != inventory.files:
        raise RuntimeError(f"{repository['id']} driver summary disagreed with inventory")
    inspected = run_bounded(
        (str(inspector), "inspect", str(graph_path), "--json"),
        cwd=REPO_ROOT,
        env={**os.environ, "RAYON_NUM_THREADS": str(rayon_threads)},
        timeout_seconds=inspect_timeout_seconds,
        max_output_bytes=MAX_GRAPH_OUTPUT_BYTES,
    )
    exported = json.loads(inspected.stdout)
    nodes = exported.get("nodes")
    edges = exported.get("edges")
    if exported.get("schema_version") != 1 or not isinstance(nodes, list) or not isinstance(edges, list):
        raise RuntimeError(f"{repository['id']} inspect output is invalid")
    if summary.get("nodes") != len(nodes) or summary.get("edges") != len(edges):
        raise RuntimeError(f"{repository['id']} driver and inspector graph counts disagree")
    after = source_inventory(project, set(repository["sources"]["extensions"]))
    if after != inventory:
        raise RuntimeError(f"{repository['id']} source tree changed during measurement")
    canonical = json.dumps(
        exported, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    edge_counts: dict[str, int] = {}
    for edge in edges:
        kind = edge.get("kind")
        if not isinstance(kind, str):
            raise RuntimeError(f"{repository['id']} contains an invalid edge")
        edge_counts[kind] = edge_counts.get(kind, 0) + 1
    return {
        "ordinal": ordinal,
        "wall_ms": round(analyzed.wall_seconds * 1000),
        "peak_rss_kib": parse_peak_rss(usage_path),
        "inspect_wall_ms": round(inspected.wall_seconds * 1000),
        "source_manifest_after_sha256": after.manifest_sha256,
        "graph": {
            "nodes": len(nodes),
            "edges": len(edges),
            "edges_by_kind": dict(sorted(edge_counts.items())),
            "artifact_bytes": graph_path.stat().st_size,
            "artifact_sha256": sha256_file(graph_path),
            "canonical_semantic_sha256": hashlib.sha256(canonical).hexdigest(),
        },
        "semantic_cases": evaluate_cases(repository, nodes, edges),
    }


def evaluate_policy(
    repositories: Sequence[Mapping[str, Any]],
    semantics: Mapping[str, Any],
    policy: Mapping[str, Any],
) -> Mapping[str, Any]:
    checks: list[Mapping[str, Any]] = []

    def maximum(check_id: str, actual: int | float, limit: int | float) -> None:
        checks.append({"id": check_id, "passed": actual <= limit, "actual": actual, "maximum": limit})

    def minimum(check_id: str, actual: int | float, limit: int | float) -> None:
        checks.append({"id": check_id, "passed": actual >= limit, "actual": actual, "minimum": limit})

    semantic_policy = policy["semantic"]
    for scope, result in (
        ("aggregate", semantics["aggregate"]),
        ("go", semantics["languages"]["go"]),
    ):
        maximum(f"semantic.{scope}.missing_endpoints", result["missing_endpoints"], 0)
        maximum(
            f"semantic.{scope}.false_positives",
            result["fp"],
            semantic_policy["false_positives_max"],
        )
        maximum(
            f"semantic.{scope}.false_negatives",
            result["fn"],
            semantic_policy["false_negatives_max"],
        )
        for metric in ("micro_precision", "micro_recall", "macro_precision", "macro_recall"):
            minimum(
                f"semantic.{scope}.{metric}",
                result[metric],
                semantic_policy[f"{metric}_min"],
            )

    defaults = policy["performance"]["default"]
    determinism = policy["determinism"]
    for repository in repositories:
        repository_id = repository["id"]
        summary = repository["summary"]
        maximum(
            f"performance.{repository_id}.wall_max_ms",
            summary["wall_max_ms"],
            defaults["analyze_max_ms"],
        )
        maximum(
            f"performance.{repository_id}.wall_median_ms",
            summary["wall_median_ms"],
            defaults["analyze_median_max_ms"],
        )
        maximum(
            f"performance.{repository_id}.peak_rss_max_kib",
            summary["peak_rss_max_kib"],
            defaults["rss_max_kib"],
        )
        maximum(
            f"determinism.{repository_id}.artifact_unique_digests",
            summary["artifact_unique_digests"],
            determinism["artifact_unique_digests_max"],
        )
        maximum(
            f"determinism.{repository_id}.semantic_unique_digests",
            summary["semantic_unique_digests"],
            determinism["semantic_unique_digests_max"],
        )
        maximum(
            f"determinism.{repository_id}.unique_node_edge_count_pairs",
            summary["unique_node_edge_count_pairs"],
            determinism["unique_node_edge_count_pairs_max"],
        )
        for run in repository["runs"]:
            maximum(
                f"performance.{repository_id}.run_{run['ordinal']}.artifact_bytes",
                run["graph"]["artifact_bytes"],
                policy["performance"]["graph_artifact_max_bytes"],
            )
            maximum(
                f"performance.{repository_id}.run_{run['ordinal']}.inspect_ms",
                run["inspect_wall_ms"],
                policy["performance"]["inspect_max_ms"],
            )
    maximum(
        "performance.sum_of_repository_medians_ms",
        sum(repository["summary"]["wall_median_ms"] for repository in repositories),
        policy["performance"]["sum_of_repository_medians_max_ms"],
    )
    return {"passed": all(check["passed"] for check in checks), "checks": checks}


def benchmark(args: argparse.Namespace) -> Mapping[str, Any]:
    policy = json.loads(args.policy.read_text(encoding="utf-8"))
    runs = policy["eligibility"]["runs_per_repository"]
    rayon_threads = policy["eligibility"]["rayon_threads"]
    repositories = []
    with tempfile.TemporaryDirectory(prefix="bitcode-go-") as temporary_name:
        temporary = Path(temporary_name)
        for repository in policy["corpus"]:
            archive = acquire_artifact(
                repository["artifact"],
                args.cache,
                offline=args.offline,
                timeout_seconds=args.download_timeout,
            )
            project = extract_archive(
                archive,
                temporary / repository["id"],
                repository["artifact"]["root"],
            )
            inventory = source_inventory(project, set(repository["sources"]["extensions"]))
            verify_inventory(repository, inventory)
            measured_runs = [
                run_repository_once(
                    args.driver.resolve(),
                    args.inspector.resolve(),
                    project,
                    repository,
                    inventory,
                    temporary,
                    ordinal,
                    analyze_timeout_seconds=policy["performance"]["default"]["analyze_max_ms"] / 1000,
                    inspect_timeout_seconds=policy["performance"]["inspect_max_ms"] / 1000,
                    rayon_threads=rayon_threads,
                )
                for ordinal in range(1, runs + 1)
            ]
            repositories.append(
                {
                    "id": repository["id"],
                    "role": repository["role"],
                    "language": repository["language"],
                    "artifact": repository["artifact"],
                    "source_inventory": {
                        "files": inventory.files,
                        "bytes": inventory.bytes,
                        "physical_lines": inventory.physical_lines,
                        "manifest_sha256": inventory.manifest_sha256,
                    },
                    "runs": measured_runs,
                    "summary": summarize_repository(measured_runs),
                }
            )
    semantics = aggregate_semantics(repositories)
    assessment = evaluate_policy(repositories, semantics, policy)
    commit = subprocess.run(
        ("git", "rev-parse", "HEAD"),
        cwd=REPO_ROOT,
        check=True,
        text=True,
        capture_output=True,
    ).stdout.strip()
    return {
        "schema_version": 1,
        "kind": "go-support-observation",
        "suite_id": policy["policy_id"],
        "policy_sha256": sha256_file(args.policy),
        "implementation_commit": commit,
        "tool": {
            "driver": str(args.driver),
            "driver_sha256": sha256_file(args.driver),
            "inspector": str(args.inspector),
            "inspector_sha256": sha256_file(args.inspector),
        },
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "run_definition": {
            "runs_per_repository": runs,
            "rayon_threads": rayon_threads,
            "offline": args.offline,
        },
        "repositories": repositories,
        "semantic_summary": semantics,
        "assessment": assessment,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--driver", type=Path, required=True)
    parser.add_argument("--inspector", type=Path, required=True)
    parser.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    parser.add_argument("--cache", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--download-timeout", type=float, default=60.0)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    result = benchmark(args)
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "suite_id": result["suite_id"],
                "passed": result["assessment"]["passed"],
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0 if result["assessment"]["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
