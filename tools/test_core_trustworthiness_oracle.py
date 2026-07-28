import unittest

from tools.core_trustworthiness_oracle import (
    FIXTURE_ROOT,
    aggregate_metrics,
    metrics,
    parse_bitcode_selection,
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

        self.assertEqual(selected, {"direct", "cli_selected"})
        self.assertEqual(impacted, 2)
        self.assertEqual(skipped, 2)

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
