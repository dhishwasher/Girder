#!/usr/bin/env python3
"""Unit tests for the description-search-accuracy harness's pure logic.

The measurement itself needs a built binary and a real graph; these cover
parsing `bitcode search` output and the accuracy/gating arithmetic.
"""

from __future__ import annotations

import unittest

try:
    from tools.description_search_accuracy import parse_hits, rank_of, summarize
except ModuleNotFoundError:  # Direct execution via `python tools/<script>.py`.
    from description_search_accuracy import parse_hits, rank_of, summarize


class ParseHitsTests(unittest.TestCase):
    def test_parses_score_and_path_per_line_in_order(self) -> None:
        stdout = (
            'Searching . for "discover response" ...\n'
            "  0.42  crate::app::mcp::discover_result\n"
            "  0.10  crate::app::render_function\n"
        )
        self.assertEqual(
            parse_hits(stdout),
            ["crate::app::mcp::discover_result", "crate::app::render_function"],
        )

    def test_no_matches_line_yields_no_hits(self) -> None:
        stdout = 'Searching . for "gibberish" ...\n  no matches\n'
        self.assertEqual(parse_hits(stdout), [])

    def test_negative_score_still_parses(self) -> None:
        stdout = "  -0.05  crate::app::weird\n"
        self.assertEqual(parse_hits(stdout), ["crate::app::weird"])

    def test_a_banner_line_is_not_mistaken_for_a_hit(self) -> None:
        # The banner line has no leading two-space indent before a decimal
        # score, so it must never be parsed as a result.
        stdout = 'Searching . for "0.5 things" ...\n  0.30  crate::app::thing\n'
        self.assertEqual(parse_hits(stdout), ["crate::app::thing"])


class RankOfTests(unittest.TestCase):
    def test_first_hit_is_rank_one(self) -> None:
        self.assertEqual(rank_of("a", ["a", "b", "c"]), 1)

    def test_third_hit_is_rank_three(self) -> None:
        self.assertEqual(rank_of("c", ["a", "b", "c"]), 3)

    def test_absent_path_is_none(self) -> None:
        self.assertIsNone(rank_of("z", ["a", "b", "c"]))

    def test_empty_hits_is_none(self) -> None:
        self.assertIsNone(rank_of("a", []))


def record(description: str, rank: int | None) -> dict:
    return {
        "description": description,
        "expected_path": "crate::x",
        "hits": [],
        "rank_of_expected": rank,
        "top1_correct": rank == 1,
        "top5_correct": rank is not None and rank <= 5,
    }


class SummarizeTests(unittest.TestCase):
    def test_baseline_mode_reports_without_gating(self) -> None:
        summary = summarize([record("a", 1), record("b", None)], None)
        self.assertAlmostEqual(summary["top1_accuracy"], 0.5)
        self.assertAlmostEqual(summary["top5_accuracy"], 0.5)
        self.assertFalse(summary["gated"])
        self.assertEqual(summary["policy_result"], "BASELINE")

    def test_gated_mode_passes_when_both_thresholds_are_met(self) -> None:
        records = [record("a", 1), record("b", 1), record("c", 3), record("d", None)]
        threshold = {"min_top1_accuracy": 0.5, "min_top5_accuracy": 0.75}
        summary = summarize(records, threshold)
        self.assertAlmostEqual(summary["top1_accuracy"], 0.5)
        self.assertAlmostEqual(summary["top5_accuracy"], 0.75)
        self.assertEqual(summary["policy_result"], "PASS")

    def test_gated_mode_fails_when_top1_misses_the_threshold(self) -> None:
        records = [record("a", 2), record("b", 3)]
        threshold = {"min_top1_accuracy": 0.5, "min_top5_accuracy": 0.5}
        summary = summarize(records, threshold)
        self.assertEqual(summary["top1_accuracy"], 0.0)
        self.assertEqual(summary["policy_result"], "FAIL")

    def test_gated_mode_fails_when_top5_misses_even_if_top1_passes(self) -> None:
        # A pathological but real shape: every hit is rank 1 or absent, so
        # top-1 and top-5 accuracy are equal -- this pins that top-5 is
        # actually checked independently rather than implied by top-1.
        records = [record("a", 1), record("b", None), record("c", None)]
        threshold = {"min_top1_accuracy": 0.3, "min_top5_accuracy": 0.9}
        summary = summarize(records, threshold)
        self.assertAlmostEqual(summary["top1_accuracy"], 1 / 3)
        self.assertAlmostEqual(summary["top5_accuracy"], 1 / 3)
        self.assertEqual(summary["policy_result"], "FAIL")

    def test_misses_are_reported_by_description(self) -> None:
        records = [record("found", 1), record("missing", None)]
        summary = summarize(records, None)
        self.assertEqual(summary["top1_misses"], ["missing"])
        self.assertEqual(summary["top5_misses"], ["missing"])

    def test_empty_corpus_is_refused_rather_than_dividing_by_zero(self) -> None:
        with self.assertRaises(ValueError):
            summarize([], None)


if __name__ == "__main__":
    unittest.main()
