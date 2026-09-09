#!/usr/bin/env python3
"""Record the frozen incremental corpus against independent cold processes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from tools.harness_support import run_bounded

ROOT = Path(__file__).resolve().parents[1]
POLICY = ROOT / "docs/incremental-update-policy.json"
CORPUS = ROOT / "docs/incremental-mutation-corpus.json"
IMPLEMENTATION_INPUTS = [
    "tools/incremental_update_benchmark.py",
    "tools/test_incremental_update_benchmark.py",
    "tools/harness_support.py",
    "crates/aether-builder/src/sync.rs",
    "crates/aether-builder/src/sync/update.rs",
    "crates/aether-builder/src/mapper.rs",
    "crates/aether-builder/examples/incremental_probe.rs",
    "crates/aether-builder/tests/support/incremental.rs",
    "crates/aether-builder/tests/incremental_corpus.rs",
    "crates/aether-builder/tests/incremental_facade.rs",
    "crates/aether-app/src/project/source/incremental.rs",
]


def compact(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def atomic_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    os.replace(temporary, path)


def scaling_fixtures(corpus):
    scaling = corpus["scaling"]
    for language in scaling["languages"]:
        template = scaling["templates"][language]
        for size in scaling["owned_file_counts"]:
            files = {}
            paths = []
            for index in range(size):
                render = lambda text: text.replace("{i}", f"{index:04d}").replace("{previous}", f"{index-1:04d}")
                path = render(template["path"])
                paths.append(path)
                files[path] = template["first_source"] if index == 0 else render(template["source"])
            current = dict(files)
            mutations = []
            for rule in scaling["mutation_rules"]:
                changes = []
                for index in rule["file_indices"]:
                    path = paths[index]
                    source = current[path]
                    if "replace" in rule:
                        for old, new in rule["replace"].items():
                            source = source.replace(old, new)
                    else:
                        source = files[path] + "\n"
                    current[path] = source
                    changes.append({"path": path, "source": source})
                mutations.append({"id": rule["id"], "changes": changes})
            yield {"id": f"scaling-{language}-{size}", "language": language,
                   "files": files, "mutations": mutations, "initial_file_count": size}


def assess(records, expected):
    completed = [record for record in records if record.get("state") == "completed"]
    fallbacks = [record for record in completed if record["update"]["report"]["full_rebuild_reasons"]]
    reasons = {}
    for record in fallbacks:
        for reason in record["update"]["report"]["full_rebuild_reasons"]:
            reasons[reason] = reasons.get(reason, 0) + 1
    matched = sum(record.get("equal") is True for record in completed)
    return {
        "overall": "PASS" if expected > 0 and len(completed) == expected
        and len({record["id"] for record in completed}) == expected and matched == expected else "FAIL",
        "expected_mutations": expected, "completed_mutations": len(completed),
        "equal_mutations": matched, "mismatches": [r["id"] for r in completed if not r.get("equal")],
        "fallback_count": len(fallbacks), "fallback_denominator": len(completed),
        "fallback_percentage": 100 * len(fallbacks) / len(completed) if completed else None,
        "fallback_reasons": reasons,
        "parsed_files": sum(len(r["update"]["report"]["parsed_files"]) for r in completed),
        "reused_files": sum(len(r["update"]["report"]["reused_files"]) for r in completed),
    }


def worker(binary, mode, input_path, directory):
    rss = directory / f"{mode}.rss"
    started = time.monotonic()
    result = run_bounded(["/usr/bin/time", "-f", "%M", "-o", str(rss), str(binary), mode, str(input_path)],
                         cwd=directory, timeout_seconds=300, max_output_bytes=32 * 1024 * 1024, check=False)
    if result.returncode:
        raise RuntimeError(f"{mode} worker exited {result.returncode}: {result.stderr[-4000:]}")
    output = json.loads(result.stdout)
    output["process_elapsed_seconds"] = time.monotonic() - started
    output["peak_rss_kib"] = int(rss.read_text().strip())
    return output


def require_no_cargo():
    result = subprocess.run(["pgrep", "-x", "cargo"], stdout=subprocess.DEVNULL)
    if result.returncode != 1:
        raise RuntimeError("Measurement requires no concurrent Cargo process")


def campaign(binary, output):
    require_no_cargo()
    if output.exists():
        raise RuntimeError("Preserve the existing observation; use a distinct corrective record")
    policy = json.loads(POLICY.read_text())
    corpus = json.loads(CORPUS.read_text())
    freeze = {}
    for relative in policy["freeze_inputs"] + IMPLEMENTATION_INPUTS:
        current = (ROOT / relative).read_bytes()
        committed = subprocess.check_output(["git", "show", f"HEAD:{relative}"], cwd=ROOT)
        if current != committed:
            raise RuntimeError(f"Uncommitted frozen input: {relative}")
        freeze[relative] = hashlib.sha256(current).hexdigest()
    fixtures = corpus["fixtures"] + list(scaling_fixtures(corpus))
    records = [{"id": f"{fixture['id']}/{step['id']}", "fixture": fixture["id"],
                "language": fixture["language"], "initial_file_count": len(fixture["files"]), "state": "pending"}
               for fixture in fixtures for step in fixture["mutations"]]
    observation = {"policy_id": policy["policy_id"], "state": "started", "claim": policy["claim"],
                   "freeze": {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "sha256": freeze},
                   "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                   "memory_scope": policy["measurement"]["memory"], "records": records, "errors": []}
    output.parent.mkdir(parents=True, exist_ok=True)
    atomic_json(output, observation)
    index = 0
    try:
        with tempfile.TemporaryDirectory(prefix="girder-incremental-measure-") as temporary:
            directory = Path(temporary)
            for fixture in fixtures:
                for step_index, step in enumerate(fixture["mutations"]):
                    require_no_cargo()
                    record = records[index]
                    record["state"] = "running"
                    atomic_json(output, observation)
                    input_path = directory / "input.json"
                    input_path.write_text(json.dumps({**fixture, "mutations": fixture["mutations"][:step_index+1]}))
                    cold = worker(binary, "cold", input_path, directory)
                    cold_graph = cold.pop("graph")
                    record.update(cold=cold, cold_graph_sha256=hashlib.sha256(compact(cold_graph).encode()).hexdigest())
                    atomic_json(output, observation)
                    update = worker(binary, "update", input_path, directory)
                    update_graph = update.pop("graph")
                    equal = cold_graph == update_graph
                    record.update(state="completed", equal=equal, cold=cold, update=update,
                                  cold_graph_sha256=hashlib.sha256(compact(cold_graph).encode()).hexdigest(),
                                  update_graph_sha256=hashlib.sha256(compact(update_graph).encode()).hexdigest())
                    if not equal:
                        failure = output.parent / f"{output.stem}-failure-{index:03d}.json"
                        atomic_json(failure, {"id": record["id"], "cold": cold_graph, "update": update_graph})
                        record["failure_artifact"] = failure.name
                    observation["assessment"] = assess(records, len(records))
                    atomic_json(output, observation)
                    print(record["id"], "equal" if equal else "MISMATCH", flush=True)
                    if index == 0 and not equal:
                        raise RuntimeError("The mandatory first facade mutation failed; later cases are gated")
                    index += 1
        observation["state"] = "completed"
    except BaseException as error:
        observation["state"] = "incomplete"
        observation["errors"].append({"type": type(error).__name__, "message": str(error)})
        if index < len(records):
            records[index]["error"] = observation["errors"][-1]
        raise
    finally:
        observation["assessment"] = assess(records, len(records))
        atomic_json(output, observation)
    return observation


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = campaign(args.probe.resolve(), args.output.resolve())
    return 0 if result["assessment"]["overall"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
