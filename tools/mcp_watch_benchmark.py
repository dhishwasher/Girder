#!/usr/bin/env python3
"""Run the frozen native-watcher probe once, outside Cargo; preserve partial data."""
import argparse
import hashlib
from itertools import zip_longest
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time

from tools.harness_support import run_bounded
from tools.incremental_update_benchmark import atomic_json, require_no_cargo

ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "docs/mcp-watch-measurement-corpus.json"
POLICY = ROOT / "docs/mcp-watch-policy.json"
PROBE = "project::commands::mcp::watch::tests::measurement::watch_measurement"


def equal(record):
    oracle = record.get("oracle", {})
    return all(oracle.get(flag) is True and isinstance(oracle.get(left), str)
               and len(oracle[left]) == 64 and oracle[left] == oracle.get(right)
               for flag, left, right in (("equal_source", "source_sha256", "cold_source_sha256"),
                                         ("equal_persisted", "persisted_sha256", "cold_reconciled_sha256")))


def assess(records, corpus):
    phases = []
    for case in corpus["cases"]:
        phases.extend([("initial_started", case["id"]), ("initial", case["id"])])
        for mutation in case["mutations"]:
            identity = f'{case["id"]}/{mutation["id"]}'
            phases.extend([("started", identity), ("completed", identity)])
    observed = [(r.get("kind"), r.get("case") if r.get("kind") in ("initial_started", "initial") else r.get("id"))
                if isinstance(r, dict) else ("invalid_record", None) for r in records]
    transcript_errors = [{"record_index": index, "expected": wanted, "observed": actual}
                         for index, (wanted, actual) in enumerate(zip_longest(phases, observed)) if wanted != actual]
    records = [r for r in records if isinstance(r, dict)]
    expected = {f'{case["id"]}/{mutation["id"]}' for case in corpus["cases"] for mutation in case["mutations"]}
    initials = [r for r in records if r.get("kind") == "initial"]
    completed = [r for r in records if r.get("kind") == "completed"]
    cases = {case["id"] for case in corpus["cases"]}
    matched = sum(equal(r) for r in completed)
    all_present = len(completed) == len(expected) and {r.get("id") for r in completed} == expected
    initial_present = len(initials) == len(cases) and {r.get("case") for r in initials} == cases
    metrics = {key: sum(r["metrics"][key] for r in completed) for key in (
        "attempts", "builds", "published", "discarded_candidates", "failed_attempts", "full_parsing", "parsed_files", "reused_files")}
    reasons = {}
    for r in completed:
        for reason, count in r["metrics"]["fallback_reasons"].items():
            reasons[reason] = reasons.get(reason, 0) + count
    latency = [r["mutation_to_publication_seconds"] for r in completed]
    return {
        "overall": "PASS" if not transcript_errors and all_present and initial_present and matched == len(expected) and all(equal(r) for r in initials) else "FAIL",
        "transcript_errors": transcript_errors,
        "expected_mutations": len(expected), "completed_mutations": len(completed), "equal_mutations": matched,
        "expected_initials": len(cases), "completed_initials": len(initials), "equal_initials": sum(equal(r) for r in initials),
        "mismatches": [r.get("id", r.get("case")) for r in completed + initials if not equal(r)],
        "metrics_for_completed_mutations": metrics,
        "fallback_denominator": metrics["builds"],
        "fallback_percentage": 100 * metrics["full_parsing"] / metrics["builds"] if metrics["builds"] else None,
        "fallback_reasons": reasons,
        "median_mutation_to_publication_seconds": statistics.median(latency) if latency else None,
        "maximum_mutation_to_publication_seconds": max(latency) if latency else None,
        "initial_construction_seconds": sum(r["seconds"] for r in initials),
        "counter_scope": "Completed-mutation deltas include discarded candidates and retries. Raw events retain emitted counter checkpoints for interrupted or uncompleted mutations; work after the last checkpoint is unquantified. Initial construction is excluded from the fallback denominator.",
    }


def parse_records(text, prefix=""):
    records = []
    for line in text.splitlines():
        if prefix and not line.startswith(prefix):
            continue
        line = line.removeprefix(prefix)
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError:
            records.append({"kind": "incomplete_record", "raw": line})
    return records


def read_records(path):
    return parse_records(path.read_bytes().decode("utf-8", errors="replace")) if path.exists() else []


def frozen_inputs():
    paths = subprocess.check_output(["git", "ls-files"], cwd=ROOT, text=True).splitlines()
    selected = [p for p in paths if p.startswith(("crates/aether-app/", "crates/aether-builder/", "crates/aether-graph/"))]
    selected += ["Cargo.toml", "Cargo.lock", "tools/harness_support.py", "tools/incremental_update_benchmark.py",
                 "tools/mcp_watch_benchmark.py", "tools/test_mcp_watch_benchmark.py", str(POLICY.relative_to(ROOT)), str(CORPUS.relative_to(ROOT))]
    for p in selected:
        subprocess.run(["git", "ls-files", "--error-unmatch", p], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "diff", "--exit-code", "HEAD", "--", *selected], cwd=ROOT, check=True, stdout=subprocess.DEVNULL)
    return {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in sorted(set(selected))}


def claim_output(output):
    companions = tuple(output.with_name(output.stem + suffix) for suffix in (
        "-probe.jsonl", "-stdout.log", "-stderr.log", "-failure.log"))
    # A prior partial run may have only its sidecars left. Preserve those too.
    for path in (output, *companions):
        if path.exists() or path.is_symlink():
            raise FileExistsError(f"refuse existing observation artifact: {path}")
    with output.open("x") as file:
        file.write("{}\n")
    return companions


def write_artifact(path, text):
    # Exclusive creation also protects against collisions after the preflight.
    with path.open("x") as file:
        file.write(text)


def campaign(binary, output):
    require_no_cargo()
    corpus = json.loads(CORPUS.read_text())
    inputs = frozen_inputs()
    if hashlib.sha256((ROOT / "docs" / corpus["source_corpus"]).read_bytes()).hexdigest() != corpus["source_sha256"]:
        raise RuntimeError("source corpus changed after watch corpus was frozen")
    # Claim before starting the process. No overwrite, including incomplete runs.
    raw, stdout, stderr, failure = claim_output(output)
    observation = {"schema_version": 1, "state": "running", "implementation_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                   "policy": str(POLICY.relative_to(ROOT)), "corpus": str(CORPUS.relative_to(ROOT)), "input_sha256": inputs,
                   "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "platform": platform.platform(),
                   "probe": PROBE, "records": [], "assessment": assess([], corpus),
                   "limitations": "Synthetic filesystem update latency and cold-analysis equivalence, not production latency or improved semantic coverage. The observer waits on publication without running cold analysis until after the latency timer stops. Background integrity checks remain active during the cold oracle. No model is used."}
    atomic_json(output, observation)
    environment = dict(os.environ, GIRDER_WATCH_MEASUREMENT_INPUT=str(CORPUS), GIRDER_WATCH_MEASUREMENT_OUTPUT=str(raw))
    environment.pop("GIRDER_FAULT_EXIT", None)
    environment.pop("GIRDER_WATCH_TEST_QUERY_DELAY_MS", None)
    started = time.monotonic()
    try:
        with stdout.open("xb") as stdout_file, stderr.open("xb") as stderr_file:
            streams = {"stdout": stdout_file, "stderr": stderr_file}

            def capture(name, chunk):
                streams[name].write(chunk)
                streams[name].flush()

            result = run_bounded([str(binary), PROBE, "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                 cwd=ROOT, env=environment, timeout_seconds=1800, max_output_bytes=32 * 1024 * 1024,
                                 check=False, on_output=capture)
        observation["returncode"] = result.returncode
        observation["state"] = "completed" if result.returncode == 0 else "incomplete"
    except BaseException as error:
        observation["state"] = "incomplete"
        observation["error"] = f"{type(error).__name__}: {error}"
        # The bounded runner includes captured stdout/stderr in timeout and
        # output-limit errors. Preserve those bytes and their event counters
        # even when it cannot return its normal process result.
        write_artifact(failure, observation["error"] + "\n")
        raise
    finally:
        observation["elapsed_seconds"] = time.monotonic() - started
        observation["records"] = read_records(raw)
        observation["assessment"] = assess(observation["records"], corpus)
        if observation["state"] != "completed":
            observation["assessment"]["overall"] = "FAIL"
        observation["raw_events"] = parse_records(stderr.read_bytes().decode("utf-8", errors="replace"), "girder watch: ") if stderr.exists() else []
        if failure.exists() and not stderr.exists():
            observation["raw_events"].extend(parse_records(failure.read_text(), "girder watch: "))
        observation["artifacts"] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (raw, stdout, stderr, failure) if p.exists()}
        atomic_json(output, observation)
    print(json.dumps(observation["assessment"], sort_keys=True))
    return 0 if observation["assessment"]["overall"] == "PASS" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "docs/mcp-watch-observation.json")
    args = parser.parse_args()
    return campaign(args.test_binary.resolve(), args.output.resolve())


if __name__ == "__main__":
    raise SystemExit(main())
