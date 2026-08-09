import unittest
import os
import sys
import tempfile
import time
from pathlib import Path

from tools.core_trustworthiness_oracle import (
    FIXTURE_ROOT,
    aggregate_metrics,
    metrics,
    parse_bitcode_selection,
    parse_framework_inventory,
    run,
)


class OracleUnitTests(unittest.TestCase):
    def test_fixture_sources_are_inert_in_the_bit_code_repository(self):
        live_sources = [
            path
            for path in FIXTURE_ROOT.rglob("*")
            if path.is_file() and path.suffix in {".py", ".rs"}
        ]

        self.assertEqual(live_sources, [])

    def test_parses_bitcode_test_impact_output(self):
        selected, impacted, skipped = parse_bitcode_selection(
            """
Impacted tests (2):
  ✓ crate::tests::impact::direct (rust)
  ✓ crate::tests::impact::cli_selected (rust)

  (2 other test(s) not in impact set — skipped)
"""
        )

        self.assertEqual(
            selected,
            {
                "crate::tests::impact::direct",
                "crate::tests::impact::cli_selected",
            },
        )
        self.assertEqual(impacted, 2)
        self.assertEqual(skipped, 2)

    def test_parser_preserves_duplicate_leaf_names_as_distinct_full_ids(self):
        selected, impacted, skipped = parse_bitcode_selection(
            """
Impacted tests (2):
  ✓ crate::tests::first::test_common (rust)
  ✓ crate::tests::second::test_common (rust)

  (1 other test(s) not in impact set — skipped)
"""
        )

        self.assertEqual(
            selected,
            {
                "crate::tests::first::test_common",
                "crate::tests::second::test_common",
            },
        )
        self.assertEqual(impacted, 2)
        self.assertEqual(skipped, 1)

    def test_framework_inventory_parser_keeps_exact_test_ids(self):
        self.assertEqual(
            parse_framework_inventory(
                "rust",
                "first::test_common: test\nsecond::test_common: test\n",
            ),
            {"first::test_common", "second::test_common"},
        )
        self.assertEqual(
            parse_framework_inventory(
                "python",
                "tests.first.Case.test_common\ntests.second.Case.test_common\n",
            ),
            {"tests.first.Case.test_common", "tests.second.Case.test_common"},
        )

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_runner_terminates_a_timed_out_process_group(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "survived"
            child = (
                "import pathlib,time; "
                "time.sleep(0.5); "
                f"pathlib.Path({str(marker)!r}).write_text('alive')"
            )
            parent = (
                "import subprocess,time; "
                f"subprocess.Popen([{sys.executable!r}, '-c', {child!r}]); "
                "time.sleep(30)"
            )
            started = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                run(
                    (sys.executable, "-c", parent),
                    cwd=Path(directory),
                    timeout_seconds=0.1,
                )
            self.assertLess(time.monotonic() - started, 5)
            time.sleep(0.7)
            self.assertFalse(marker.exists(), "timed-out descendant survived")

    @unittest.skipUnless(os.name == "posix", "requires POSIX process groups")
    def test_runner_rejects_excessive_combined_output(self):
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "survived"
            child = (
                "import pathlib,time; "
                "time.sleep(0.5); "
                f"pathlib.Path({str(marker)!r}).write_text('alive')"
            )
            parent = (
                "import subprocess,time; "
                f"subprocess.Popen([{sys.executable!r}, '-c', {child!r}]); "
                "print('x' * 1024, flush=True); time.sleep(30)"
            )
            with self.assertRaisesRegex(RuntimeError, "output exceeded 64 bytes"):
                run(
                    (sys.executable, "-c", parent),
                    cwd=Path(directory),
                    max_output_bytes=64,
                )
            time.sleep(0.7)
            self.assertFalse(marker.exists(), "noisy descendant survived")

    def test_computes_precision_and_recall_from_exact_sets(self):
        result = metrics(
            {"direct", "cli_selected", "cli_unrelated", "missed"},
            {"direct", "cli_selected", "cli_unrelated"},
            {"direct", "cli_selected", "missed"},
            reported_impacted=3,
            reported_skipped=1,
        )

        self.assertEqual(result["true_positives"], ["cli_selected", "direct"])
        self.assertEqual(result["false_positives"], ["cli_unrelated"])
        self.assertEqual(result["false_negatives"], ["missed"])
        self.assertEqual(result["true_negatives"], [])
        self.assertEqual(result["precision"], 0.666667)
        self.assertEqual(result["recall"], 0.666667)

    def test_metrics_require_an_exact_reported_test_inventory(self):
        with self.assertRaisesRegex(RuntimeError, "test universe"):
            metrics(
                {"selected", "skipped"},
                {"selected"},
                {"selected"},
                reported_impacted=1,
                reported_skipped=0,
            )

    def test_aggregates_fixture_counts_without_averaging_rates(self):
        result = aggregate_metrics(
            {
                "rust": {
                    "true_positives": ["a", "b"],
                    "false_positives": ["c"],
                    "false_negatives": [],
                    "true_negatives": ["d"],
                },
                "python": {
                    "true_positives": ["e"],
                    "false_positives": [],
                    "false_negatives": ["f"],
                    "true_negatives": ["g"],
                },
            }
        )

        self.assertEqual(
            result,
            {
                "true_positives": 3,
                "false_positives": 1,
                "false_negatives": 1,
                "true_negatives": 2,
                "precision": 0.75,
                "recall": 0.75,
            },
        )


if __name__ == "__main__":
    unittest.main()
