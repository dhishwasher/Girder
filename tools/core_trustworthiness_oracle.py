#!/usr/bin/env python3
"""Reproducible static-vs-dynamic affected-test measurement for Bit Code."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence

try:
    from tools.harness_support import BoundedProcessResult, run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from harness_support import BoundedProcessResult, run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ROOT = REPO_ROOT / "fixtures" / "core-trustworthiness"
DEFAULT_BASELINE = REPO_ROOT / "docs" / "core-trustworthiness-baseline.json"
IMPACTED_HEADER = re.compile(r"^Impacted tests \((\d+)\):$")
IMPACTED_TEST = re.compile(r"^\s*✓\s+(.+?)\s+\((rust|python)\)\s*$")
SKIPPED_TESTS = re.compile(r"^\s*\((\d+) other test\(s\) not in impact set")
DEFAULT_COMMAND_TIMEOUT_SECONDS = 120.0
DEFAULT_MAX_OUTPUT_BYTES = 1024 * 1024


@dataclass(frozen=True)
class Fixture:
    name: str
    tests: tuple[str, ...]
    graph_paths: Mapping[str, str]
    framework_ids: Mapping[str, str]
    discovery_command: tuple[str, ...]
    dynamic_commands: Mapping[str, tuple[str, ...]]


RUST_TESTS = (
    "rust_direct_selected",
    "rust_cli_selected",
    "rust_cli_unrelated",
    "rust_unrelated",
    "rust_custom_bin_selected",
    "rust_raii_drop_selected",
)
PYTHON_TESTS = (
    "test_python_direct_selected",
    "test_python_optional_selected",
    "test_python_decoy",
    "test_python_cross_module_selected",
    "test_python_third_party_decoy",
)

FIXTURES = (
    Fixture(
        name="rust",
        tests=RUST_TESTS,
        graph_paths={test: f"crate::tests::impact::{test}" for test in RUST_TESTS},
        framework_ids={test: test for test in RUST_TESTS},
        discovery_command=(
            "cargo",
            "test",
            "--quiet",
            "--test",
            "impact",
            "--",
            "--list",
            "--format",
            "terse",
        ),
        dynamic_commands={
            test: (
                "cargo",
                "test",
                "--quiet",
                "--test",
                "impact",
                test,
                "--",
                "--exact",
                "--test-threads=1",
            )
            for test in RUST_TESTS
        },
    ),
    Fixture(
        name="python",
        tests=PYTHON_TESTS,
        graph_paths={
            test: f"crate::tests::test_service::OracleTests::{test}"
            for test in PYTHON_TESTS
        },
        framework_ids={
            test: f"tests.test_service.OracleTests.{test}" for test in PYTHON_TESTS
        },
        discovery_command=(
            sys.executable,
            "-c",
            """import unittest
def flatten(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from flatten(item)
        else:
            yield item
suite = unittest.defaultTestLoader.discover("tests", top_level_dir=".")
print("\\n".join(test.id() for test in flatten(suite)))
""",
        ),
        dynamic_commands={
            test: (
                sys.executable,
                "-m",
                "unittest",
                "-q",
                f"tests.test_service.OracleTests.{test}",
            )
            for test in PYTHON_TESTS
        },
    ),
)


def run(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    timeout_seconds: float = DEFAULT_COMMAND_TIMEOUT_SECONDS,
    max_output_bytes: int = DEFAULT_MAX_OUTPUT_BYTES,
) -> BoundedProcessResult:
    return run_bounded(
        command,
        cwd=cwd,
        env=env,
        timeout_seconds=timeout_seconds,
        max_output_bytes=max_output_bytes,
    )


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def initialize_fixture(
    fixture: Fixture,
    project: Path,
    *,
    timeout_seconds: float,
    max_output_bytes: int,
) -> Path:
    source = FIXTURE_ROOT / fixture.name / "baseline"
    mutations = FIXTURE_ROOT / fixture.name / "mutations"
    project.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source, project)
    for template in sorted(project.rglob("*.txt")):
        template.rename(template.with_suffix(""))
    command_options = {
        "timeout_seconds": timeout_seconds,
        "max_output_bytes": max_output_bytes,
    }
    run(("git", "init", "--quiet"), cwd=project, **command_options)
    run(
        ("git", "config", "user.email", "oracle@bitcode.invalid"),
        cwd=project,
        **command_options,
    )
    run(
        ("git", "config", "user.name", "Bit Code Oracle"),
        cwd=project,
        **command_options,
    )
    run(("git", "add", "."), cwd=project, **command_options)
    run(
        ("git", "commit", "--quiet", "-m", "baseline"),
        cwd=project,
        **command_options,
    )
    for mutation in sorted(path for path in mutations.rglob("*") if path.is_file()):
        relative = mutation.relative_to(mutations)
        destination = project / (
            relative.with_suffix("") if relative.suffix == ".txt" else relative
        )
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(mutation, destination)
    return project


def parse_bitcode_selection(output: str) -> tuple[set[str], int, int]:
    selected: set[str] = set()
    reported_impacted = 0
    reported_skipped = 0
    for line in output.splitlines():
        if match := IMPACTED_HEADER.match(line):
            reported_impacted = int(match.group(1))
        elif match := IMPACTED_TEST.match(line):
            selected.add(match.group(1))
        elif match := SKIPPED_TESTS.match(line):
            reported_skipped = int(match.group(1))
    if reported_impacted != len(selected):
        raise RuntimeError(
            "Bit Code's impacted-test header disagrees with its listed tests: "
            f"header={reported_impacted}, listed={len(selected)}"
        )
    return selected, reported_impacted, reported_skipped


def parse_framework_inventory(fixture_name: str, output: str) -> set[str]:
    if fixture_name == "rust":
        suffix = ": test"
        return {
            line[: -len(suffix)]
            for line in output.splitlines()
            if line.endswith(suffix)
        }
    if fixture_name == "python":
        return {line.strip() for line in output.splitlines() if line.strip()}
    raise ValueError(f"unsupported fixture language: {fixture_name}")


def assert_isolated_test_executed(
    fixture_name: str,
    test: str,
    completed: BoundedProcessResult,
) -> None:
    combined = f"{completed.stdout}\n{completed.stderr}"
    if fixture_name == "rust":
        observed = re.search(r"(?m)^test result: ok\. 1 passed; 0 failed;", combined)
    elif fixture_name == "python":
        observed = re.search(r"(?m)^Ran 1 test(?:s)? in ", combined)
    else:
        raise ValueError(f"unsupported fixture language: {fixture_name}")
    if observed is None:
        raise RuntimeError(
            f"isolated {fixture_name} command did not execute exactly one test: {test}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )


def metrics(
    universe: set[str],
    selected: set[str],
    executed: set[str],
    *,
    reported_impacted: int,
    reported_skipped: int,
) -> dict[str, object]:
    unknown = (selected | executed) - universe
    if unknown:
        raise RuntimeError(f"measurement contains unknown tests: {sorted(unknown)}")
    if reported_impacted + reported_skipped != len(universe):
        raise RuntimeError(
            "Bit Code's impacted and skipped counts disagree with the test universe: "
            f"impacted={reported_impacted}, skipped={reported_skipped}, "
            f"universe={len(universe)}"
        )
    true_positives = selected & executed
    false_positives = selected - executed
    false_negatives = executed - selected
    true_negatives = universe - selected - executed
    precision_denominator = len(true_positives) + len(false_positives)
    recall_denominator = len(true_positives) + len(false_negatives)
    precision = (
        len(true_positives) / precision_denominator
        if precision_denominator
        else 1.0
    )
    recall = (
        len(true_positives) / recall_denominator if recall_denominator else 1.0
    )
    return {
        "universe": sorted(universe),
        "selected": sorted(selected),
        "executed": sorted(executed),
        "true_positives": sorted(true_positives),
        "false_positives": sorted(false_positives),
        "false_negatives": sorted(false_negatives),
        "true_negatives": sorted(true_negatives),
        "precision": round(precision, 6),
        "recall": round(recall, 6),
        "bitcode_reported_impacted": reported_impacted,
        "bitcode_reported_skipped": reported_skipped,
    }


def aggregate_metrics(
    results: Mapping[str, Mapping[str, object]],
) -> dict[str, float | int]:
    counts = {
        key: sum(len(result[key]) for result in results.values())
        for key in (
            "true_positives",
            "false_positives",
            "false_negatives",
            "true_negatives",
        )
    }
    precision_denominator = counts["true_positives"] + counts["false_positives"]
    recall_denominator = counts["true_positives"] + counts["false_negatives"]
    return {
        **counts,
        "precision": round(
            counts["true_positives"] / precision_denominator
            if precision_denominator
            else 1.0,
            6,
        ),
        "recall": round(
            counts["true_positives"] / recall_denominator
            if recall_denominator
            else 1.0,
            6,
        ),
    }


def measure_fixture(
    fixture: Fixture,
    work_root: Path,
    bitcode: Path,
    *,
    verbose: bool,
    timeout_seconds: float,
    max_output_bytes: int,
) -> dict[str, object]:
    command_options = {
        "timeout_seconds": timeout_seconds,
        "max_output_bytes": max_output_bytes,
    }
    static_project = initialize_fixture(
        fixture,
        work_root / "static",
        **command_options,
    )
    inventory = run(
        fixture.discovery_command,
        cwd=static_project,
        **command_options,
    )
    observed_framework_ids = parse_framework_inventory(fixture.name, inventory.stdout)
    expected_framework_ids = set(fixture.framework_ids.values())
    if observed_framework_ids != expected_framework_ids:
        raise RuntimeError(
            f"{fixture.name} framework inventory differs from the declared universe: "
            f"expected={sorted(expected_framework_ids)}, "
            f"observed={sorted(observed_framework_ids)}"
        )
    static = run(
        (str(bitcode), "test-impact", str(static_project)),
        cwd=REPO_ROOT,
        **command_options,
    )
    if verbose:
        print(f"\n--- Bit Code {fixture.name} output ---\n{static.stdout.rstrip()}")
    selected_paths, reported_impacted, reported_skipped = parse_bitcode_selection(
        static.stdout
    )
    path_to_test = {path: test for test, path in fixture.graph_paths.items()}
    unknown_paths = selected_paths - path_to_test.keys()
    if unknown_paths:
        raise RuntimeError(
            f"Bit Code selected unknown graph test paths: {sorted(unknown_paths)}"
        )
    selected = {path_to_test[path] for path in selected_paths}

    executed: set[str] = set()
    for index, test in enumerate(fixture.tests):
        project = initialize_fixture(
            fixture,
            work_root / f"dynamic-{index}",
            **command_options,
        )
        probe = project / ".bitcode-oracle-probe"
        environment = os.environ.copy()
        environment["BITCODE_ORACLE_PROBE"] = str(probe)
        completed = run(
            fixture.dynamic_commands[test],
            cwd=project,
            env=environment,
            **command_options,
        )
        assert_isolated_test_executed(fixture.name, test, completed)
        if probe.exists() and "selected_operation" in probe.read_text(
            encoding="utf-8"
        ).splitlines():
            executed.add(test)

    return metrics(
        set(fixture.tests),
        selected,
        executed,
        reported_impacted=reported_impacted,
        reported_skipped=reported_skipped,
    )


def render_table(results: Mapping[str, Mapping[str, object]]) -> str:
    lines = [
        "| Fixture | TP | FP | FN | TN | Precision | Recall |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for name in ("rust", "python"):
        result = results[name]
        lines.append(
            f"| {name.title()} | {len(result['true_positives'])} | "
            f"{len(result['false_positives'])} | "
            f"{len(result['false_negatives'])} | "
            f"{len(result['true_negatives'])} | "
            f"{float(result['precision']):.3f} | "
            f"{float(result['recall']):.3f} |"
        )
    aggregate = aggregate_metrics(results)
    lines.append(
        f"| Combined | {aggregate['true_positives']} | "
        f"{aggregate['false_positives']} | {aggregate['false_negatives']} | "
        f"{aggregate['true_negatives']} | "
        f"{float(aggregate['precision']):.3f} | "
        f"{float(aggregate['recall']):.3f} |"
    )
    return "\n".join(lines)


def check_baseline(
    document: Mapping[str, object], baseline_path: Path
) -> None:
    baseline = json.loads(baseline_path.read_text(encoding="utf-8"))
    if baseline != document:
        expected_text = json.dumps(baseline, indent=2, sort_keys=True)
        actual_text = json.dumps(document, indent=2, sort_keys=True)
        raise RuntimeError(
            f"measurement differs from {baseline_path}\n"
            f"expected:\n{expected_text}\nactual:\n{actual_text}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bitcode",
        type=Path,
        required=True,
        help="exact Bit Code binary to measure",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=DEFAULT_BASELINE,
        help="checked JSON baseline",
    )
    parser.add_argument(
        "--no-check",
        action="store_true",
        help="print results without comparing the checked baseline",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="print the complete result as JSON",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="include Bit Code's raw test-impact output",
    )
    parser.add_argument(
        "--command-timeout-seconds",
        type=float,
        default=DEFAULT_COMMAND_TIMEOUT_SECONDS,
        help="per-command timeout, including each isolated test",
    )
    parser.add_argument(
        "--max-command-output-bytes",
        type=int,
        default=DEFAULT_MAX_OUTPUT_BYTES,
        help="combined stdout/stderr limit for each child process",
    )
    args = parser.parse_args()
    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        parser.error(f"Bit Code binary does not exist: {bitcode}")
    if args.command_timeout_seconds <= 0:
        parser.error("--command-timeout-seconds must be positive")
    if args.max_command_output_bytes <= 0:
        parser.error("--max-command-output-bytes must be positive")

    binary_sha256 = file_sha256(bitcode)
    with tempfile.TemporaryDirectory(prefix="bitcode-core-trust-") as temporary:
        work_root = Path(temporary)
        measured_bitcode = work_root / "bitcode-under-test"
        shutil.copy2(bitcode, measured_bitcode)
        measured_bitcode.chmod(0o500)
        if file_sha256(measured_bitcode) != binary_sha256:
            raise RuntimeError("the private Bit Code copy differs from the supplied binary")
        results = {
            fixture.name: measure_fixture(
                fixture,
                work_root / fixture.name,
                measured_bitcode,
                verbose=args.verbose,
                timeout_seconds=args.command_timeout_seconds,
                max_output_bytes=args.max_command_output_bytes,
            )
            for fixture in FIXTURES
        }
        if file_sha256(measured_bitcode) != binary_sha256:
            raise RuntimeError("the private Bit Code copy changed during the oracle run")
    if file_sha256(bitcode) != binary_sha256:
        raise RuntimeError("the measured Bit Code binary changed during the oracle run")
    document = {
        "schema_version": 1,
        "results": results,
        "aggregate": aggregate_metrics(results),
    }

    if not args.no_check:
        check_baseline(document, args.baseline.resolve())
    if args.json:
        print(json.dumps(document, indent=2))
    else:
        print(render_table(results))
        for name in ("rust", "python"):
            result = results[name]
            print(
                f"{name}: selected={','.join(result['selected'])}; "
                f"executed={','.join(result['executed'])}; "
                f"fp={','.join(result['false_positives']) or '-'}; "
                f"fn={','.join(result['false_negatives']) or '-'}"
            )
        if not args.no_check:
            print(f"baseline: matches {args.baseline.resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
