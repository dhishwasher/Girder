#!/usr/bin/env python3

import unittest

from tools.typescript_support_benchmark import evaluate_cases, summarize_cases


class TypeScriptSupportBenchmarkTests(unittest.TestCase):
    def test_missing_negative_endpoint_is_true_negative(self) -> None:
        repository = {
            "semantic_cases": [
                {
                    "id": "negative",
                    "kind": "Contains",
                    "source": "crate::module",
                    "target": "crate::module::invented",
                    "expected_present": False,
                }
            ]
        }
        cases = evaluate_cases(
            repository,
            [{"path": "crate::module"}],
            [],
        )
        self.assertEqual(cases[0]["classification"], "tn")
        self.assertFalse(cases[0]["target_endpoint_present"])

    def test_missing_positive_endpoint_is_false_negative(self) -> None:
        repository = {
            "semantic_cases": [
                {
                    "id": "positive",
                    "kind": "Contains",
                    "source": "crate::module",
                    "target": "crate::module::required",
                    "expected_present": True,
                }
            ]
        }
        cases = evaluate_cases(repository, [{"path": "crate::module"}], [])
        self.assertEqual(cases[0]["classification"], "fn")
        self.assertEqual(summarize_cases(cases)["recall"], 0)


if __name__ == "__main__":
    unittest.main()
