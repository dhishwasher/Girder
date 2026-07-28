#!/usr/bin/env python3
"""Reproducible static-vs-dynamic affected-test measurement for Bit Code."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Sequence


REPO_ROOT = Path(__file__).resolve().parents[1]
FIXTURE_ROOT = REPO_ROOT / "fixtures" / "core-trustworthiness"
DEFAULT_BASELINE = REPO_ROOT / "docs" / "core-trustworthiness-baseline.json"
IMPACTED_HEADER = re.compile(r"^Impacted tests \((\d+)\):$")
IMPACTED_TEST = re.compile(r"^\s*✓\s+(.+?)\s+\((rust|python)\)\s*$")
SKIPPED_TESTS = re.compile(r"^\s*\((\d+) other test\(s\) not in impact set")


@dataclass(frozen=True)
class Fixture:
    name: str
    tests: tuple[str, ...]
    dynamic_commands: Mapping[str, tuple[str, ...]]


FIXTURES = (
    Fixture(
        name="rust",
        tests=(
            "rust_direct_selected",
            "rust_cli_selected",
            "rust_cli_unrelated",
            "rust_unrelated",
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
            for test in (
                "rust_direct_selected",
                "rust_cli_selected",
                "rust_cli_unrelated",
                "rust_unrelated",
            )
        },
    ),
    Fixture(
        name="python",
        tests=(
            "test_python_direct_selected",
            "test_python_optional_selected",
            "test_python_decoy",
        ),
        dynamic_commands={
            test: (
                sys.executable,
                "-m",
                "unittest",
                "-q",
                f"tests.test_service.OracleTests.{test}",
            )
            for test in (
                "test_python_direct_selected",
                "test_python_optional_selected",
                "test_python_decoy",
            )
        },
    ),
)


def run(
    command: Sequence[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command,
        cwd=cwd,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        rendered = " ".join(command)
        raise RuntimeError(
            f"command failed ({completed.returncode}): {rendered}\n"
            f"stdout:\n{completed.stdout}\n"
            f"stderr:\n{completed.stderr}"
        )
    return completed


def initialize_fixture(fixture: Fixture, work_root: Path) -> Path:
    source = FIXTURE_ROOT / fixture.name / "baseline"
    mutations = FIXTURE_ROOT / fixture.name / "mutations"
    project = work_root / fixture.name
    shutil.copytree(source, project)
    for template in sorted(project.rglob("*.txt")):
        template.rename(template.with_suffix(""))
    run(("git", "init", "--quiet"), cwd=project)
    run(("git", "config", "user.email", "oracle@bitcode.invalid"), cwd=project)
    run(("git", "config", "user.name", "Bit Code Oracle"), cwd=project)
    run(("git", "add", "."), cwd=project)
    run(("git", "commit", "--quiet", "-m", "baseline"), cwd=project)
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
            selected.add(match.group(1).rsplit("::", 1)[-1])
        elif match := SKIPPED_TESTS.match(line):
            reported_skipped = int(match.group(1))
    if reported_impacted != len(selected):
        raise RuntimeError(
            "Bit Code's impacted-test header disagrees with its listed tests: "
            f"header={reported_impacted}, listed={len(selected)}"
        )
    return selected, reported_impacted, reported_skipped


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
    project: Path,
    bitcode: Path,
    *,
    verbose: bool,
) -> dict[str, object]:
    static = run((str(bitcode), "test-impact", str(project)), cwd=REPO_ROOT)
    if verbose:
        print(f"\n--- Bit Code {fixture.name} output ---\n{static.stdout.rstrip()}")
    selected, reported_impacted, reported_skipped = parse_bitcode_selection(
        static.stdout
    )

    executed: set[str] = set()
    probe = project / ".bitcode-oracle-probe"
    for test in fixture.tests:
        probe.unlink(missing_ok=True)
        environment = os.environ.copy()
        environment["BITCODE_ORACLE_PROBE"] = str(probe)
        run(fixture.dynamic_commands[test], cwd=project, env=environment)
        if probe.exists() and "selected_operation" in probe.read_text(
            encoding="utf-8"
        ).splitlines():
            executed.add(test)
    probe.unlink(missing_ok=True)

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
    args = parser.parse_args()
    bitcode = args.bitcode.resolve()
    if not bitcode.is_file():
        parser.error(f"Bit Code binary does not exist: {bitcode}")

    with tempfile.TemporaryDirectory(prefix="bitcode-core-trust-") as temporary:
        work_root = Path(temporary)
        results = {
            fixture.name: measure_fixture(
                fixture,
                initialize_fixture(fixture, work_root),
                bitcode,
                verbose=args.verbose,
            )
            for fixture in FIXTURES
        }
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
