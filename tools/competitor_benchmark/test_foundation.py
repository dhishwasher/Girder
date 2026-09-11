"""Narrow tests for the frozen competitor-benchmark foundation."""

from __future__ import annotations

import json
import os
import sys
import hashlib
import tempfile
import unittest
from pathlib import Path

from tools.competitor_benchmark.fixtures import (
    apply_mutation,
    load_corpus,
    materialize,
    safe_relative_path,
)
from tools.competitor_benchmark.protocol import ExecutionRecord, Status, normalize_paths
from tools.competitor_benchmark.process import run_supervised
from tools.competitor_benchmark.resources import (
    MemorySnapshot,
    read_meminfo,
    resource_block_reason,
)
from tools.competitor_benchmark.scoring import (
    assess,
    assess_definition,
    expand_test_file_predictions,
    score_set,
)


ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs" / "competitor-benchmark"


class ScoringTests(unittest.TestCase):
    def test_precision_and_recall_penalize_returning_everything(self) -> None:
        score = score_set(["a", "b", "noise"], ["a", "b", "missing"])
        self.assertEqual((score.true_positive, score.false_positive, score.false_negative), (2, 1, 1))
        self.assertAlmostEqual(score.precision, 2 / 3)
        self.assertAlmostEqual(score.recall, 2 / 3)

    def test_empty_set_rules_are_explicit(self) -> None:
        self.assertEqual(score_set([], []).precision, 1.0)
        self.assertEqual(score_set([], []).recall, 1.0)
        self.assertEqual(score_set([], ["required"]).recall, 0.0)
        self.assertEqual(score_set(["noise"], []).precision, 0.0)

    def test_whole_test_file_expands_and_preserves_unrelated_false_positive(self) -> None:
        inventory = ["test_a.py::test_needed", "test_a.py::test_unrelated", "test_b.py::test_other"]
        self.assertEqual(
            expand_test_file_predictions(["test_a.py::*"], inventory),
            ("test_a.py::test_needed", "test_a.py::test_unrelated"),
        )
        score = score_set(expand_test_file_predictions(["test_a.py::*"], inventory),
                          ["test_a.py::test_needed"])
        self.assertEqual((score.precision, score.recall), (0.5, 1.0))

    def test_matching_wrong_answers_remain_wrong(self) -> None:
        left = assess(["wrong.py::same"], ["right.py::target"])
        right = assess(["wrong.py::same"], ["right.py::target"])
        self.assertEqual(left[0], Status.WRONG)
        self.assertEqual(right[0], Status.WRONG)

    def test_generator_inputs_are_materialized_once(self) -> None:
        status, score = assess(iter(["wrong"]), iter(["required"]))
        self.assertEqual(status, Status.WRONG)
        self.assertEqual(score.false_positive, 1)
        self.assertEqual(score.false_negative, 1)

    def test_initial_empty_answer_is_wrong_not_stale(self) -> None:
        self.assertEqual(assess([], ["required"])[0], Status.WRONG)

    def test_distinct_previous_oracle_is_stale(self) -> None:
        status, _ = assess(["old.py::name"], ["new.py::name"], prior_expected=["old.py::name"])
        self.assertEqual(status, Status.STALE)

    def test_native_failure_status_is_not_overwritten(self) -> None:
        status, score = assess(["a"], ["a"], native_status=Status.TIMEOUT)
        self.assertEqual(status, Status.TIMEOUT)
        self.assertEqual(score.recall, 1.0)

    def test_definition_requires_current_source_evidence(self) -> None:
        self.assertEqual(
            assess_definition(["core.py::f"], ["core.py::f"], source_text="old body",
                              expected_source_marker="new body", prior_source_marker="old body")[0],
            Status.STALE,
        )
        self.assertEqual(
            assess_definition(["core.py::f"], ["core.py::f"], source_text=None,
                              expected_source_marker="new body")[0],
            Status.UNSUPPORTED,
        )

    def test_rename_is_stale_when_old_identity_has_unchanged_body(self) -> None:
        status, _ = assess_definition(
            ["core.py::old"], ["core.py::new"], source_text="same body",
            expected_source_marker="same body", prior_expected=["core.py::old"],
            prior_source_marker="same body",
        )
        self.assertEqual(status, Status.STALE)


class FixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.corpus = load_corpus(DOCS / "corpus.json")
        cls.oracle = json.loads((DOCS / "oracle.json").read_text(encoding="utf-8"))

    def test_path_restrictions_reject_escape_and_absolute_paths(self) -> None:
        for value in ("../escape", "/absolute", "a/../../escape", "."):
            with self.subTest(value=value), self.assertRaises(ValueError):
                safe_relative_path(value)

    def test_every_mutation_materializes_and_every_oracle_identity_exists(self) -> None:
        states = {(row["fixture_id"], row["state"]): row for row in self.oracle["states"]}
        with tempfile.TemporaryDirectory() as raw:
            workspace = Path(raw)
            for fixture in self.corpus["fixtures"]:
                destination = workspace / fixture["id"]
                materialize(fixture, destination)
                self._assert_state(destination, states[(fixture["id"], 0)])
                for index, mutation in enumerate(fixture["mutations"], 1):
                    apply_mutation(destination, mutation)
                    self._assert_state(destination, states[(fixture["id"], index)])

    def _assert_state(self, root: Path, state: dict[str, object]) -> None:
        identities = {state["dynamic_dispatch_edge"].split(" -> ")[0], state["dynamic_dispatch_edge"].split(" -> ")[1]}
        identities.update(state["test_inventory"])
        for answers in state["expected"].values():
            identities.update(answers)
        for identity in identities:
            path_text, symbol = identity.split("::", 1)
            path = root / path_text
            self.assertTrue(path.is_file(), identity)
            self.assertIn(symbol, path.read_text(encoding="utf-8"), identity)
        definition = state["expected"]["definition"][0]
        definition_path = root / definition.split("::", 1)[0]
        self.assertIn(state["definition_source_marker"], definition_path.read_text(encoding="utf-8"))
        for path in root.rglob("*.py"):
            compile(path.read_text(encoding="utf-8"), str(path), "exec")

    def test_mutations_reject_symlink_ancestor_escape(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            parent = Path(raw)
            root = parent / "fixture"
            outside = parent / "outside"
            root.mkdir()
            outside.mkdir()
            (root / "link").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(ValueError):
                apply_mutation(root, {"write": {"link/stolen.py": "bad"}})
            victim = outside / "victim.py"
            victim.write_text("safe")
            with self.assertRaises(ValueError):
                apply_mutation(root, {"delete": ["link/victim.py"]})
            self.assertTrue(victim.exists())

    def test_corpus_has_required_languages_and_mutation_order(self) -> None:
        fixtures = {row["id"]: row for row in self.corpus["fixtures"]}
        self.assertEqual(
            {row["language"] for row in fixtures.values()},
            {"python", "rust", "typescript_tsx", "go"},
        )
        expected = ["body_edit", "rename", "caller_added", "caller_removed", "symbol_moved", "file_rename"]
        for fixture in fixtures.values():
            self.assertEqual([row["id"] for row in fixture["mutations"]], expected)

    def test_smoke_fixture_is_excluded_from_comparative_aggregate(self) -> None:
        fixtures = {row["id"]: row for row in self.corpus["fixtures"]}
        self.assertFalse(fixtures["tiny-python"]["aggregate_inclusion"])
        self.assertTrue(all(row["aggregate_inclusion"] for key, row in fixtures.items() if key != "tiny-python"))


class PolicyAndProtocolTests(unittest.TestCase):
    def test_policy_pins_every_product_and_serial_execution(self) -> None:
        policy = json.loads((DOCS / "policy.json").read_text(encoding="utf-8"))
        self.assertEqual(policy["host_limits"]["execution_concurrency"], 1)
        self.assertEqual(
            {row["id"] for row in policy["competitors"]},
            {"girder", "gitnexus", "codebase-memory-mcp", "code-review-graph", "ripwire"},
        )
        for product in policy["competitors"]:
            self.assertTrue(product["version"])
            self.assertRegex(product["commit"], r"^[0-9a-f]{40}$")

    def test_dependency_locks_match_frozen_hashes(self) -> None:
        policy = json.loads((DOCS / "policy.json").read_text(encoding="utf-8"))
        for relative, expected in policy["acquisition"]["dependency_lock_sha256"].items():
            actual = hashlib.sha256((ROOT / relative).read_bytes()).hexdigest()
            self.assertEqual(actual, expected, relative)
        package = json.loads((DOCS / "locks" / "gitnexus-package-lock.json").read_text())
        self.assertEqual(package["packages"][""]["dependencies"], {"gitnexus": "1.6.11"})

    def test_freeze_manifest_matches_inputs(self) -> None:
        manifest = json.loads((DOCS / "freeze-manifest.json").read_text(encoding="utf-8"))
        for relative, expected in manifest["sha256"].items():
            self.assertEqual(hashlib.sha256((ROOT / relative).read_bytes()).hexdigest(), expected, relative)

    def test_adapter_implementations_cannot_import_evaluator_oracle(self) -> None:
        for path in (Path(__file__).parent / "adapters").glob("*.py"):
            text = path.read_text(encoding="utf-8")
            self.assertNotIn("oracle.json", text, path.name)
            self.assertNotIn("load_oracle", text, path.name)

    def test_execution_record_keeps_byte_counts_and_distinct_status(self) -> None:
        record = ExecutionRecord(
            schema_version=1, competitor="x", version="1", commit="0" * 40,
            task_id="t", fixture_id="f", command=("x",),
            started_at="2026-09-11T00:00:00Z", ended_at="2026-09-11T00:00:01Z",
            elapsed_seconds=1.0, exit_status=0, timed_out=False,
            resource_blocked=False, stdout_bytes=len("µ".encode()), stderr_bytes=0,
            stdout_artifact="raw/out", stderr_artifact="raw/err",
            normalized_answer=("a",), expected_answer=("b",), status=Status.WRONG,
        ).to_dict()
        self.assertEqual(record["stdout_bytes"], 2)
        self.assertEqual(record["status"], "WRONG")

    def test_normalization_is_narrow_and_deterministic(self) -> None:
        self.assertEqual(normalize_paths(["./b.py::f", "a.py::g", "a.py::g", "x\\y.py::z"]),
                         ("a.py::g", "b.py::f", "x/y.py::z"))

    def test_resource_threshold_is_fail_closed(self) -> None:
        snapshot = MemorySnapshot(3_000, 799, 0, 0)
        self.assertIsNotNone(resource_block_reason(snapshot, 800))
        self.assertIsNone(resource_block_reason(snapshot, 799))

    def test_meminfo_parser_requires_all_fields(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            path = Path(raw) / "meminfo"
            path.write_text("MemTotal: 10 kB\nMemAvailable: 8 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n")
            self.assertEqual(read_meminfo(path).available_bytes, 8192)
            path.write_text("MemTotal: 10 kB\n")
            with self.assertRaises(ValueError):
                read_meminfo(path)

    def test_supervisor_accounts_utf8_bytes_and_writes_raw_streams(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            result = run_supervised(
                [sys.executable, "-c", "import sys; print('µ', end=''); print('err', file=sys.stderr, end='')"],
                cwd=root, env=os.environ, stdout_path=root / "stdout", stderr_path=root / "stderr",
                timeout_seconds=5, max_output_bytes=1024,
                minimum_available_bytes=1, emergency_available_bytes=1,
                maximum_tree_rss_bytes=256 * 1024 * 1024,
            )
            self.assertEqual(result.status, Status.PASS)
            self.assertEqual(result.stdout_bytes, 2)
            self.assertEqual(result.stderr_bytes, 3)
            self.assertEqual((root / "stdout").read_bytes(), "µ".encode())

    def test_supervisor_enforces_timeout_and_rss_cap(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            common = dict(cwd=root, env=os.environ, minimum_available_bytes=1,
                          emergency_available_bytes=1, max_output_bytes=1024)
            timeout = run_supervised(
                [sys.executable, "-c", "import time; time.sleep(2)"],
                stdout_path=root / "timeout.out", stderr_path=root / "timeout.err",
                timeout_seconds=0.1, maximum_tree_rss_bytes=256 * 1024 * 1024, **common,
            )
            self.assertEqual(timeout.status, Status.TIMEOUT)
            blocked = run_supervised(
                [sys.executable, "-c", "import time; x=bytearray(1024*1024); time.sleep(2)"],
                stdout_path=root / "rss.out", stderr_path=root / "rss.err",
                timeout_seconds=2, maximum_tree_rss_bytes=1, **common,
            )
            self.assertEqual(blocked.status, Status.RESOURCE_BLOCKED)


if __name__ == "__main__":
    unittest.main()
