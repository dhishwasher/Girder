"""Tests for the final cross-language benchmark aggregate."""

from __future__ import annotations

import unittest

from tools.competitor_benchmark.matrix_reporting import (
    FIXTURES, RUNNABLE_PRODUCTS, aggregate_campaigns, matched_all_pass_cost, validate_matrix,
)


def row(product: str, fixture: str) -> dict:
    pass_definition = fixture != "modest-go" or product != "girder"
    return {
        "product": product, "fixture": fixture, "version": "1", "commit": "c",
        "campaign_state": "COMPLETE", "cold_setup_seconds": 1.0, "warm_query_seconds_total": 0.5,
        "base_task_status": {"PASS": 1 if pass_definition else 0, "WRONG": 4 if pass_definition else 5},
        "base_status_by_kind": {"definition": "PASS" if pass_definition else "WRONG"},
        "base_score_by_kind": {
            kind: {"true_positive": 1, "false_positive": 0, "false_negative": 1,
                   "precision": 1.0, "recall": 0.5}
            for kind in ("callers", "callees", "impact", "tests")
        },
        "freshness_terminal_query_status": {"PASS": 6, "WRONG": 24},
        "mutation_terminal_status": {"WRONG": 6},
        "update_to_all_correct_count": 0, "update_to_all_correct_seconds_mean": None,
        "query_response_bytes": 100, "query_tool_calls": 140, "query_record_count": 100,
        "calls_per_query_record": 1.4, "peak_rss_bytes": 1024,
        "policy_id": "p", "harness_commit": "h",
        "query_cost_by_kind": {
            kind: {"response_bytes": 10 if product == "girder" else 20, "tool_calls": 2,
                   "query_records": 1, "status": {"PASS": 1}}
            for kind in ("definition", "callers", "callees", "impact", "tests")
        },
    }


class MatrixReportingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.rows = [row(product, fixture) for product in RUNNABLE_PRODUCTS for fixture in FIXTURES]

    def test_aggregate_counts_distinct_tasks_and_terminal_queries(self) -> None:
        result = {item["product"]: item for item in aggregate_campaigns(self.rows)}
        self.assertEqual(result["girder"]["base_task_status"], {"PASS": 3, "WRONG": 17})
        self.assertEqual(result["girder"]["freshness_terminal_query_status"], {"PASS": 24, "WRONG": 96})
        self.assertEqual(result["girder"]["mutation_terminal_status"], {"WRONG": 24})
        self.assertEqual(result["girder"]["base_score_by_kind"]["impact"]["recall"], 0.5)
        self.assertIsNone(result["girder"]["update_to_all_correct_seconds_mean"])

    def test_matrix_rejects_missing_and_duplicate_rows(self) -> None:
        with self.assertRaisesRegex(ValueError, "matrix mismatch"):
            validate_matrix(self.rows[:-1])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            validate_matrix(self.rows + [self.rows[0]])

    def test_matched_cost_excludes_wrong_pair(self) -> None:
        girder_go = next(item for item in self.rows if item["product"] == "girder" and item["fixture"] == "modest-go")
        girder_go["query_cost_by_kind"]["definition"]["status"] = {"WRONG": 1}
        result = matched_all_pass_cost(self.rows, "girder", "ripwire", "definition")
        self.assertEqual(result["comparable_pairs"], 3)
        self.assertEqual(result["left_response_bytes"], 30)
        self.assertEqual(result["right_response_bytes"], 60)
        self.assertEqual(result["left_tool_calls"], result["right_tool_calls"])


if __name__ == "__main__":
    unittest.main()
