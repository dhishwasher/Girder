#!/usr/bin/env python3
"""Serial evaluator and artifact writer for one product-fixture campaign."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import platform
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Mapping

from .adapters.girder import GirderAdapter
from .adapters.codebase_memory import CodebaseMemoryAdapter
from .adapters.ripwire import RipwireAdapter
from .fixtures import load_corpus, materialize
from .protocol import NativeResult, QueryKind, Status
from .resources import read_meminfo
from .scoring import assess, assess_definition, assess_test_predictions


ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs" / "competitor-benchmark"
ENV_LIMITS = {
    "CARGO_BUILD_JOBS": "1", "CMAKE_BUILD_PARALLEL_LEVEL": "1", "MAKEFLAGS": "-j1",
    "RAYON_NUM_THREADS": "1", "npm_config_jobs": "1",
}


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def verify_freeze_manifest() -> None:
    manifest = json.loads((DOCS / "freeze-manifest.json").read_text(encoding="utf-8"))
    failures = []
    for relative, expected in manifest["sha256"].items():
        actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
        if actual != expected:
            failures.append(relative)
    if failures:
        raise RuntimeError(f"frozen inputs changed: {', '.join(failures)}")


def assess_native(
    native: NativeResult,
    kind: str,
    oracle: Mapping[str, Any],
    prior: Mapping[str, Any] | None,
) -> tuple[Status, dict[str, float | int], tuple[str, ...]]:
    expected = oracle["expected"][kind]
    prior_expected = None if prior is None else prior["expected"][kind]
    if kind == QueryKind.DEFINITION.value:
        status, score = assess_definition(
            native.answer, expected, source_text=native.metadata.get("source_text"),
            expected_source_marker=oracle["definition_source_marker"],
            prior_expected=prior_expected,
            prior_source_marker=None if prior is None else prior["definition_source_marker"],
            native_status=native.status,
        )
    elif kind == QueryKind.TESTS.value:
        status, score, scored_answer = assess_test_predictions(
            native.answer, expected, oracle["test_inventory"],
            prior_expected=prior_expected,
            prior_inventory=None if prior is None else prior["test_inventory"],
            native_status=native.status,
        )
        return status, score.to_dict(), scored_answer
    else:
        status, score = assess(native.answer, expected, prior_expected=prior_expected,
                               native_status=native.status)
        return status, score.to_dict(), tuple(native.answer)
    return status, score.to_dict(), tuple(native.answer)


def run_campaign(product: str, fixture_id: str, binary: Path, work_root: Path, output: Path) -> dict[str, Any]:
    verify_freeze_manifest()
    policy = json.loads((DOCS / "policy.json").read_text(encoding="utf-8"))
    corpus = load_corpus(DOCS / "corpus.json")
    oracle_data = json.loads((DOCS / "oracle.json").read_text(encoding="utf-8"))
    fixture = next(row for row in corpus["fixtures"] if row["id"] == fixture_id)
    oracles = {row["state"]: row for row in oracle_data["states"] if row["fixture_id"] == fixture_id}
    if set(oracles) != set(range(len(fixture["mutations"]) + 1)):
        raise ValueError("oracle states do not match mutation sequence")

    output.mkdir(parents=True, exist_ok=False)
    capture_environment(output)
    work_root.mkdir(parents=True, exist_ok=True)
    fixture_root = work_root / "fixture"
    private_home = work_root / "home"
    materialize(fixture, fixture_root)
    _git_baseline(fixture_root)
    for key, value in ENV_LIMITS.items():
        os.environ[key] = value

    limits = {
        "prepare_timeout": policy["timeouts_seconds"]["prepare_or_cold_index"],
        "query_timeout": policy["timeouts_seconds"]["query"],
        "max_output_bytes": policy["host_limits"]["maximum_combined_output_bytes_per_invocation"],
        "minimum_available_bytes": policy["host_limits"]["minimum_mem_available_before_expensive_step_bytes"],
        "emergency_available_bytes": policy["host_limits"]["emergency_mem_available_bytes"],
        "maximum_tree_rss_bytes": policy["host_limits"]["maximum_competitor_process_tree_rss_bytes"],
    }
    if product in {"girder", "girder-watch"}:
        adapter = GirderAdapter(binary, watch=(product == "girder-watch"), limits=limits,
                                private_home=private_home)
    elif product == "codebase-memory-mcp":
        adapter = CodebaseMemoryAdapter(binary, limits=limits, private_home=private_home)
    elif product == "ripwire":
        adapter = RipwireAdapter(binary, limits=limits, private_home=private_home,
                                 initial_target=oracles[0]["target"])
    else:
        raise ValueError(f"adapter is not implemented: {product}")
    records: list[dict[str, Any]] = []
    started = time.monotonic()
    campaign_deadline = started + policy["timeouts_seconds"]["single_product_fixture_campaign"]
    try:
        prepare_started = utc_now()
        prepare = adapter.prepare(fixture_root, output / "raw" / "prepare")
        records.append(_record_native(prepare, adapter.name, adapter.version, adapter.commit,
                                      fixture_id, "prepare", "prepare",
                                      prepare_started, utc_now(), (), ()))
        if prepare.status is not Status.PASS:
            return _finish(output, product, fixture_id, records, "PREPARE_FAILED", started)

        query_functions: dict[str, Callable[[str], NativeResult]] = {
            "definition": adapter.query_definition,
            "callers": adapter.query_callers,
            "callees": adapter.query_callees,
            "impact": adapter.query_impact,
            "tests": adapter.query_tests,
        }
        for phase in ("warmup", "warm_query"):
            for kind in policy["scope"]["query_order"]:
                _deadline_check(campaign_deadline)
                native, status, score, scored_answer, began, ended = _query_and_score(
                    query_functions[kind], oracles[0]["target"], kind, oracles[0], None
                )
                records.append(_record_native(native, adapter.name, adapter.version, adapter.commit, fixture_id,
                                              f"state0:{phase}:{kind}", kind, began, ended,
                                              oracles[0]["expected"][kind], (), status, score,
                                              scored_answer=scored_answer))

        mutation_summaries = []
        for state, mutation in enumerate(fixture["mutations"], 1):
            _deadline_check(campaign_deadline)
            mutation_started_mono = time.monotonic()
            mutation_started = utc_now()
            changed = adapter.apply_mutation(mutation)
            records.append(_record_native(changed, adapter.name, adapter.version, adapter.commit, fixture_id,
                                          f"state{state}:mutation:{mutation['id']}", "mutation",
                                          mutation_started, utc_now(), (), ()))
            if changed.status is not Status.PASS:
                mutation_summaries.append({"state": state, "mutation": mutation["id"],
                                           "terminal_status": changed.status.value,
                                           "update_to_correct_seconds": None})
                break
            adapter.wait_until_ready(policy["timeouts_seconds"]["wait_until_ready"])
            probe_deadline = mutation_started_mono + policy["timeouts_seconds"]["wait_until_ready"]
            prior_signature: tuple[tuple[str, str, tuple[str, ...]], ...] | None = None
            identical_wrong = 0
            first_pass: dict[str, float] = {}
            probe_index = 0
            terminal = "TIMEOUT"
            while time.monotonic() < probe_deadline:
                _deadline_check(campaign_deadline)
                probe_index += 1
                probe_statuses: dict[str, Status] = {}
                signature_parts = []
                for kind in policy["scope"]["query_order"]:
                    native, status, score, scored_answer, began, ended = _query_and_score(
                        query_functions[kind], oracles[state]["target"], kind,
                        oracles[state], oracles[state - 1]
                    )
                    probe_statuses[kind] = status
                    signature_parts.append((kind, status.value, native.answer))
                    if status is Status.PASS and kind not in first_pass:
                        first_pass[kind] = time.monotonic() - mutation_started_mono
                    records.append(_record_native(
                        native, adapter.name, adapter.version, adapter.commit, fixture_id,
                        f"state{state}:probe{probe_index}:{kind}", kind, began, ended,
                        oracles[state]["expected"][kind], oracles[state - 1]["expected"][kind],
                        status, score, scored_answer=scored_answer,
                    ))
                comparable = [value for value in probe_statuses.values() if value is not Status.UNSUPPORTED]
                if comparable and all(value is Status.PASS for value in comparable):
                    terminal = "PASS"
                    break
                if any(value in {Status.TIMEOUT, Status.RESOURCE_BLOCKED, Status.ERROR} for value in comparable):
                    terminal = next(value.value for value in comparable
                                    if value in {Status.TIMEOUT, Status.RESOURCE_BLOCKED, Status.ERROR})
                    break
                signature = tuple(signature_parts)
                identical_wrong = _wrong_stability_count(
                    comparable, signature, prior_signature, identical_wrong
                )
                prior_signature = signature
                elapsed = time.monotonic() - mutation_started_mono
                if identical_wrong >= policy["timeouts_seconds"]["identical_wrong_probe_sets_before_terminal"] \
                        and elapsed >= policy["timeouts_seconds"]["minimum_wrong_stability_window"]:
                    terminal = "WRONG"
                    break
                time.sleep(policy["timeouts_seconds"]["freshness_poll_interval"])
            elapsed = time.monotonic() - mutation_started_mono
            mutation_summaries.append({
                "state": state, "mutation": mutation["id"], "terminal_status": terminal,
                "update_to_correct_seconds": elapsed if terminal == "PASS" else None,
                "first_pass_seconds_by_query": first_pass,
                "probe_sets": probe_index,
            })
        result = _finish(output, product, fixture_id, records, "COMPLETE", started,
                         mutation_summaries=mutation_summaries)
        return result
    finally:
        adapter.cleanup()


def _query_and_score(function: Callable[[str], NativeResult], target: str, kind: str,
                     oracle: Mapping[str, Any], prior: Mapping[str, Any] | None):
    began = utc_now()
    native = function(target)
    ended = utc_now()
    status, score, scored_answer = assess_native(native, kind, oracle, prior)
    return native, status, score, scored_answer, began, ended


def _record_native(native: NativeResult, product: str, version: str, commit: str,
                   fixture: str, task: str, kind: str,
                   began: str, ended: str, expected: tuple[str, ...] | list[str],
                   prior: tuple[str, ...] | list[str], status: Status | None = None,
                   score: Mapping[str, float | int] | None = None,
                   scored_answer: tuple[str, ...] | None = None) -> dict[str, Any]:
    meta = native.metadata
    calls = meta.get("methods", [])
    process = meta.get("process", {})
    return {
        "schema_version": 1, "competitor": product, "version": version,
        "commit": commit, "task_id": task, "fixture_id": fixture,
        "query_kind": kind, "command_or_tools": calls or process.get("command", []),
        "started_at": began, "ended_at": ended,
        "elapsed_seconds": meta.get("elapsed_seconds", process.get("wall_seconds", 0.0)),
        "exit_status": process.get("returncode", (meta.get("returncodes") or [None])[-1]),
        "timeout_status": (status or native.status) is Status.TIMEOUT,
        "resource_blocked_status": (status or native.status) is Status.RESOURCE_BLOCKED,
        "stdout_bytes": meta.get("stdout_bytes", process.get("stdout_bytes", 0)),
        "stderr_bytes": meta.get("stderr_bytes", process.get("stderr_bytes", 0)),
        "stdout_artifacts": meta.get("stdout_artifacts", [process["stdout_artifact"]] if process.get("stdout_artifact") else []),
        "stderr_artifacts": meta.get("stderr_artifacts", [process["stderr_artifact"]] if process.get("stderr_artifact") else []),
        "normalized_answer": list(native.answer), "expected_answer": list(expected),
        "scored_answer": list(native.answer if scored_answer is None else scored_answer),
        "prior_expected_answer": list(prior), "status": (status or native.status or Status.ERROR).value,
        "score": dict(score or {}), "tool_calls": native.tool_calls,
        "peak_rss_bytes": meta.get("peak_rss_bytes", process.get("peak_rss_bytes")),
        "detail": native.detail,
    }


def _finish(output: Path, product: str, fixture: str, records: list[dict[str, Any]], state: str,
            started: float, **extra: Any) -> dict[str, Any]:
    result = {
        "schema_version": 1, "policy_id": policy_id(),
        "policy_sha256": hashlib.sha256((DOCS / "policy.json").read_bytes()).hexdigest(),
        "freeze_manifest_sha256": hashlib.sha256((DOCS / "freeze-manifest.json").read_bytes()).hexdigest(),
        "harness_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "product": product, "fixture": fixture, "campaign_state": state,
        "elapsed_seconds": time.monotonic() - started, "records": records, **extra,
    }
    (output / "result.json").write_text(json.dumps(_jsonable(result), indent=2, sort_keys=True) + "\n")
    return result


def capture_environment(output: Path) -> None:
    mem = read_meminfo()
    data = {
        "captured_at": utc_now(), "platform": platform.platform(), "machine": platform.machine(),
        "python": platform.python_version(), "cpu_count": os.cpu_count(),
        "memory": vars(mem), "required_environment": ENV_LIMITS,
    }
    (output / "environment.json").write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")


def policy_id() -> str:
    return json.loads((DOCS / "policy.json").read_text(encoding="utf-8"))["policy_id"]


def _git_baseline(root: Path) -> None:
    for command in (["git", "init", "--quiet"], ["git", "config", "user.name", "Benchmark"],
                    ["git", "config", "user.email", "benchmark@example.invalid"],
                    ["git", "add", "."], ["git", "commit", "--quiet", "-m", "base"]):
        subprocess.run(command, cwd=root, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def _deadline_check(deadline: float) -> None:
    if time.monotonic() >= deadline:
        raise TimeoutError("single-product fixture campaign deadline exceeded")


def _wrong_stability_count(
    statuses: Sequence[Status], signature: tuple[Any, ...],
    prior_signature: tuple[Any, ...] | None, current: int,
) -> int:
    if any(status is Status.STALE for status in statuses):
        return 0
    if not any(status is Status.WRONG for status in statuses):
        return 0
    return current + 1 if signature == prior_signature else 1


def _jsonable(value: Any) -> Any:
    if isinstance(value, Status):
        return value.value
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [_jsonable(item) for item in value]
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--product", choices=("girder", "girder-watch", "ripwire", "codebase-memory-mcp"),
        required=True,
    )
    parser.add_argument("--fixture", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--work-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    lock_path = args.work_root.parent / "campaign.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("another competitor campaign is already running")
        try:
            run_campaign(args.product, args.fixture, args.binary, args.work_root, args.output)
        except BaseException as error:
            args.output.mkdir(parents=True, exist_ok=True)
            failure = {
                "schema_version": 1,
                "classification": "INTERRUPTED_CAMPAIGN",
                "recorded_at": utc_now(),
                "exception_type": type(error).__name__,
                "detail": str(error),
            }
            (args.output / "failure.json").write_text(
                json.dumps(failure, indent=2, sort_keys=True) + "\n"
            )
            raise
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
