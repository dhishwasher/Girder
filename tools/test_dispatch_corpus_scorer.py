import unittest

from tools.dispatch_corpus_scorer import cell_label, confusion_matrix, observed_class


class CellLabelUnitTests(unittest.TestCase):
    def test_exact_match(self):
        for cls in ("must", "may", "unknown", "excluded"):
            self.assertEqual(cell_label(cls, cls), "exact")

    def test_observed_excluded_is_always_unsafe_exclusion_when_wrong(self):
        # excluded is the STRONGEST claim (a provable true negative); being
        # wrong about it silently drops a reachable test from the union.
        for expected in ("must", "may", "unknown"):
            self.assertEqual(cell_label(expected, "excluded"), "unsafe_exclusion")

    def test_observed_must_when_not_expected_is_overclaim(self):
        for expected in ("may", "unknown", "excluded"):
            self.assertEqual(cell_label(expected, "must"), "overclaim")

    def test_observed_may_against_unknown_or_excluded_is_overclaim(self):
        self.assertEqual(cell_label("unknown", "may"), "overclaim")
        self.assertEqual(cell_label("excluded", "may"), "overclaim")

    def test_observed_less_certain_than_expected_is_conservative(self):
        # The common Stage 1 case: expected must/may, Girder floods to
        # unknown. Safe (still in the union), just imprecise.
        self.assertEqual(cell_label("must", "unknown"), "conservative")
        self.assertEqual(cell_label("may", "unknown"), "conservative")
        self.assertEqual(cell_label("must", "may"), "conservative")

    def test_observed_unknown_against_excluded_is_conservative_not_unsound(self):
        # This was the original bug: an inverted ordinal ranked "unknown"
        # as MORE certain than "excluded", flagging safe over-inclusion of
        # a true negative as unsound. It is the opposite: including an
        # unreachable test never drops a reachable one.
        self.assertEqual(cell_label("excluded", "unknown"), "conservative")


class ObservedClassUnitTests(unittest.TestCase):
    def test_finds_test_in_must_bucket(self):
        selection = {"must": {"a"}, "may": set(), "unknown": set()}
        self.assertEqual(observed_class(selection, "a"), "must")

    def test_absent_from_every_bucket_is_excluded(self):
        selection = {"must": set(), "may": set(), "unknown": {"b"}}
        self.assertEqual(observed_class(selection, "a"), "excluded")


class ConfusionMatrixUnitTests(unittest.TestCase):
    def _result(self, case_id, language, tests):
        return {"id": case_id, "language": language, "status": "scored", "tests": tests}

    def _test(self, test_id, expected, observed):
        return {
            "test_id": test_id,
            "status": "scored",
            "expected": expected,
            "observed": observed,
            "cell": cell_label(expected, observed),
        }

    def test_must_precision_requires_expected_must_for_a_true_positive(self):
        # This was the second original bug: a false-positive Must claim on
        # an expected=may case was being counted as a Must true positive
        # merely because expected != "excluded".
        results = [
            self._result(
                "c1",
                "rust",
                [
                    self._test("t1", "must", "must"),  # true positive
                    self._test("t2", "may", "must"),  # false positive
                ],
            )
        ]
        matrix = confusion_matrix(results)
        self.assertEqual(matrix["must_true_positives"], 1)
        self.assertEqual(matrix["must_false_positives"], 1)
        self.assertEqual(matrix["must_precision_on_corpus"], 0.5)

    def test_must_precision_is_none_on_zero_denominator(self):
        results = [self._result("c1", "rust", [self._test("t1", "may", "unknown")])]
        matrix = confusion_matrix(results)
        self.assertIsNone(matrix["must_precision_on_corpus"])

    def test_must_or_may_recall_pools_across_cases(self):
        results = [
            self._result("c1", "rust", [self._test("t1", "must", "unknown")]),
            self._result("c2", "rust", [self._test("t2", "may", "may")]),
        ]
        matrix = confusion_matrix(results)
        self.assertEqual(matrix["must_or_may_positive_denominator"], 2)
        self.assertEqual(matrix["must_or_may_true_positives"], 1)
        self.assertEqual(matrix["must_or_may_recall_on_corpus"], 0.5)

    def test_failed_case_counts_once_and_scored_cases_are_unaffected(self):
        results = [
            {"id": "c1", "status": "failed", "reason": "could not resolve origin"},
            self._result("c2", "rust", [self._test("t1", "must", "must")]),
        ]
        matrix = confusion_matrix(results)
        self.assertEqual(matrix["pooled"]["failed"], 1)
        self.assertEqual(matrix["pooled"]["exact"], 1)
        self.assertEqual(matrix["total_test_cells"], 2)


if __name__ == "__main__":
    unittest.main()
