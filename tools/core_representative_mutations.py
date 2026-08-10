#!/usr/bin/env python3
"""Dynamic-vs-static test-impact comparison on a real cached repository.

The trustworthiness oracle proves precision/recall on synthetic fixtures.
This extends the same per-test dynamic-proof technique — a probe write that
only fires when the mutated code actually executes — to one cached
representative repository (Click), for a small hand-declared set of
mutations and their real, unmodified test callers.

Per docs/core-gap-analysis.md, the checked result is evidence that an
extended, reproducible dynamic comparison against a real codebase exists and
runs, not a claim of general accuracy across Click's behavior: only the
declared mutations and declared tests are measured. A defect this finds is a
new recorded gap, not a failure of this harness.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Mapping, Sequence

try:
    from tools.core_representative_benchmark import (
        DEFAULT_CACHE,
        acquire_artifact,
        extract_archive,
    )
    from tools.harness_support import BoundedProcessResult, run_bounded
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from core_representative_benchmark import (
        DEFAULT_CACHE,
        acquire_artifact,
        extract_archive,
    )
    from harness_support import BoundedProcessResult, run_bounded


REPO_ROOT = Path(__file__).resolve().parents[1]
CORPUS_PATH = REPO_ROOT / "docs" / "core-representative-corpus.json"
DEFAULT_OUTPUT = REPO_ROOT / "docs" / "core-representative-mutations.json"
REPOSITORY_ID = "click-8.4.1"
DEFAULT_COMMAND_TIMEOUT_SECONDS = 120.0
DEFAULT_MAX_OUTPUT_BYTES = 1024 * 1024

PROBE_HELPER_ANCHOR = 'F = t.TypeVar("F", bound="t.Callable[..., t.Any]")\n'
PROBE_HELPER = '''def _bitcode_mutation_probe(marker: str) -> None:
    import os as _bitcode_os

    path = _bitcode_os.environ.get("BITCODE_ORACLE_PROBE")
    if path:
        with open(path, "a", encoding="utf-8") as probe:
            probe.write(f"{marker}\\n")


'''


@dataclasses.dataclass(frozen=True)
class DeclaredTest:
    node_id: str
    expected_positive: bool


@dataclasses.dataclass(frozen=True)
class Mutation:
    id: str
    file: str
    anchor: str
    replacement: str
    graph_path: str
    tests: tuple[DeclaredTest, ...]


MUTATIONS = (
    Mutation(
        id="group-invoke",
        file="src/click/core.py",
        anchor=(
            "    def invoke(self, ctx: Context) -> t.Any:\n"
            "        def _process_result(value: t.Any) -> t.Any:"
        ),
        replacement=(
            "    def invoke(self, ctx: Context) -> t.Any:\n"
            '        _bitcode_mutation_probe("group-invoke")\n'
            "        def _process_result(value: t.Any) -> t.Any:"
        ),
        graph_path="crate::click::core::Group::invoke",
        tests=(
            # Both use click.group(); Group.invoke overrides Command.invoke,
            # so a real dispatch through a Group must execute the probe.
            DeclaredTest(
                "tests/test_commands.py::test_other_command_forward", True
            ),
            DeclaredTest("tests/test_chain.py::test_basic_chaining", True),
            # Plain @click.command(): dispatch resolves to the base
            # Command.invoke, never Group.invoke.
            DeclaredTest(
                "tests/test_commands.py::test_other_command_invoke", False
            ),
        ),
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


def load_repository_manifest() -> Mapping[str, object]:
    corpus = json.loads(CORPUS_PATH.read_text(encoding="utf-8"))
    for repository in corpus["repositories"]:
        if repository["id"] == REPOSITORY_ID:
            return repository
    raise RuntimeError(f"{REPOSITORY_ID} is not declared in {CORPUS_PATH}")


def checkout_repository(
    manifest: Mapping[str, object],
    destination: Path,
    *,
    cache_dir: Path,
    offline: bool,
    timeout_seconds: float,
) -> Path:
    archive = acquire_artifact(
        manifest["artifact"],
        cache_dir,
        offline=offline,
        timeout_seconds=timeout_seconds,
    )
    return extract_archive(archive, destination, manifest["artifact"]["root"])


def apply_mutation(root: Path, mutation: Mutation) -> None:
    target = root / mutation.file
    text = target.read_text(encoding="utf-8")
    if PROBE_HELPER_ANCHOR not in text:
        raise RuntimeError(
            f"probe helper anchor not found in {mutation.file}; "
            "Click's source may have moved"
        )
    if mutation.anchor not in text:
        raise RuntimeError(
            f"mutation anchor not found in {mutation.file} for {mutation.id}; "
            "Click's source may have moved"
        )
    text = text.replace(PROBE_HELPER_ANCHOR, PROBE_HELPER + PROBE_HELPER_ANCHOR, 1)
    text = text.replace(mutation.anchor, mutation.replacement, 1)
    target.write_text(text, encoding="utf-8")


def initialize_checkout(
    manifest: Mapping[str, object],
    mutation: Mutation,
    destination: Path,
    *,
    cache_dir: Path,
    offline: bool,
    timeout_seconds: float,
    max_output_bytes: int,
) -> Path:
    command_options = {
        "timeout_seconds": timeout_seconds,
        "max_output_bytes": max_output_bytes,
    }
    project = checkout_repository(
        manifest,
        destination,
        cache_dir=cache_dir,
        offline=offline,
        timeout_seconds=timeout_seconds,
    )
    run(("git", "init", "--quiet"), cwd=project, **command_options)
    run(
        ("git", "config", "user.email", "oracle@bitcode.invalid"),
        cwd=project,
        **command_options,
    )
    run(
        ("git", "config", "user.name", "Bit Code Representative Oracle"),
        cwd=project,
        **command_options,
    )
    run(("git", "add", "."), cwd=project, **command_options)
    run(
        ("git", "commit", "--quiet", "-m", "baseline"),
        cwd=project,
        **command_options,
    )
    apply_mutation(project, mutation)
    return project


def parse_bitcode_selected_paths(output: str) -> set[str]:
    selected = set()
    for line in output.splitlines():
        line = line.strip()
        if line.startswith("✓"):
            # "  ✓ <graph_path> (python)"
            rest = line[1:].strip()
            if "(" in rest:
                rest = rest.rsplit("(", 1)[0].strip()
            selected.add(rest)
    return selected


def measure_mutation(
    mutation: Mutation,
    bitcode: Path,
    work_root: Path,
    manifest: Mapping[str, object],
    *,
    cache_dir: Path,
    offline: bool,
    timeout_seconds: float,
    max_output_bytes: int,
) -> Mapping[str, object]:
    command_options = {
        "timeout_seconds": timeout_seconds,
        "max_output_bytes": max_output_bytes,
    }
    static_project = initialize_checkout(
        manifest,
        mutation,
        work_root / "static",
        cache_dir=cache_dir,
        offline=offline,
        timeout_seconds=timeout_seconds,
        max_output_bytes=max_output_bytes,
    )
    static = run(
        (str(bitcode), "test-impact", str(static_project)),
        cwd=REPO_ROOT,
        **command_options,
    )
    selected_paths = parse_bitcode_selected_paths(static.stdout)
    node_id_to_selected: dict[str, bool] = {}

    executed: dict[str, bool] = {}
    for index, test in enumerate(mutation.tests):
        project = initialize_checkout(
            manifest,
            mutation,
            work_root / f"dynamic-{index}",
            cache_dir=cache_dir,
            offline=offline,
            timeout_seconds=timeout_seconds,
            max_output_bytes=max_output_bytes,
        )
        probe = project / ".bitcode-representative-probe"
        environment = os.environ.copy()
        environment["BITCODE_ORACLE_PROBE"] = str(probe)
        environment["PYTHONPATH"] = str(project / "src")
        completed = run(
            (sys.executable, "-m", "pytest", test.node_id, "-q"),
            cwd=project,
            env=environment,
            **command_options,
        )
        if "1 passed" not in completed.stdout:
            raise RuntimeError(
                f"declared test {test.node_id!r} did not pass in isolation:\n"
                f"{completed.stdout}\n{completed.stderr}"
            )
        executed[test.node_id] = bool(
            probe.exists() and mutation.id in probe.read_text(encoding="utf-8")
        )
        # Bit Code's static universe is keyed by graph path, not pytest node
        # id; a test is "selected" here iff the mutated function's graph
        # path appears anywhere in the printed impacted-test section next to
        # a graph path this declared test owns. Since the static run and each
        # dynamic run use fresh, identically-mutated checkouts, a simpler and
        # exact signal is whether the *test's own* function is present in the
        # printed selection for the shared static run.
        node_id_to_selected[test.node_id] = _selected_for_node(
            selected_paths, test.node_id
        )

    true_positives = []
    false_positives = []
    false_negatives = []
    true_negatives = []
    for test in mutation.tests:
        node_id = test.node_id
        was_selected = node_id_to_selected[node_id]
        was_executed = executed[node_id]
        if was_executed and test.expected_positive is not True:
            raise RuntimeError(
                f"{node_id} dynamically executed the mutation but was declared "
                "a negative case; fix the declared expectation"
            )
        if not was_executed and test.expected_positive is not False:
            raise RuntimeError(
                f"{node_id} did not dynamically execute the mutation but was "
                "declared a positive case; fix the declared expectation"
            )
        if was_executed and was_selected:
            true_positives.append(node_id)
        elif was_executed and not was_selected:
            false_negatives.append(node_id)
        elif not was_executed and was_selected:
            false_positives.append(node_id)
        else:
            true_negatives.append(node_id)

    return {
        "id": mutation.id,
        "graph_path": mutation.graph_path,
        "true_positives": sorted(true_positives),
        "false_positives": sorted(false_positives),
        "false_negatives": sorted(false_negatives),
        "true_negatives": sorted(true_negatives),
        "precision": _ratio(
            len(true_positives), len(true_positives) + len(false_positives)
        ),
        "recall": _ratio(
            len(true_positives), len(true_positives) + len(false_negatives)
        ),
    }


def _selected_for_node(selected_paths: set[str], node_id: str) -> bool:
    # tests/test_commands.py::test_other_command_forward
    #   -> crate::tests::test_commands::test_other_command_forward
    file_part, function = node_id.split("::", 1)
    module = file_part[: -len(".py")].replace("/", "::")
    graph_path = f"crate::{module}::{function}"
    return graph_path in selected_paths


def _ratio(numerator: int, denominator: int) -> float:
    return 1.0 if denominator == 0 else round(numerator / denominator, 6)


def aggregate_metrics(results: Sequence[Mapping[str, object]]) -> Mapping[str, object]:
    tp = sum(len(r["true_positives"]) for r in results)
    fp = sum(len(r["false_positives"]) for r in results)
    fn = sum(len(r["false_negatives"]) for r in results)
    tn = sum(len(r["true_negatives"]) for r in results)
    return {
        "true_positives": tp,
        "false_positives": fp,
        "false_negatives": fn,
        "true_negatives": tn,
        "precision": _ratio(tp, tp + fp),
        "recall": _ratio(tp, tp + fn),
    }


def atomic_write_json(path: Path, value: Mapping[str, object]) -> None:
    directory = path.parent
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        "w", dir=directory, delete=False, suffix=".tmp", encoding="utf-8"
    ) as handle:
        json.dump(value, handle, indent=2, sort_keys=True)
        handle.write("\n")
        temp_path = Path(handle.name)
    os.replace(temp_path, path)


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bitcode", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path, default=DEFAULT_CACHE)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--output", type=Path, default=None)
    parser.add_argument("--json", action="store_true")
    parser.add_argument(
        "--command-timeout-seconds",
        type=float,
        default=DEFAULT_COMMAND_TIMEOUT_SECONDS,
    )
    parser.add_argument(
        "--max-command-output-bytes", type=int, default=DEFAULT_MAX_OUTPUT_BYTES
    )
    args = parser.parse_args(argv)
    if not args.bitcode.is_file():
        parser.error(f"Bit Code binary does not exist: {args.bitcode}")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    manifest = load_repository_manifest()
    results = []
    with tempfile.TemporaryDirectory(prefix="bitcode-repr-mutations-") as temporary:
        work_root = Path(temporary)
        for mutation in MUTATIONS:
            results.append(
                measure_mutation(
                    mutation,
                    args.bitcode.resolve(),
                    work_root / mutation.id,
                    manifest,
                    cache_dir=args.cache_dir,
                    offline=args.offline,
                    timeout_seconds=args.command_timeout_seconds,
                    max_output_bytes=args.max_command_output_bytes,
                )
            )
    document = {
        "schema_version": 1,
        "repository": REPOSITORY_ID,
        "mutations": results,
        "aggregate": aggregate_metrics(results),
    }
    if args.output:
        atomic_write_json(args.output, document)
        print(f"wrote representative mutations result: {args.output}")
    elif args.json:
        print(json.dumps(document, indent=2, sort_keys=True))
    else:
        for result in results:
            print(
                f"{result['id']}: precision={result['precision']} "
                f"recall={result['recall']} "
                f"fp={result['false_positives']} fn={result['false_negatives']}"
            )
        print(f"aggregate: {document['aggregate']}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from None
