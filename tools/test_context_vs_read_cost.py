#!/usr/bin/env python3
"""Unit tests for the context-vs-read cost harness's pure arithmetic.

The measurement itself needs a built binary and a real graph; these cover
the parts that decide PASS/FAIL, including the negative-reduction case that
the real run actually hit.
"""

from __future__ import annotations

import unittest

try:
    from tools.context_vs_read_cost import reduction_ratio, summarize
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from context_vs_read_cost import reduction_ratio, summarize


def record(node: str, read: int, context: int, source_only: int) -> dict:
    return {
        "node": node,
        "read_bytes": read,
        "context_bytes": context,
        "source_only_bytes": source_only,
    }


class ReductionRatioTests(unittest.TestCase):
    def test_saving_half_the_bytes_is_one_half(self) -> None:
        self.assertAlmostEqual(reduction_ratio(1000, 500), 0.5)

    def test_identical_sizes_save_nothing(self) -> None:
        self.assertAlmostEqual(reduction_ratio(1000, 1000), 0.0)

    def test_growing_the_output_is_a_negative_ratio(self) -> None:
        # The real run hit this: a 39-byte function in a 392-byte file cost
        # 6,120 bytes through `bitcode context`, ~15.6x the whole file.
        self.assertAlmostEqual(reduction_ratio(392, 6120), -14.612244897959183)

    def test_a_nonpositive_baseline_is_refused_rather_than_dividing_by_zero(self) -> None:
        for before in (0, -1):
            with self.subTest(before=before):
                with self.assertRaises(ValueError):
                    reduction_ratio(before, 100)


class SummarizeTests(unittest.TestCase):
    def test_aggregates_are_summed_across_nodes_not_averaged_per_node(self) -> None:
        summary = summarize(
            [record("a", 1000, 500, 100), record("b", 3000, 500, 100)],
            0.4,
        )
        self.assertEqual(summary["read_bytes_total"], 4000)
        self.assertEqual(summary["context_bytes_total"], 1000)
        self.assertEqual(summary["source_only_bytes_total"], 200)
        self.assertAlmostEqual(summary["source_only_reduction_ratio"], 0.95)
        self.assertEqual(summary["nodes_measured"], 2)

    def test_gate_is_the_source_only_arm_so_a_bad_context_arm_cannot_fail_it(self) -> None:
        # Deliberately the shape the real run produced: `context` is a net
        # loss on a node, `--source-only` still wins. Only the latter gates.
        summary = summarize([record("tiny", 392, 6120, 188)], 0.4)
        self.assertEqual(summary["policy_result"], "PASS")
        self.assertEqual(
            summary["nodes_where_context_costs_more_than_reading"], ["tiny"]
        )
        self.assertEqual(summary["nodes_where_source_only_costs_more_than_reading"], [])

    def test_missing_the_threshold_reports_fail(self) -> None:
        summary = summarize([record("a", 1000, 900, 700)], 0.4)
        self.assertEqual(summary["policy_result"], "FAIL")

    def test_exactly_meeting_the_threshold_passes(self) -> None:
        summary = summarize([record("a", 1000, 600, 600)], 0.4)
        self.assertAlmostEqual(summary["source_only_reduction_ratio"], 0.4)
        self.assertEqual(summary["policy_result"], "PASS")

    def test_a_source_only_regression_is_reported_not_hidden(self) -> None:
        summary = summarize([record("a", 100, 50, 200)], 0.4)
        self.assertEqual(
            summary["nodes_where_source_only_costs_more_than_reading"], ["a"]
        )
        self.assertEqual(summary["policy_result"], "FAIL")


if __name__ == "__main__":
    unittest.main()
